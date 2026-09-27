"""CI-only native Foot/Zsh/Yazi selected-login consumption evidence."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from lock_roundtrip import complete_ppm, monitor_command


def select_foot_config(root, check):
    for variant in ('foot-modern.ini', 'foot.ini'):
        path = root + '/foot/' + variant
        if check(path):
            return path
    raise AssertionError('installed Foot rejects both generated configuration variants')


def collect_shell_proofs(process, guest):
    # The sourced shell writes both files before executing Yazi. Its real exec
    # is the completion barrier; the first file alone could precede the second.
    files = process('yazi')
    version = guest('cat /tmp/realm-native-shell-proof').strip()
    tools = guest('cat /tmp/realm-native-tools-proof').splitlines()
    return files, version, tools


def assert_consumer(process, root, executable):
    assert process['executable'] == executable, process
    environment = process['environment']
    for key, expected in {
        'REALM_GENERATION': root, 'ZDOTDIR': root + '/zsh',
        'STARSHIP_CONFIG': root + '/starship.toml',
        'YAZI_CONFIG_HOME': root + '/yazi', 'GTK_THEME': 'realm',
        'QT_QPA_PLATFORMTHEME': 'qt6ct',
    }.items():
        assert environment.get(key) == expected, (key, expected, process)
    assert environment['XDG_DATA_DIRS'].split(':')[0] == root + '/share', process
    assert environment['XDG_CONFIG_DIRS'].split(':')[0] == root, process


def consumer_process_command(name, selected_config, mode='present'):
    return shlex.join(['sudo', 'python3', '/var/tmp/realm-native-vm/consumer_process.py',
                       name, selected_config if name == 'foot' else '', mode])


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path)
    result = {'passed': False, 'processes': {}}

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=40)

    def wait(command):
        guest('for attempt in $(seq 1 100); do if ' + command + '; then exit 0; fi; sleep 0.1; done; exit 1')

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

    def process(name):
        command = consumer_process_command(name, selected_config)
        wait(command + ' >/dev/null 2>&1')
        value = json.loads(guest(command))
        result['processes'][name] = value
        (evidence / 'consumer-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        return value

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus',
                      'XDG_CONFIG_HOME=/home/alice/.config'])

    def state(count, name):
        deadline = time.monotonic() + 15
        while True:
            guest(user + ' python3 /var/tmp/realm-native-vm/control_get_state.py '
                  + runtime + '/realm/ctl.sock /tmp/realm-native-consumer-state')
            raw = guest('cat /tmp/realm-native-consumer-state')
            frames = [json.loads(line) for line in raw.splitlines()]
            current = frames[-1]
            if current.get('reply') == 'state' and sum(orbit['windows'] for orbit in current['data']['orbits']) == count:
                (evidence / (name + '-state.ndjson')).write_text(raw)
                return
            assert time.monotonic() < deadline, current
            time.sleep(0.1)

    try:
        # Explicit #236 prerequisite: absence must fail, not select distro tools.
        guest('test -x /usr/lib/realm/bin/yazi && test -x /usr/lib/realm/bin/starship')
        login = json.loads(guest('cat ' + runtime + '/realm/session-theme.json'))
        generation = login['generation']
        root = '/home/alice/.config/realm/generated/generations/' + generation
        result.update(login=login, generation_root=root)
        result['foot_version'] = guest('/usr/bin/foot --version').strip()
        result['foot_config_probes'] = []
        def check_foot(path):
            completed = subprocess.run(ssh + [user + ' timeout 5 /usr/bin/foot --check-config --config=' + shlex.quote(path)],
                                       text=True, capture_output=True, timeout=15)
            result['foot_config_probes'].append({'path': path, 'returncode': completed.returncode,
                                                'stdout': completed.stdout[-16384:], 'stderr': completed.stderr[-16384:]})
            return completed.returncode == 0
        selected_config = select_foot_config(root, check_foot)
        result['foot_config'] = selected_config
        state(0, 'consumer-before')
        key('meta_l-ret')
        terminal = process('foot')
        shell = process('zsh')
        assert_consumer(terminal, root, '/usr/bin/foot')
        assert_consumer(shell, root, '/usr/bin/zsh')
        assert shell['parent_pid'] == terminal['pid'], (terminal, shell)
        assert terminal['arguments'] == ['foot', '--config=' + selected_config,
            '--log-level=error', '--override=key-bindings.spawn-terminal=none', 'zsh'], terminal
        state(1, 'consumer-terminal')
        screenshot('consumer-terminal-a')

        guest(user + ' timeout 30 realmctl theme apply')
        next_generation = guest('cat /home/alice/.config/realm/generated/current').strip()
        assert next_generation != generation, (next_generation, generation)
        assert json.loads(guest('cat ' + runtime + '/realm/session-theme.json')) == login
        result['next_generation'] = next_generation
        assert process('foot') == terminal
        assert process('zsh') == shell

        guest("sudo install -d -o alice -g alice /tmp/realm-native-files && sudo -u alice touch /tmp/realm-native-files/realm-visible-file")
        guest('sudo tee /tmp/proof >/dev/null', '''cd /tmp/realm-native-files || return 1
printf '%s\\n' "$ZSH_VERSION" > /tmp/realm-native-shell-proof
command -v yazi starship > /tmp/realm-native-tools-proof
yazi
printf '%s\\n' "$?" > /tmp/realm-native-yazi-exit
''')
        # Actual input to the retained interactive shell, not SSH-launched Yazi.
        for character in 'source /tmp/proof':
            key({' ': 'spc', '/': 'slash'}.get(character, character))
        key('ret')
        files, result['zsh_version'], tools = collect_shell_proofs(process, guest)
        assert tools == ['/usr/lib/realm/bin/yazi', '/usr/lib/realm/bin/starship'], tools
        result['resolved_tools'] = tools
        assert_consumer(files, root, '/usr/lib/realm/bin/yazi')
        assert files['parent_pid'] == shell['pid'], files
        state(1, 'consumer-yazi')
        screenshot('consumer-yazi-after-apply-b')
        key('q')
        wait('test -s /tmp/realm-native-yazi-exit && ! pgrep -u alice -x yazi')
        assert guest('cat /tmp/realm-native-yazi-exit').strip() == '0'
        key('meta_l-q')
        selected_foot_absent = consumer_process_command('foot', selected_config, 'absent')
        wait(selected_foot_absent + ' && ! pgrep -u alice -x zsh')
        state(0, 'consumer-closed')
        result['passed'] = True
    finally:
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'consumer-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        for name, action in (
            ('screenshot', lambda: screenshot('consumer-final')),
            ('journal', lambda: guest('sudo journalctl -b -n 200 --no-pager _UID=' + uid)),
        ):
            try:
                output = action()
                if output is not None:
                    (evidence / ('consumer-' + name + '.txt')).write_text(output)
            except Exception as error:
                print(f'consumer diagnostic {name} failed: {error}', file=sys.stderr)


if __name__ == '__main__':
    main()
