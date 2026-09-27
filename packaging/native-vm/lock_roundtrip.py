"""CI-only installed native swaylock authentication, through SSH and QEMU HMP."""
import json
from pathlib import Path
import shlex
import socket
import subprocess
import sys
import time


def roundtrips(start, snapshot, key, password, suppressed, gone, screenshot, sleep, evidence, restored):
    results = []
    try:
        for cycle in range(2):
            start()
            locked = snapshot()
            assert locked["state"] == "active" and locked["processes"], locked
            start()
            assert snapshot() == locked, "duplicate start changed locker identities"
            screenshot(f"lock-{cycle}")
            key("meta_l-d")
            suppressed()
            key("ctrl-u")
            password("wrongpassword")
            sleep(5)
            assert snapshot() == locked, "wrong password did not preserve the lock"
            screenshot(f"lock-wrong-password-{cycle}")
            key("ctrl-u")
            password("realmtest")
            for _ in range(30):
                if snapshot()["state"] == "inactive":
                    break
                sleep(1)
            else:
                raise AssertionError("correct password did not unlock within 30 seconds")
            gone(locked["processes"])
            screenshot(f"lock-unlocked-{cycle}")
            key("meta_l-d")
            restored(True)
            key("esc")
            restored(False)
            results.append({"cycle": cycle, "processes": locked["processes"],
                            "duplicate_start_same_processes": True,
                            "launcher_binding_suppressed": True,
                            "launcher_binding_restored": True,
                            "wrong_password_rejected": True, "password_unlock": True})
    finally:
        (evidence / "lock-roundtrip.json").write_text(json.dumps(results, indent=2) + "\n")
    return results


def read_prompt(connection, deadline):
    output = b''
    while not output.endswith(b'(qemu) '):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError('QEMU monitor prompt deadline')
        connection.settimeout(remaining)
        chunk = connection.recv(4096)
        if not chunk:
            raise RuntimeError('QEMU monitor closed before command completion')
        output += chunk
        if len(output) > 1024 * 1024:
            raise RuntimeError('QEMU monitor reply exceeded bound')
    return output


def monitor_command(monitor, command):
    deadline = time.monotonic() + 5
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(monitor)
        read_prompt(connection, deadline)
        connection.sendall((command + '\n').encode())
        output = read_prompt(connection, deadline).decode(errors='replace')
    assert not any(error in output.lower() for error in ('unknown command', 'invalid parameter', 'error:')), output
    return output


def complete_ppm(path):
    try:
        with path.open('rb') as stream:
            if stream.readline() != b'P6\n':
                return False
            width, height = map(int, stream.readline().split())
            if stream.readline() != b'255\n' or min(width, height) <= 0:
                return False
            return path.stat().st_size - stream.tell() == width * height * 3
    except (OSError, ValueError):
        return False


# Execute in the guest so /proc identities cannot accidentally describe the host.
SNAPSHOT = r'''
import json, pathlib, subprocess
uid = subprocess.check_output(['id', '-u', 'alice'], text=True).strip()
command = ['runuser', '-u', 'alice', '--', 'env', 'XDG_RUNTIME_DIR=/run/user/' + uid,
           'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/' + uid + '/bus',
           'timeout', '10', 'systemctl', '--user', 'show', 'realm-lock.service',
           '--property=ActiveState,MainPID,ControlGroup']
props = dict(line.split('=', 1) for line in subprocess.check_output(command, text=True).splitlines())
identities = {}
group = props['ControlGroup']
if group:
    assert group.startswith('/') and group != '/', group
    path = pathlib.Path('/sys/fs/cgroup' + group + '/cgroup.procs')
    for pid in path.read_text().split() if path.exists() else []:
        try:
            fields = pathlib.Path('/proc/' + pid + '/stat').read_text().rsplit(')', 1)[1].split()
            exe = str(pathlib.Path('/proc/' + pid + '/exe').resolve(strict=True))
        except FileNotFoundError:
            continue
        assert fields[0] not in ('Z', 'X'), fields
        assert pathlib.Path(exe).name == 'swaylock', exe
        identities[pid] = {'start_time': int(fields[19]), 'executable': exe}
print(json.dumps({'state': props['ActiveState'], 'main_pid': int(props['MainPID']), 'processes': identities}))
'''


def main():
    idle_mode = sys.argv[1:2] == ['--idle']
    monitor, evidence_path, *ssh = sys.argv[2:] if idle_mode else sys.argv[1:]
    evidence = Path(evidence_path)

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=40)

    uid = guest("id -u alice").strip()
    assert uid.isdigit(), uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      f'XDG_RUNTIME_DIR=/run/user/{uid}',
                      f'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/{uid}/bus'])

    def hmp(command):
        monitor_command(monitor, command)

    def key(value):
        hmp('sendkey ' + value)
        time.sleep(0.15)  # QEMU's default key hold is 100 ms.

    def password(value):
        for character in value:
            key(character)
        key('ret')

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        hmp('screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            if time.monotonic() >= deadline:
                raise AssertionError(f'incomplete QEMU screenshot: {path}')
            time.sleep(0.05)
        return path

    def gone(identities):
        guest('sudo python3 -', 'import pathlib\nidentities = ' + repr(identities) + '''
for pid, old in identities.items():
    path = pathlib.Path('/proc/' + pid + '/stat')
    try:
        fields = path.read_text().rsplit(')', 1)[1].split()
    except FileNotFoundError:
        continue
    assert int(fields[19]) != old['start_time'], (pid, old)
''')

    observations = []
    def restored(present):
        condition = 'pgrep -u alice -x fuzzel' if present else '! pgrep -u alice -x fuzzel'
        guest('for attempt in $(seq 1 100); do if ' + condition + '; then exit 0; fi; sleep 0.1; done; exit 1')

    def snapshot():
        current = json.loads(guest('sudo python3 -', SNAPSHOT))
        observations.append(current)
        print(json.dumps(current), flush=True)
        (evidence / ('idle-lock-observations.json' if idle_mode else 'lock-observations.json')).write_text(json.dumps(observations, indent=2) + '\n')
        return current

    try:
        assert guest(user + ' timeout 10 systemctl --user show realm-idle.service -p ActiveState --value').strip() == 'inactive'
        if idle_mode:
            from idle_roundtrip import run
            run(guest, user, key, password, snapshot,
                lambda: guest("for attempt in $(seq 1 20); do if pgrep -u alice -x fuzzel; then exit 1; fi; sleep 0.1; done"),
                gone, screenshot, restored, evidence)
            return
        roundtrips(
            lambda: guest(user + ' timeout 30 systemctl --user start realm-lock.service'),
            snapshot, key, password,
            lambda: guest("for attempt in $(seq 1 20); do if pgrep -u alice -x fuzzel; then exit 1; fi; sleep 0.1; done"),
            gone, screenshot, time.sleep, evidence, restored)
        assert guest(user + ' timeout 10 systemctl --user show realm-idle.service -p ActiveState --value').strip() == 'inactive'
    finally:
        # Diagnostics must not replace the original assertion/transport failure.
        for name, collect in (
            ('screenshot', lambda: screenshot('idle-final' if idle_mode else 'lock-final')),
            ('journal', lambda: guest('sudo journalctl -b -n 200 --no-pager _SYSTEMD_USER_UNIT=realm-lock.service + _COMM=swaylock + _COMM=unix_chkpwd')),
        ):
            try:
                output = collect()
                if isinstance(output, str):
                    (evidence / (('idle-lock-' if idle_mode else 'lock-') + name + '.txt')).write_text(output)
                    print(output, flush=True)
            except Exception as error:
                print(f'lock diagnostic {name} failed: {error}', file=sys.stderr, flush=True)


if __name__ == '__main__':
    main()
