"""CI-only installed window controls: real keys, read-only state, frame evidence."""
import json
from pathlib import Path
import shlex
import subprocess
import sys
import time

from lock_roundtrip import complete_ppm, monitor_command


def active(value):
    selected = [orbit for orbit in value['ledger'] if orbit['active']]
    assert len(selected) == 1, value
    return selected[0]


def exercise_controls(wait, key, screenshot):
    initial = wait(lambda value: len(active(value)['windows']) == 3, 'three fixture windows')
    orbit = active(initial)
    home = orbit['orbit']
    windows = orbit['windows']
    order = [window['id'] for window in windows]
    titles = {window['id']: window['title'] for window in windows}
    assert len(titles) == 3 and set(titles.values()) == {'Realm window A', 'Realm window B', 'Realm window C'}, windows
    focused = [window['id'] for window in windows if window['focused']]
    assert len(focused) == 1, windows
    focus = focused[0]
    neighbour = (order.index(focus) + 1) % 3

    def matches(value, expected_order, expected_focus):
        current = active(value)
        current_windows = current['windows']
        return (current['orbit'] == home
                and [window['id'] for window in current_windows] == expected_order
                and {window['id']: window['title'] for window in current_windows} == titles
                and [window['id'] for window in current_windows if window['focused']] == [expected_focus]
                and value['state']['focused_title'] == titles[expected_focus])

    key('meta_l-j')
    wait(lambda value: matches(value, order, order[neighbour]), 'focus next')
    key('meta_l-k')
    wait(lambda value: matches(value, order, focus), 'focus previous')
    swapped = list(order)
    index = order.index(focus)
    swapped[index], swapped[neighbour] = swapped[neighbour], swapped[index]
    key('meta_l-l')
    wait(lambda value: matches(value, swapped, focus), 'swap next preserves focus')
    screenshot('window-swapped')
    key('meta_l-h')
    wait(lambda value: matches(value, order, focus), 'swap previous restores order')

    empty = next(orbit['orbit'] for orbit in initial['ledger'] if not orbit['windows'] and orbit['orbit'] != home)
    key(f'meta_l-{empty}')
    wait(lambda value: active(value)['orbit'] == empty and not active(value)['windows']
         and value['state']['focused_title'] == '', 'empty orbit')
    screenshot('window-empty-orbit')
    key(f'meta_l-{home}')
    wait(lambda value: matches(value, order, focus), 'return orbit preserves windows')

    for chord, layout in (('meta_l-m', 'mono'), ('meta_l-t', 'triptych')):
        key(chord)
        wait(lambda value: matches(value, order, focus) and value['state']['layout'] == layout
             and active(value)['layout'] == layout, layout)
        screenshot('window-' + layout)

    shown = initial['state']['whichkey']
    if shown:
        key('meta_l-w')
        wait(lambda value: value['state']['whichkey'] is False, 'which-key initially hidden')
    for desired in (True, False):
        key('meta_l-w')
        wait(lambda value: value['state']['whichkey'] is desired, 'which-key ' + str(desired))
        screenshot('window-whichkey-' + ('shown' if desired else 'dismissed'))
    if shown:
        key('meta_l-w')
        wait(lambda value: value['state']['whichkey'] is True, 'which-key restored')
    key('meta_l-shift-0x35')
    wait(lambda value: value['state']['grimoire'] is True, 'full key help open')
    screenshot('window-help')
    key('meta_l-esc')
    wait(lambda value: value['state']['grimoire'] is False and matches(value, order, focus), 'full key help dismissed')
    screenshot('window-help-dismissed')


# Only observational protocol requests; actions are always QEMU keyboard input.
SNAPSHOT = r'''
import json, socket, sys
requests = [{'cmd':'hello','arg':{'version':2,'client':'realm-native-window-probe'}},
            {'cmd':'get-state'}, {'cmd':'show-ledger','arg':None}]
frames = []
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
    connection.settimeout(5)
    connection.connect(sys.argv[1])
    with connection.makefile('rwb', buffering=0) as stream:
        for request in requests:
            stream.write(json.dumps(request).encode() + b'\n')
            line = stream.readline(1024 * 1024)
            assert line.endswith(b'\n'), 'missing or oversized state frame'
            frames.append(json.loads(line))
assert [frame['reply'] for frame in frames] == ['hello','state','ledger'], frames
assert frames[0]['data']['version'] == 2, frames
print(json.dumps({'state':frames[1]['data'],'ledger':frames[2]['data']}))
'''


def main():
    monitor, evidence_path, *ssh = sys.argv[1:]
    evidence = Path(evidence_path)
    result = {'passed': False, 'observations': []}

    def guest(command, script=None):
        return subprocess.check_output(ssh + [command], input=script, text=True, timeout=20)

    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    runtime = '/run/user/' + uid
    user = shlex.join(['sudo', 'runuser', '-u', 'alice', '--', 'env',
                      'XDG_RUNTIME_DIR=' + runtime,
                      'DBUS_SESSION_BUS_ADDRESS=unix:path=' + runtime + '/bus'])

    def key(chord):
        monitor_command(monitor, 'sendkey ' + chord)
        time.sleep(0.15)  # Let QEMU release its default 100ms key hold.

    def wait(predicate, label):
        deadline = time.monotonic() + 20
        while True:
            value = json.loads(guest(user + ' python3 - ' + runtime + '/realm/ctl.sock', SNAPSHOT))
            result['last_observation'] = value
            if predicate(value):
                result['observations'].append({'step': label, **value})
                return value
            assert time.monotonic() < deadline, (label, value)
            time.sleep(0.1)

    def screenshot(name):
        path = evidence / (name + '.ppm')
        assert not any(character.isspace() for character in str(path)), path
        monitor_command(monitor, 'screendump ' + str(path))
        deadline = time.monotonic() + 5
        while not complete_ppm(path):
            assert time.monotonic() < deadline, path
            time.sleep(0.05)

    def count(value):
        return sum(len(orbit['windows']) for orbit in value['ledger'])

    try:
        wait(lambda value: count(value) == 0 and not value['state']['grimoire'], 'empty session before window probe')
        for number, letter in enumerate('ABC', 1):
            script = '/tmp/win' + letter.lower()
            guest('sudo tee ' + script + ' >/dev/null',
                  "printf '\\033]0;Realm window " + letter + "\\007'\n"
                  "printf '\\nRealm window " + letter + "\\nKeyboard acceptance fixture\\n'\n"
                  "exec sleep infinity\n")
            key('meta_l-ret')
            wait(lambda value: count(value) == number, 'terminal ' + letter)
            for character in 'source ' + script:
                key({' ': 'spc', '/': 'slash'}.get(character, character))
            key('ret')
            wait(lambda value: value['state']['focused_title'] == 'Realm window ' + letter,
                 'titled terminal ' + letter)
        exercise_controls(wait, key, screenshot)
        for remaining in (2, 1, 0):
            key('meta_l-q')
            wait(lambda value: count(value) == remaining, 'close fixture window ' + str(remaining))
        result['passed'] = True
    finally:
        print(json.dumps(result, indent=2), flush=True)
        (evidence / 'window-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        for name, collect in (
            ('screenshot', lambda: screenshot('window-final')),
            ('journal', lambda: guest('sudo journalctl -b -n 160 --no-pager _UID=' + uid)),
        ):
            try:
                output = collect()
                if output is not None:
                    (evidence / ('window-' + name + '.txt')).write_text(output)
            except Exception as error:
                print(f'window diagnostic {name} failed: {error}', file=sys.stderr, flush=True)


if __name__ == '__main__':
    main()
