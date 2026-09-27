"""CI-only real native idle timing, using the manual-lock transport adapters."""
import json
import hashlib
import sys
import time
import uuid


def trace_scripts(path, config, token):
    marker = f'# realm-ci-owner {token}\n'
    prefix = f'from pathlib import Path\np = Path({path!r})\n'
    install = prefix + f'''import os, tempfile
p.parent.mkdir(parents=True, exist_ok=True)
fd, temporary = tempfile.mkstemp(prefix='.realm-ci-', dir=p.parent)
try:
    with os.fdopen(fd, 'w') as stream:
        os.fchmod(stream.fileno(), 0o644)
        stream.write({(marker + config)!r})
    os.link(temporary, p)  # Exclusive publication; never replace an existing file.
finally:
    os.unlink(temporary)
'''
    cleanup = prefix + f'''if p.exists() and p.read_text().startswith({marker!r}):
    p.unlink()
'''
    return install, cleanup


def lock_trace_config(version, distro):
    # 1.7.2's daemon preserves stderr; 1.8.6 redirects it to /dev/null.
    # Restrict this disposable CI diagnostic to the observed Ubuntu package.
    if (version, distro) != ('swaylock version 1.7.2', 'ubuntu'):
        return None
    return ('[Service]\nEnvironment=WAYLAND_DEBUG=client\nStandardError=journal\n'
            'LogRateLimitIntervalSec=1h\nLogRateLimitBurst=1000\n')


def failed_blank(original, guest, key, screenshot, evidence):
    """Gather bounded evidence, never turn redraw success into acceptance."""
    errors = {}
    def attempt(name, action):
        try:
            action()
        except Exception as error:
            errors[name] = str(error)[:2000]
    def retain(name, command, limit):
        output = guest(command)
        (evidence / name).write_bytes(output.encode()[:limit])
    journal = ("sudo timeout 5 sh -c 'journalctl -b -n 1000 --no-pager "
               "-o short-monotonic _SYSTEMD_USER_UNIT=realm-lock.service | head -c 65536'")
    attempt('compositor', lambda: retain('idle-compositor-stderr.txt',
        "sudo timeout 5 sh -c 'head -c 65536 /home/alice/.local/share/sddm/wayland-session.log; "
        "tail -c 65536 /home/alice/.local/share/sddm/wayland-session.log'", 131072))
    attempt('protocol-before', lambda: retain('idle-lock-protocol-before.txt', journal, 65536))
    # A modifier cannot authenticate. No Enter/password and no acceptance retry.
    def redraw():
        key('ctrl')
        time.sleep(0.25)
        screenshot('idle-failed-redraw')
    attempt('redraw', redraw)
    attempt('protocol-after', lambda: retain('idle-lock-protocol-after.txt', journal, 65536))
    attempt('errors', lambda: (evidence / 'idle-diagnostic-errors.json').write_text(
        json.dumps(errors, indent=2) + '\n'))
    raise original


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
        assert any(event > baseline + result['lock_elapsed_seconds']
                   for event in observe()['events']), 'missing activity restore no-op'
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
        evidence_error = None
        try:
            (evidence / 'idle-journal.jsonl').write_text(journal())
        except Exception as error:
            evidence_error = error
            result['journal_error'] = str(error)
        result['readiness_verified'] = not failed and cleanup_error is None and evidence_error is None
        print('idle-roundtrip: ' + json.dumps(result, sort_keys=True), flush=True)
        try:
            (evidence / 'idle-roundtrip.json').write_text(json.dumps(result, indent=2) + '\n')
        except Exception:
            if not failed and cleanup_error is None and evidence_error is None:
                raise
        if cleanup_error is not None and not failed:
            raise cleanup_error
        if evidence_error is not None and not failed:
            raise evidence_error
    return result


def run(guest, user, key, password, snapshot, suppressed, gone, screenshot, restored, evidence):
    identity = {}
    baseline = None
    uid = guest('id -u alice').strip()
    assert uid.isdigit(), uid
    trace_path = f'/run/user/{uid}/systemd/user/realm-lock.service.d/90-ci-protocol.conf'
    trace_installed = False
    trace_cleanup = None

    def systemctl(arguments):
        return guest(user + ' timeout 30 systemctl --user ' + arguments).strip()

    def journal():
        return guest('sudo journalctl -b -n 200 --no-pager -o json '
                     f'SYSLOG_IDENTIFIER=realm-idle _UID={uid}')

    def start():
        nonlocal baseline, trace_installed, trace_cleanup
        guest('test -z "$(ls -A /sys/class/backlight)"')
        assert snapshot()['state'] == 'inactive', 'manual fixture must leave locker inactive'
        config = lock_trace_config(guest('swaylock --version').strip(),
                                   guest('. /etc/os-release; printf "%s" "$ID"').strip())
        if config:
            # Runtime-only drop-in; leave shipped command and renderer untouched.
            install, trace_cleanup = trace_scripts(trace_path, config, uuid.uuid4().hex)
            # The remote write may succeed even if its SSH response is lost.
            trace_installed = True
            guest('sudo python3 -', install)
            systemctl('daemon-reload')
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
                  if entry.get('MESSAGE') == 'realm: backlight adjustment unavailable; idle locking remains enabled'
                  and int(entry['__MONOTONIC_TIMESTAMP']) / 1_000_000 >= baseline]
        return {'events': sorted(events), 'lock': snapshot(),
                'lock_time': int(systemctl('show realm-lock.service -p ActiveEnterTimestampMonotonic --value')) / 1_000_000}

    def blank():
        deadline = time.monotonic() + 5
        attempts = []
        while True:
            started = time.monotonic()
            path = screenshot(f'idle-locked-{len(attempts):03d}')
            with path.open('rb') as stream:
                assert stream.readline() == b'P6\n'
                stream.readline()
                assert stream.readline() == b'255\n'
                pixels = stream.read()
            completed = time.monotonic()
            uniform = bool(pixels) and pixels == pixels[:3] * (len(pixels) // 3)
            attempts.append({'path': path.name,
                             'started_monotonic': started,
                             'completed_monotonic': completed,
                             'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                             'uniform': uniform})
            (evidence / 'idle-blank-attempts.json').write_text(json.dumps(attempts, indent=2) + '\n')
            if uniform:
                return
            if completed >= deadline:
                failed_blank(AssertionError('idle lock frame is not uniformly opaque'),
                             guest, key, screenshot, evidence)
            time.sleep(0.2)

    def stop():
        try:
            systemctl('stop realm-idle.service')
            assert systemctl('show realm-idle.service -p ActiveState --value') == 'inactive'
            gone(identity)
        finally:
            failed = sys.exc_info()[0] is not None
            if trace_installed:
                try:
                    guest('sudo python3 -', trace_cleanup)
                    systemctl('daemon-reload')
                except Exception:
                    if not failed:
                        raise

    return timed_roundtrip(start, observe, snapshot, key, password, suppressed,
                           gone, screenshot, blank, restored, stop, journal, evidence)
