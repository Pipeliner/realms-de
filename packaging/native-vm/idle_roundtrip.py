"""CI-only real native idle timing, using the manual-lock transport adapters."""
import json
import sys
import time


def timed_roundtrip(start, observe, snapshot, key, password, suppressed, gone,
                    screenshot, blank, restored, stop, journal, evidence,
                    sleep=time.sleep, clock=time.monotonic):
    result = {'readiness_verified': False, 'backlight': 'absent'}
    try:
        result.update(start())
        baseline = result['baseline']
        deadline = clock() + 640
        locked = None
        while clock() < deadline:
            observation = observe()
            events = observation['events']
            if events and 'dim_elapsed_seconds' not in result:
                elapsed = events[0] - baseline
                result['dim_elapsed_seconds'] = elapsed
                assert 299 <= elapsed <= 330, result
            if observation['lock']['state'] == 'active':
                elapsed = observation['lock_time'] - baseline
                result['lock_elapsed_seconds'] = elapsed
                assert 599 <= elapsed <= 630, result
                locked = observation['lock']
                break
            sleep(5)
        assert 'dim_elapsed_seconds' in result and locked and locked['processes'], result
        result['locker_processes'] = locked['processes']
        blank()
        key('meta_l-d')
        suppressed()
        key('ctrl-u')
        password('wrongpassword')
        sleep(5)
        assert snapshot() == locked, 'wrong password did not preserve idle lock'
        result['wrong_password_rejected'] = True
        screenshot('idle-wrong-password')
        key('ctrl-u')
        password('realmtest')
        for _ in range(30):
            if snapshot()['state'] == 'inactive':
                break
            sleep(1)
        else:
            raise AssertionError('correct password did not unlock idle lock')
        gone(locked['processes'])
        result['password_unlock'] = True
        key('meta_l-d')
        restored(True)
        key('esc')
        restored(False)
        result['launcher_binding_restored'] = True
        assert len(observe()['events']) >= 2, 'missing activity restore no-op'
        result['resume_backlight_noop'] = True
        screenshot('idle-unlocked')
    finally:
        failed = sys.exc_info()[0] is not None
        cleanup_error = None
        try:
            stop()
            result['stopped_without_live_idle'] = True
        except Exception as error:
            cleanup_error = error
            result['stop_error'] = str(error)
        # Diagnostics must never replace the acceptance/transport exception.
        try:
            (evidence / 'idle-journal.jsonl').write_text(journal())
        except Exception as error:
            result['journal_error'] = str(error)
        result['readiness_verified'] = not failed and cleanup_error is None
        print('idle-roundtrip: ' + json.dumps(result, sort_keys=True), flush=True)
        try:
            (evidence / 'idle-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        except Exception:
            if not failed and cleanup_error is None:
                raise
        if cleanup_error is not None and not failed:
            raise cleanup_error
    return result


def run(guest, user, key, password, snapshot, suppressed, gone, screenshot, restored, evidence):
    identity = {}
    baseline = None

    def systemctl(arguments):
        return guest(user + ' timeout 30 systemctl --user ' + arguments).strip()

    def journal():
        return guest('sudo journalctl -b -n 200 --no-pager -o json _SYSTEMD_USER_UNIT=realm-idle.service')

    def start():
        nonlocal baseline
        guest('test -z "$(ls -A /sys/class/backlight)"')
        assert snapshot()['state'] == 'inactive', 'manual fixture must leave locker inactive'
        systemctl('start realm-idle.service')
        pid = systemctl('show realm-idle.service -p MainPID --value')
        assert pid.isdigit() and int(pid) > 0, pid
        metadata = json.loads(guest('sudo python3 -', '''import json, pathlib
p = pathlib.Path('/proc/''' + pid + "''')\n" + '''
print(json.dumps({'argv': p.joinpath('cmdline').read_bytes().decode().rstrip('\\0').split('\\0'),
 'start_time': int(p.joinpath('stat').read_text().rsplit(')', 1)[1].split()[19]),
 'executable': str(p.joinpath('exe').resolve(strict=True))}))
'''))
        assert metadata['executable'].split('/')[-1] == 'swayidle', metadata
        assert metadata['argv'][1:] == [
            '-w', '-C', '/dev/null',
            'timeout', '300', 'realm-backlight dim', 'resume', 'realm-backlight restore',
            'timeout', '600', 'systemctl --user start realm-lock.service',
            'before-sleep', 'systemctl --user start realm-lock.service',
            'lock', 'systemctl --user start realm-lock.service'], metadata
        identity[pid] = {'start_time': metadata['start_time']}
        time.sleep(1)  # Allow upstream idle notifications to bind before input.
        baseline = float(guest("python3 -c 'import time; print(time.monotonic())'"))
        key('esc')
        return {'baseline': baseline, 'idle_pid': pid, **metadata}

    def observe():
        events = [int(entry['__MONOTONIC_TIMESTAMP']) / 1_000_000
                  for entry in map(json.loads, filter(None, journal().splitlines()))
                  if 'backlight adjustment unavailable' in entry.get('MESSAGE', '')
                  and int(entry['__MONOTONIC_TIMESTAMP']) / 1_000_000 >= baseline]
        return {'events': sorted(events), 'lock': snapshot(),
                'lock_time': int(systemctl('show realm-lock.service -p ActiveEnterTimestampMonotonic --value')) / 1_000_000}

    def blank():
        deadline = time.monotonic() + 5
        while True:
            path = screenshot('idle-locked')  # Adapter requires a complete P6.
            with path.open('rb') as stream:
                assert stream.readline() == b'P6\n'
                stream.readline()
                assert stream.readline() == b'255\n'
                pixels = stream.read()
            if pixels and pixels == pixels[:3] * (len(pixels) // 3):
                return
            if time.monotonic() >= deadline:
                raise AssertionError('idle lock frame is not uniformly opaque')
            time.sleep(0.2)

    def stop():
        systemctl('stop realm-idle.service')
        assert systemctl('show realm-idle.service -p ActiveState --value') == 'inactive'
        gone(identity)

    return timed_roundtrip(start, observe, snapshot, key, password, suppressed,
                           gone, screenshot, blank, restored, stop, journal, evidence)
