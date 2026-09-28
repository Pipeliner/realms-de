"""CI-only installed GTK3/GTK4/Qt6 selected-generation consumption proof."""

import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time

from browser_roundtrip import wait_visible
from lock_roundtrip import complete_ppm, monitor_command


TOOLKITS = {
    'gtk3': ('/usr/bin/gtk3-widget-factory', 'gtk3-widget-factory', 'togglebutton'),
    'gtk4': ('/usr/bin/gtk4-widget-factory', 'GTK Widget Factory', 'Page 1'),
    'qt6': ('/usr/bin/qt6ct', 'Qt6 Configuration Tool', 'Appearance'),
}
GTK_DIAGNOSTIC = re.compile(
    r'(css|theme).*(error|failed|invalid|not found|unable|warning)'
    r'|(error|failed|invalid|warning).*(css|theme)', re.IGNORECASE,
)
QT_DIAGNOSTIC = re.compile(
    r'(qt6ct|palette|colou?r.scheme|config).*(error|failed|invalid|not found|unable|warning)'
    r'|(error|failed|invalid|warning).*(qt6ct|palette|colou?r.scheme|config)',
    re.IGNORECASE,
)


def required_paths(kind, root):
    if kind == 'gtk3':
        return [root + '/share/themes/realm/gtk-3.0/gtk.css']
    if kind == 'gtk4':
        return [root + '/share/themes/realm/gtk-4.0/gtk.css']
    assert kind == 'qt6', kind
    return [root + '/qt6ct/qt6ct.conf', root + '/qt6ct/colors/realm.conf']


def successful_open(trace, path):
    return any(
        re.match(r'^(?:[0-9]+[ \t]+)?openat\(', line) and f'"{path}"' in line
        and re.search(r'= [0-9]+$', line)
        for line in trace.splitlines()
    )


def validate_probe(kind, root, observation):
    _executable, expected_title, expected_label = TOOLKITS[kind]
    environment = observation['environment']
    assert environment.get('REALM_GENERATION') == root, 'wrong selected generation'
    if kind.startswith('gtk'):
        assert environment.get('GTK_THEME') == 'realm', 'wrong GTK_THEME'
        assert environment.get('XDG_DATA_DIRS', '').split(':')[0] == root + '/share', (
            'wrong XDG_DATA_DIRS', environment.get('XDG_DATA_DIRS')
        )
    else:
        assert environment.get('QT_QPA_PLATFORMTHEME') == 'qt6ct', 'wrong Qt selector'
        assert environment.get('XDG_CONFIG_DIRS', '').split(':')[0] == root, (
            'wrong XDG_CONFIG_DIRS', environment.get('XDG_CONFIG_DIRS')
        )
    state = observation['state']
    assert state.get('reply') == 'state', 'missing real state response'
    assert sum(orbit['windows'] for orbit in state['data']['orbits']) == 1, (
        'wrong managed window count', state
    )
    assert state['data']['focused_title'] == expected_title, (
        'wrong focused title', state['data']['focused_title'], expected_title
    )
    assert expected_label.casefold() in observation['ocr'].casefold(), (
        'missing visible toolkit label', expected_label, observation['ocr']
    )
    diagnostic = QT_DIAGNOSTIC if kind == 'qt6' else GTK_DIAGNOSTIC
    assert not diagnostic.search(observation['stderr']), (
        'toolkit theme diagnostic', observation['stderr'][-8192:]
    )
    assert observation['exit_status'] == 0, ('nonzero toolkit exit', observation['exit_status'])
    for path in required_paths(kind, root):
        assert successful_open(observation['trace'], path), ('selected file not opened', path)


def launcher_assets(kind, probe_dir):
    executable = TOOLKITS[kind][0]
    prefix = probe_dir + '/' + kind
    script = (
        '#!/bin/sh\n'
        + 'env -0 > ' + shlex.quote(prefix + '.environment') + '\n'
        + 'strace -f -qq -e trace=openat -o ' + shlex.quote(prefix + '.trace')
        + ' ' + shlex.quote(executable) + ' 2> ' + shlex.quote(prefix + '.stderr') + '\n'
        + 'status=$?\nprintf "%s\\n" "$status" > '
        + shlex.quote(prefix + '.done') + '\nexit "$status"\n'
    )
    desktop = (
        '[Desktop Entry]\nType=Application\n'
        + 'Name=realm' + kind + 'probe\n'
        + 'Exec=' + prefix + '.sh\nTerminal=false\n'
    )
    return script, desktop


def open_launcher(name, key, wait_ready, wait_filtered):
    key('meta_l-d')
    wait_ready()
    for character in name:
        key(character)
    wait_filtered(name)
    key('ret')


def retain_probe_artifacts(ssh, kind, probe_dir, evidence):
    """Keep bounded guest evidence without replacing the acceptance error."""
    errors = []
    for suffix in ('trace', 'stderr', 'environment', 'done'):
        path = probe_dir + '/' + kind + '.' + suffix
        try:
            completed = subprocess.run(
                ssh + ['test -f ' + shlex.quote(path) + ' && head -c 65536 '
                       + shlex.quote(path)],
                text=True, capture_output=True, timeout=15, check=False,
            )
            if completed.returncode == 0:
                (evidence / (kind + '-' + suffix + '.log')).write_text(completed.stdout)
        except (subprocess.SubprocessError, OSError, UnicodeError) as error:
            errors.append(f'{kind}.{suffix}: {type(error).__name__}: {str(error)[:256]}')
    return errors


PROCESS = r'''
import json, os, pathlib, pwd, sys
uid = pwd.getpwnam('alice').pw_uid
expected = os.path.realpath(sys.argv[1])
matches = []
for entry in pathlib.Path('/proc').iterdir():
    if not entry.name.isdigit():
        continue
    try:
        if entry.stat().st_uid != uid:
            continue
        executable = os.path.realpath(os.readlink(entry / 'exe'))
        if executable != expected:
            continue
        raw = (entry / 'environ').read_bytes().split(b'\0')
        environment = dict(item.decode(errors='replace').split('=', 1)
                           for item in raw if b'=' in item)
        keys = ('REALM_GENERATION', 'GTK_THEME', 'QT_QPA_PLATFORMTHEME',
                'XDG_DATA_DIRS', 'XDG_CONFIG_DIRS')
        matches.append({'pid': int(entry.name), 'executable': executable,
                        'environment': {key: environment.get(key) for key in keys}})
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        continue
assert len(matches) == 1, ('expected one real toolkit process', matches)
print(json.dumps(matches[0]))
'''


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path) / 'toolkit-roundtrip'
    evidence.mkdir()
    probe_dir = '/tmp/realm-native-toolkit'
    result = {'passed': False, 'toolkits': {}}

    def guest(command, script=None, timeout=40):
        return subprocess.check_output(
            ssh + [command], input=script, text=True, timeout=timeout
        )

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

    def read_text(name):
        path = screenshot(name)
        output = subprocess.check_output(
            ssh + ['timeout 10 tesseract stdin stdout -l eng'],
            input=path.read_bytes(), timeout=15,
        ).decode()
        (evidence / (name + '.ocr.txt')).write_text(output)
        return output

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join([
        'sudo', 'runuser', '-u', 'alice', '--', 'env',
        'XDG_RUNTIME_DIR=' + runtime,
        'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus',
    ])

    def current_state():
        guest(user + ' python3 /var/tmp/realm-native-vm/control_get_state.py '
              + runtime + '/realm/ctl.sock ' + probe_dir + '/state.ndjson')
        raw = guest('cat ' + probe_dir + '/state.ndjson')
        return json.loads(raw.splitlines()[-1])

    def focused_state(expected_title, name):
        deadline = time.monotonic() + 30
        observed = []
        while True:
            state = current_state()
            title = state.get('data', {}).get('focused_title')
            if title not in observed and len(observed) < 10:
                observed.append(title)
            windows = sum(orbit['windows'] for orbit in state['data']['orbits'])
            if windows == 1 and title == expected_title:
                (evidence / (name + '-state.json')).write_text(json.dumps(state, indent=2) + '\n')
                return state
            assert time.monotonic() < deadline, (
                'toolkit focus/window deadline', name, expected_title, observed, state
            )
            time.sleep(0.2)

    def relevant_trace(kind):
        path = probe_dir + '/' + kind + '.trace'
        return guest('grep -E -i '
                     + shlex.quote(r'gtk[.]css|qt6ct|realm[.]conf|loaders[.]cache')
                     + ' ' + shlex.quote(path) + ' | head -c 65536')

    try:
        prior = json.loads((Path(evidence_path) / 'consumer-roundtrip.json').read_text())
        assert prior['passed'] and prior['next_generation'] != prior['login']['generation'], prior
        login = json.loads(guest('cat ' + runtime + '/realm/session-theme.json'))
        assert login == prior['login'], (login, prior['login'])
        root = prior['generation_root']
        result['login'] = login
        result['next_generation'] = prior['next_generation']
        result['packages'] = guest('cat /var/tmp/realm-native-toolkit-packages.txt')
        guest('sudo install -d -o alice -g alice -m 0700 ' + probe_dir
              + ' && sudo install -d -o alice -g alice -m 0755 '
              '/home/alice/.local/share/applications')
        initial = current_state()
        assert sum(orbit['windows'] for orbit in initial['data']['orbits']) == 0, initial
        for kind, (executable, title, label) in TOOLKITS.items():
            script, desktop = launcher_assets(kind, probe_dir)
            script_path = probe_dir + '/' + kind + '.sh'
            desktop_path = '/home/alice/.local/share/applications/realm-native-' + kind + '.desktop'
            guest('sudo -u alice tee ' + shlex.quote(script_path) + ' >/dev/null', script)
            guest('sudo -u alice chmod 0755 ' + shlex.quote(script_path))
            guest('sudo -u alice tee ' + shlex.quote(desktop_path) + ' >/dev/null', desktop)
            name = 'realm' + kind + 'probe'
            probe = {'passed': False, 'expected_title': title, 'expected_label': label}
            result['toolkits'][kind] = probe
            try:
                open_launcher(
                    name, key,
                    lambda: guest(
                        'for attempt in $(seq 1 100); do pgrep -u alice -x fuzzel '
                        '>/dev/null && exit 0; sleep 0.1; done; exit 1', timeout=15,
                    ),
                    lambda expected: wait_visible(
                        lambda: read_text(kind + '-launcher-filtered'), expected,
                    ),
                )
                probe['state'] = focused_state(title, kind)
                process = json.loads(guest('sudo python3 - ' + shlex.quote(executable), PROCESS))
                probe['process'] = process
                probe['environment'] = process['environment']
                probe['ocr'] = wait_visible(lambda: read_text(kind + '-visible'), label)
                screenshot(kind + '-accepted')
                key('meta_l-q')
                deadline = time.monotonic() + 20
                while True:
                    current = current_state()
                    if sum(orbit['windows'] for orbit in current['data']['orbits']) == 0:
                        break
                    assert time.monotonic() < deadline, ('toolkit did not close', kind, current)
                    time.sleep(0.2)
                done = probe_dir + '/' + kind + '.done'
                guest('for attempt in $(seq 1 200); do test -s ' + shlex.quote(done)
                      + ' && exit 0; sleep 0.1; done; exit 1', timeout=25)
                probe['exit_status'] = int(guest('cat ' + shlex.quote(done)).strip())
                probe['trace'] = relevant_trace(kind)
                probe['stderr'] = guest('head -c 65536 ' + shlex.quote(probe_dir + '/' + kind + '.stderr'))
                validate_probe(kind, root, probe)
                probe['passed'] = True
            finally:
                artifact_errors = retain_probe_artifacts(ssh, kind, probe_dir, evidence)
                if artifact_errors:
                    probe['artifact_errors'] = artifact_errors
                (evidence / 'toolkit-roundtrip.json').write_text(
                    json.dumps(result, indent=2) + '\n'
                )
        result['passed'] = True
    finally:
        (evidence / 'toolkit-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
