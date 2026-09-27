"""Native real Firefox sharing, through visible UI and the existing collector."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from lock_roundtrip import complete_ppm, monitor_command
from portal_roundtrip import relative_mouse


def activate_share(wait_text, key):
    wait_text('Realm browser capture ready', 'browser-page-ready')
    key('ret')
    wait_text('Use operating system settings', 'browser-permission')
    key('alt-a')


def wait_for_capture(wait, root):
    wait('test -s ' + root + '/result.json || test -s ' + root + '/error.json', seconds=35)


def navigate_capture(key):
    key('ctrl-l')
    # Match the Nix driver CHAR_TO_KEY scan codes for punctuation.
    for character in 'http://127.0.0.1:8765/':
        key({':': 'shift-0x27', '.': '0x34', '/': '0x35'}.get(character, character))
    key('ret')


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path) / 'browser-screencast'
    evidence.mkdir()
    root = '/tmp/realm-native-browser'
    result = {'passed': False}

    def guest(command, script=None, timeout=40):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=timeout)

    def wait(command, seconds=15):
        guest('for attempt in $(seq 1 ' + str(seconds * 10) + '); do if ' + command
              + '; then exit 0; fi; sleep 0.1; done; exit 1', timeout=seconds + 10)

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus'])

    def key(value):
        monitor_command(monitor, 'sendkey ' + value)
        time.sleep(0.15)

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        monitor_command(monitor, 'screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            assert time.monotonic() < deadline, path
            time.sleep(0.05)
        return path

    def wait_text(text, name):
        deadline = time.monotonic() + 45
        while True:
            path = screenshot(name)
            # OCR is a guest-only fixture dependency; PPM travels through the
            # existing SSH channel, with no browser automation or fake input.
            output = subprocess.check_output(ssh + ['timeout 10 tesseract stdin stdout -l eng'],
                input=path.read_bytes(), timeout=15).decode()
            (evidence / (name + '.ocr.txt')).write_text(output)
            if text.casefold() in ' '.join(output.split()).casefold():
                return
            assert time.monotonic() < deadline, (text, output)
            time.sleep(0.5)

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
        guest(user + ' install -d -m 0700 ' + root)
        result['provisioning'] = guest('cat /var/tmp/realm-native-browser-packages.txt')
        result['version'] = guest('/usr/bin/firefox --version').strip()
        guest(user + ' systemd-run --user --no-block --collect --unit=realm-native-browser-collector '
              + shlex.join(['python3', '/var/tmp/realm-native-vm/browser_screencast.py',
                            '--page', '/var/tmp/realm-native-vm/browser_screencast.html', '--output', root]))
        wait('test -s ' + root + '/ready')
        selected = guest(user + ' systemd-run --user --wait --pipe --quiet --collect /usr/bin/xdg-settings get default-web-browser').strip()
        assert selected == 'realm-browser-test.desktop', selected
        state(0, 'browser-before')
        key('meta_l-b')
        wait('pgrep -u alice -x firefox')
        result['browser_processes'] = guest('for pid in $(pgrep -u alice -x firefox); do readlink /proc/$pid/exe; cat /proc/$pid/stat; done')
        state(1, 'browser-open')
        navigate_capture(key)
        activate_share(wait_text, key)
        wait('pgrep -u alice -x slurp')
        path = screenshot('browser-output-chooser')
        with path.open('rb') as stream:
            stream.readline()
            width, height = map(int, stream.readline().split())
        mice = monitor_command(monitor, 'info mice')
        result['mice'] = mice
        monitor_command(monitor, 'mouse_set ' + relative_mouse(mice))
        monitor_command(monitor, 'mouse_move -32767 -32767')
        monitor_command(monitor, f'mouse_move {width // 2} {height // 2}')
        deadline = time.monotonic() + 20
        while guest('if pgrep -u alice -x slurp >/dev/null; then echo yes; else echo no; fi').strip() == 'yes':
            assert time.monotonic() < deadline, 'browser Slurp selection timed out'
            monitor_command(monitor, 'mouse_button 1')
            monitor_command(monitor, 'mouse_button 0')
            time.sleep(0.5)
        wait_for_capture(wait, root)
        guest('test ! -e ' + root + '/error.json')
        capture = json.loads(guest('cat ' + root + '/result.json'))
        # The unchanged collector validates PNG payloads, dimensions and
        # advancing media time before it atomically publishes this result.
        assert capture['stopped'] and capture['trackStates'] == ['ended'], capture
        assert len(capture['frames']) == 2, capture
        result['capture'] = capture
        wait_text('Realm capture passed and stopped', 'browser-capture-stopped')
        key('meta_l-q')
        state(0, 'browser-closed')
        wait('! pgrep -u alice -x firefox')
        result['passed'] = True
    finally:
        retention_errors = []
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'browser-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        for name in ('frame-0.png', 'frame-1.png', 'result.json', 'error.json', 'ready'):
            try:
                data = subprocess.check_output(ssh + ['cat ' + root + '/' + name], timeout=15)
                (evidence / name).write_bytes(data)
            except Exception as error:
                print(f'browser artifact {name} unavailable: {error}', file=sys.stderr)
                if result['passed'] and name != 'error.json':
                    retention_errors.append(str(error))
        for name, action in (
            ('journal', lambda: guest('sudo journalctl -b -n 200 --no-pager _UID=' + uid)),
            ('stop', lambda: guest(user + ' timeout 10 systemctl --user stop realm-native-browser-collector.service')),
            ('screenshot', lambda: screenshot('browser-final')),
        ):
            try:
                output = action()
                if isinstance(output, str):
                    (evidence / (name + '.txt')).write_text(output)
            except Exception as error:
                print(f'browser diagnostic {name} failed: {error}', file=sys.stderr)
                if result['passed'] and name == 'stop':
                    retention_errors.append(str(error))
        if retention_errors:
            result.update(passed=False, retention_errors=retention_errors)
            (evidence / 'browser-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
            raise AssertionError(retention_errors)


if __name__ == '__main__':
    main()
