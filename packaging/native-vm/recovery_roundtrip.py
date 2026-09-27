"""CI-only forced WM death; runtime assertions and retained geometry images."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from lock_roundtrip import complete_ppm, monitor_command
from relogin_roundtrip import SNAPSHOT as IDENTITIES
from window_roundtrip import SNAPSHOT, active


def validate_durable(live, durable):
    assert durable['active_orbit'] + 1 == active(live)['orbit'], durable
    assert len(durable['ledger']['orbits']) == len(live['ledger']), durable
    ids = []
    for current, saved in zip(live['ledger'], durable['ledger']['orbits']):
        windows = current['windows']
        expected = [window['id'] for window in windows]
        ids.extend(expected)
        assert saved['id'] + 1 == current['orbit'], saved
        assert saved['windows'] == expected and saved['layout'] == current['layout'], saved
        assert saved['stowed'] == [window['id'] for window in windows if window['stowed']], saved
        focused = [index for index, window in enumerate(windows) if window['focused']]
        assert saved['focus'] == (focused[0] if focused else None), saved
    assert sorted(binding['win_id'] for binding in durable['bindings']) == sorted(ids), durable


def validate_recovery(before, after):
    assert before['session'] == after['session'] and before['login'] == after['login'], after
    assert before['processes']['realm-wm'] != after['processes']['realm-wm'], after
    for name in ('river', 'realm-bar'):
        assert before['processes'][name] == after['processes'][name], after
    assert before['bar']['ActiveState'] == 'active' and after['bar'] == before['bar'], after
    assert after['durable'] == before['durable'], after


def validate_state_recovery(before, after):
    assert after['ledger'] == before['ledger'], after
    for field in ('layout', 'focused_title'):
        assert after['state'][field] == before['state'][field], after


KILL = r'''
import json, os, pathlib, signal, sys
pid, start = json.loads(sys.argv[1])
fields = pathlib.Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()
assert int(fields[19]) == start and fields[0] not in ('Z', 'X'), fields
assert pathlib.Path('/proc', str(pid), 'exe').resolve().name == 'realm-wm'
os.kill(pid, signal.SIGKILL)
'''


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path)
    result = {'passed': False, 'geometry_acceptance': 'pending image inspection', 'observations': []}

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=10)

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus'])

    def key(chord):
        monitor_command(monitor, 'sendkey ' + chord)
        time.sleep(0.15)

    def state():
        return json.loads(guest(user + ' python3 - ' + runtime + '/realm/ctl.sock', SNAPSHOT))

    def snapshot():
        value = json.loads(guest('sudo python3 - ' + runtime + " '{}'", IDENTITIES))
        raw = guest(user + ' systemctl --user show realm-bar.service '
                    '-p ActiveState -p ActiveEnterTimestampMonotonic -p NRestarts')
        value['bar'] = dict(line.split('=', 1) for line in raw.splitlines())
        assert value['bar']['ActiveEnterTimestampMonotonic'].isdigit(), value['bar']
        assert int(value['bar']['ActiveEnterTimestampMonotonic']) > 0, value['bar']
        assert value['bar']['NRestarts'].isdigit(), value['bar']
        value['durable'] = json.loads(guest('cat ' + runtime + '/realm/ledger.json'))
        return value

    def wait(check, label, deadline=None):
        if deadline is None:
            deadline = time.monotonic() + 20
        while True:
            try:
                value = check()
                assert time.monotonic() <= deadline, 'observation exceeded deadline'
                result['observations'].append({'step': label, 'value': value})
                return value
            except (AssertionError, KeyError, ValueError, subprocess.SubprocessError) as error:
                result['last_wait_error'] = {'step': label, 'error': str(error)}
                if time.monotonic() >= deadline:
                    raise AssertionError(result['last_wait_error']) from error
                time.sleep(0.1)

    def match(predicate):
        value = state()
        assert predicate(value), value
        return value

    def count(value):
        return sum(len(orbit['windows']) for orbit in value['ledger'])

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        monitor_command(monitor, 'screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            assert time.monotonic() < deadline, path
            time.sleep(0.05)
        with path.open('rb') as stream:
            stream.readline()
            return {'path': path.name, 'framebuffer_pixels': list(map(int, stream.readline().split())),
                    'scale': 'not exposed by read-only Realm protocol; inspect retained session journal'}

    try:
        wait(lambda: match(lambda value: count(value) == 0), 'empty session')
        for number, letter in enumerate('ABC', 1):
            script = '/tmp/rec' + letter.lower()
            guest('sudo tee ' + script + ' >/dev/null',
                  "printf '\\033]0;Realm recovery " + letter + "\\007'\n"
                  "printf '\\nRealm recovery " + letter + "\\nCrash recovery fixture\\n'\n"
                  "exec sleep infinity\n")
            key('meta_l-ret')
            wait(lambda: match(lambda value: count(value) == number), 'open terminal ' + letter)
            for character in 'source ' + script:
                key({' ': 'spc', '/': 'slash'}.get(character, character))
            key('ret')
            wait(lambda: match(lambda value: value['state']['focused_title'] == 'Realm recovery ' + letter),
                 'title terminal ' + letter)
        original = state()
        order = [window['id'] for window in active(original)['windows']]
        key('meta_l-l')
        prepared = wait(lambda: match(lambda value: [w['id'] for w in active(value)['windows']] != order),
                        'non-default ledger order')

        def persisted():
            value = snapshot()
            validate_durable(prepared, value['durable'])
            return value
        before = wait(persisted, 'durable pre-crash ledger')
        result.update(before=before, before_state=prepared, before_frame=screenshot('recovery-before'))
        delay = guest(user + ' systemctl --user show realm-wm.service -p RestartUSec --value').strip()
        assert delay == '1s', delay
        result['restart_delay'] = delay
        started = time.monotonic()
        guest('sudo python3 - ' + shlex.quote(json.dumps(before['processes']['realm-wm'])), KILL)

        def recovered():
            after = snapshot()
            validate_recovery(before, after)
            value = state()
            validate_state_recovery(prepared, value)
            return {'identity': after, 'state': value}
        result['after'] = wait(recovered, 'supervised recovery', started + 20)
        result['recovery_elapsed_seconds'] = time.monotonic() - started
        result['after_frame'] = screenshot('recovery-after')
        focus = prepared['state']['focused_title']
        windows = active(prepared)['windows']
        index = next(index for index, window in enumerate(windows) if window['focused'])
        next_title = windows[(index + 1) % len(windows)]['title']
        key('meta_l-j')
        assert next_title != focus, windows
        wait(lambda: match(lambda value: value['state']['focused_title'] == next_title and count(value) == 3),
             'real focus binding after restart')
        key('meta_l-k')
        wait(lambda: match(lambda value: value['ledger'] == prepared['ledger']), 'focus restored')
        for remaining in (2, 1, 0):
            key('meta_l-q')
            wait(lambda: match(lambda value: count(value) == remaining), 'close recovery fixture')
        result['passed'] = True
    finally:
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'recovery-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        try:
            journal = guest('sudo journalctl -b -n 240 --no-pager _UID=' + uid)
            (evidence / 'recovery-journal.txt').write_text(journal)
        except Exception as error:
            print(f'recovery journal failed: {error}', file=sys.stderr)
        if not result['passed']:
            try:
                screenshot('recovery-failure')
            except Exception as error:
                print(f'recovery failure screenshot failed: {error}', file=sys.stderr)


if __name__ == '__main__':
    main()
