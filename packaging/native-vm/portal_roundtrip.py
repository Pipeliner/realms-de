"""CI-only native chooser/capture orchestration; reuse the persistent client."""
import json
import hashlib
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time

from lock_roundtrip import complete_ppm, monitor_command
from pointer import position_pointer, click_pointer


def validate_result(result):
    chooser = result['filechooser']
    assert chooser['elapsed_ms'] <= 2000, chooser
    assert chooser['completion'] == 'response' and chooser['response_code'] == 1, chooser
    selected = result['file_selection']
    assert selected['elapsed_ms'] <= 2000 and selected['completion'] == 'response', selected
    assert selected['response_code'] == 0 and selected['uri'] == 'file:///tmp/realmfile', selected
    assert selected['bytes'] == 29, selected
    assert selected['sha256'] == hashlib.sha256(b'Realm portal selection proof\n').hexdigest(), selected
    assert result['settings']['reply_type'] == '(a{sa{sv}})', result
    frame = result['screencast']
    assert all(frame[key] > 0 for key in ('node_id', 'buffer_bytes', 'width', 'height')), frame
    assert re.fullmatch(r'[0-9a-f]{64}', frame['sha256']), frame


def select_file(key):
    key('ctrl-l')
    for character in '/tmp/realmfile':
        key({'/': 'slash'}.get(character, character))
    key('alt-o')


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path)
    result = {'passed': False}
    root = '/tmp/realm-native-portal'

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=40)

    def wait(command):
        guest('for attempt in $(seq 1 150); do if ' + command + '; then exit 0; fi; sleep 0.1; done; exit 1')

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus'])

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        monitor_command(monitor, 'screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            assert time.monotonic() < deadline, path
            time.sleep(0.05)
        return path

    def state(count, name):
        deadline = time.monotonic() + 15
        while True:
            guest(user + ' python3 /var/tmp/realm-native-vm/control_get_state.py '
                  + runtime + '/realm/ctl.sock ' + root + '/state.ndjson')
            raw = guest('cat ' + root + '/state.ndjson')
            current = json.loads(raw.splitlines()[-1])
            if current.get('reply') == 'state' and sum(orbit['windows'] for orbit in current['data']['orbits']) == count:
                (evidence / (name + '-state.ndjson')).write_text(raw)
                return
            assert time.monotonic() < deadline, current
            time.sleep(0.1)

    try:
        guest(user + ' timeout 10 systemctl --user is-active --quiet pipewire.socket')
        guest(user + ' install -d -m 0700 ' + root)
        state(0, 'portal-before')
        # User-manager launch inherits the real imported session environment.
        # The shell retains helper exit status, and systemd owns its lifetime.
        script = ('python3 /var/tmp/realm-native-vm/portal_vm_helper.py > ' + root + '/result.json 2> ' + root + '/stderr; '
                  + 'status=$?; printf "%s\\n" "$status" > ' + root + '/status; exit "$status"')
        guest(user + ' systemd-run --user --no-block --collect --unit=realm-native-portal '
              + '--setenv=REALM_PORTAL_FILECHOOSER_READY=' + root + '/ready.json '
              + shlex.join(['timeout', '150', 'sh', '-c', script]))
        wait('test -s ' + root + '/ready.json')
        ready = json.loads(guest('cat ' + root + '/ready.json'))
        assert ready['elapsed_ms'] <= 2000 and ready['handle'].endswith('/realm_file'), ready
        result['ready'] = ready
        state(1, 'portal-filechooser')
        screenshot('portal-filechooser')
        monitor_command(monitor, 'sendkey alt-c')
        time.sleep(0.15)
        state(0, 'portal-filechooser-closed')
        guest(user + ' touch ' + root + '/ready.json.continue')
        wait('test -s ' + root + '/ready.json.selection')
        selection_ready = json.loads(guest('cat ' + root + '/ready.json.selection'))
        assert selection_ready['elapsed_ms'] <= 2000 and selection_ready['handle'].endswith('/realm_select'), selection_ready
        result['selection_ready'] = selection_ready
        state(1, 'portal-file-selection')
        screenshot('portal-file-selection')
        def selection_key(value):
            monitor_command(monitor, 'sendkey ' + value)
            time.sleep(0.15)
        select_file(selection_key)
        state(0, 'portal-file-selection-closed')
        wait('pgrep -u alice -x slurp')
        screenshot('portal-slurp')
        position_pointer(monitor + ".qmp", result)
        screenshot('portal-slurp-pointer')
        deadline = time.monotonic() + 20
        while guest('if pgrep -u alice -x slurp >/dev/null; then echo yes; else echo no; fi').strip() == 'yes':
            assert time.monotonic() < deadline, 'Slurp selection timed out'
            click_pointer(monitor + ".qmp", result)
            time.sleep(0.5)
        wait('test -s ' + root + '/status')
        assert guest('cat ' + root + '/status').strip() == '0', guest('cat ' + root + '/stderr')
        result['portal'] = json.loads(guest('cat ' + root + '/result.json'))
        validate_result(result['portal'])
        guest(user + ' timeout 10 systemctl --user is-active --quiet pipewire.service')
        result['passed'] = True
    finally:
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'portal-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        actions = [(name, lambda name=name: guest('tail -c 16384 ' + root + '/' + name))
                   for name in ('ready.json', 'result.json', 'stderr', 'status')]
        actions += [('journal', lambda: guest('sudo journalctl -b -n 200 --no-pager _UID=' + uid)),
                    ('stop', lambda: guest(user + ' timeout 10 systemctl --user stop realm-native-portal.service')),
                    ('screenshot', lambda: screenshot('portal-final'))]
        for name, action in actions:
            try:
                output = action()
                if isinstance(output, str):
                    print(name + ':\n' + output, flush=True)
                    (evidence / ('portal-' + name + '.txt')).write_text(output)
            except Exception as error:
                print(f'portal diagnostic {name} failed: {error}', file=sys.stderr)


if __name__ == '__main__':
    main()
