"""CI-only normal graphical logout/relogin and next-login theme evidence."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from consumer_roundtrip import PROCESS, assert_consumer
from lock_roundtrip import complete_ppm, monitor_command


def validate_transition(before, after, generation):
    assert after['session'] != before['session'], after
    assert not after['old_session_present'] and not after['survivors'], after
    assert after['login']['generation'] == generation, after
    assert (after['login']['owner_pid'], after['login']['start_time']) != (
        before['login']['owner_pid'], before['login']['start_time']), after
    for name in ('river', 'realm-wm', 'realm-bar'):
        assert after['processes'][name] != before['processes'][name], after


SNAPSHOT = r'''
import json, pathlib, subprocess, sys
runtime, previous = sys.argv[1:]
previous = json.loads(previous)
def identity(pid):
    try:
        fields = pathlib.Path('/proc', str(pid), 'stat').read_text().rsplit(')', 1)[1].split()
        return [int(pid), int(fields[19])] if fields[0] not in ('Z', 'X') else None
    except FileNotFoundError:
        return None
sessions = subprocess.check_output(['loginctl', 'list-sessions', '--no-legend'], text=True).splitlines()
ids = [line.split()[0] for line in sessions if line.split()]
graphical = []
for session in ids:
    output = subprocess.check_output(['loginctl', 'show-session', session, '-p', 'Name', '-p', 'Type', '-p', 'Remote', '-p', 'State'], text=True)
    props = dict(line.split('=', 1) for line in output.splitlines())
    if props.get('Name') == 'alice' and props.get('Type') == 'wayland' and props.get('Remote') == 'no' and props.get('State') == 'active':
        graphical.append(session)
assert len(graphical) == 1, graphical
login = json.loads(pathlib.Path(runtime, 'realm/session-theme.json').read_text())
assert identity(login['owner_pid']) == [login['owner_pid'], login['start_time']], login
assert login['boot_id'] == pathlib.Path('/proc/sys/kernel/random/boot_id').read_text().strip(), login
processes = {}
for name in ('river', 'realm-wm', 'realm-bar'):
    pids = subprocess.check_output(['pgrep', '-u', 'alice', '-x', name], text=True).split()
    assert len(pids) == 1, (name, pids)
    processes[name] = identity(pids[0])
    assert processes[name] is not None, (name, pids)
old = list(previous.get('processes', {}).values())
old.extend(previous.get('applications', {}).values())
if previous:
    old.append([previous['login']['owner_pid'], previous['login']['start_time']])
print(json.dumps({'session': graphical[0], 'login': login, 'processes': processes,
    'survivors': [item for item in old if identity(item[0]) == item],
    'old_session_present': previous.get('session') in ids}))
'''

QUIT = r'''
import json, socket, sys
with socket.socket(socket.AF_UNIX) as connection:
    connection.settimeout(10)
    connection.connect(sys.argv[1])
    with connection.makefile('rwb', buffering=0) as stream:
        replies = []
        for request in ({'cmd': 'hello', 'arg': {'version': 2, 'client': 'realm-native-relogin'}}, {'cmd': 'quit'}):
            stream.write(json.dumps(request).encode() + b'\n')
            frame = stream.readline(1024 * 1024)
            assert frame.endswith(b'\n'), frame
            replies.append(json.loads(frame))
        assert replies[-1] == {'reply': 'ok'}, replies
        print(json.dumps(replies))
'''


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path)
    result = {'passed': False}

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=30)

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus'])

    def snapshot(previous):
        return json.loads(guest('sudo python3 - ' + shlex.quote(runtime) + ' '
                                + shlex.quote(json.dumps(previous)), SNAPSHOT))

    def wait(check, seconds=90):
        deadline = time.monotonic() + seconds
        while True:
            try:
                return check()
            except (AssertionError, KeyError, ValueError, subprocess.SubprocessError) as error:
                result['last_wait_error'] = str(error)
                assert time.monotonic() < deadline, result
                time.sleep(0.25)

    def state(count):
        guest(user + ' python3 /var/tmp/realm-native-vm/control_get_state.py '
              + runtime + '/realm/ctl.sock /tmp/realm-native-relogin-state')
        raw = guest('cat /tmp/realm-native-relogin-state')
        current = json.loads(raw.splitlines()[-1])
        assert current['reply'] == 'state', current
        assert sum(orbit['windows'] for orbit in current['data']['orbits']) == count, current
        return current

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        monitor_command(monitor, 'screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            assert time.monotonic() < deadline, path
            time.sleep(0.05)

    try:
        before = snapshot({})
        result['before'] = before
        generation = guest('cat /home/alice/.config/realm/generated/current').strip()
        assert generation != before['login']['generation'], before
        result['expected_generation'] = generation
        result['before_state'] = state(0)
        monitor_command(monitor, 'sendkey meta_l-ret')
        result['before_quit_state'] = wait(lambda: state(1), 20)
        before['applications'] = {}
        for name in ('foot', 'zsh'):
            application = wait(lambda: json.loads(guest('sudo python3 - ' + name, PROCESS)), 20)
            before['applications'][name] = [application['pid'], application['start_time']]
        screenshot('relogin-open-terminal-before-quit')
        result['quit_response'] = json.loads(guest(user + ' python3 - ' + runtime + '/realm/ctl.sock', QUIT))

        def transitioned():
            after = snapshot(before)
            validate_transition(before, after, generation)
            return after
        result['after'] = wait(transitioned)
        result['after_state'] = wait(lambda: state(0))
        guest(user + ' timeout 10 systemctl --user is-active --quiet realm-session.target realm-wm.service realm-bar.service')
        monitor_command(monitor, 'sendkey meta_l-ret')
        root = '/home/alice/.config/realm/generated/generations/' + generation

        def consumer(name, executable):
            value = json.loads(guest('sudo python3 - ' + name, PROCESS))
            assert_consumer(value, root, executable)
            return value
        terminal = wait(lambda: consumer('foot', '/usr/bin/foot'), 20)
        shell = wait(lambda: consumer('zsh', '/usr/bin/zsh'), 20)
        assert shell['parent_pid'] == terminal['pid'], (terminal, shell)
        result.update(terminal=terminal, shell=shell)
        result['terminal_state'] = wait(lambda: state(1), 20)
        screenshot('relogin-terminal-b')
        monitor_command(monitor, 'sendkey meta_l-q')
        result['closed_state'] = wait(lambda: state(0), 20)
        wait(lambda: guest('! pgrep -u alice -x foot && ! pgrep -u alice -x zsh'), 20)
        result['passed'] = True
    finally:
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'relogin-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        for name, action in (
            ('journal', lambda: guest('sudo journalctl -b -n 200 --no-pager _UID=' + uid)),
            ('screenshot', lambda: screenshot('relogin-final')),
        ):
            try:
                output = action()
                if output is not None:
                    (evidence / ('relogin-' + name + '.txt')).write_text(output)
            except Exception as error:
                print(f'relogin diagnostic {name} failed: {error}', file=sys.stderr)


if __name__ == '__main__':
    main()
