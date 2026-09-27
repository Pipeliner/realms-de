"""Native timing acceptance decisions; injected external clock/VM boundaries only."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import json
import hashlib


class IdleTests(unittest.TestCase):
    def fixture(self, dim=300, lock=600, wrong_unlock=False, correct_unlock=True,
                resume=True, stop_error=False, missing_lock=False, restoration_fails=False,
                journal_error=False, journal_write_error=False, duplicate_dim=False):
        path = Path(__file__).with_name('idle_roundtrip.py')
        self.assertTrue(path.exists(), 'native real idle acceptance probe is missing')
        spec = importlib.util.spec_from_file_location('idle_roundtrip', path)
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        state = {'now': 0, 'unlocked': False, 'activity': False, 'stopped': False}
        locked = {'state': 'active', 'processes': {'17': {'start_time': 22}}}
        def snapshot():
            active = state['now'] >= lock and not state['unlocked'] and not missing_lock
            return locked if active else {'state': 'inactive', 'processes': {}}
        def observe():
            events = [dim] if state['now'] >= dim else []
            if events and duplicate_dim:
                events.append(dim + 0.001)
            if state['activity'] and resume:
                events.append(state['now'])
            return {'events': events, 'lock_time': lock, 'lock': snapshot()}
        def password(value):
            state['activity'] = True
            if wrong_unlock or (value == 'realmtest' and correct_unlock):
                state['unlocked'] = True
        def sleep(seconds):
            state['now'] += seconds
        def stop():
            state['stopped'] = True
            if stop_error:
                raise RuntimeError('stop failed')
        def restored(present):
            assert not restoration_fails, 'launcher restoration failed'
        def journal():
            if journal_error:
                raise RuntimeError('journal collection failed')
            return ''
        with tempfile.TemporaryDirectory() as directory:
            if journal_write_error:
                (Path(directory) / 'idle-journal.jsonl').mkdir()
            try:
                result = probe.timed_roundtrip(lambda: {'baseline': 0}, observe,
                    snapshot, lambda key: None, password, lambda: None,
                    lambda old: None, lambda name: None, lambda: None,
                    restored, stop, journal, Path(directory), sleep, lambda: state['now'])
                self.assertTrue(result['password_unlock'])
                self.assertEqual(result['dim_elapsed_seconds'], 300)
                self.assertEqual(result['lock_elapsed_seconds'], 600)
            finally:
                self.assertTrue(state['stopped'])
                self.assertTrue((Path(directory) / 'idle-roundtrip.json').exists())
                if journal_error or journal_write_error:
                    saved = json.loads((Path(directory) / 'idle-roundtrip.json').read_text())
                    self.assertFalse(saved['readiness_verified'])

    def test_journal_collection_or_persistence_failure_blocks_readiness(self):
        with self.assertRaisesRegex(RuntimeError, 'journal collection failed'):
            self.fixture(journal_error=True)
        with self.assertRaises(IsADirectoryError):
            self.fixture(journal_write_error=True)

    def test_journal_failure_preserves_acceptance_and_cleanup_errors(self):
        for option in ('journal_error', 'journal_write_error'):
            with self.subTest(option=option), self.assertRaises(AssertionError):
                self.fixture(dim=30, **{option: True})
            with self.subTest(option=option), self.assertRaisesRegex(RuntimeError, 'stop failed'):
                self.fixture(stop_error=True, **{option: True})

    def test_real_timer_acceptance(self):
        self.fixture()

    def test_short_or_missing_timers_rejected(self):
        for options in ({'dim': 30}, {'lock': 60}, {'dim': 900}, {'missing_lock': True}):
            with self.subTest(options=options), self.assertRaises(AssertionError):
                self.fixture(**options)

    def test_auth_resume_and_cleanup_failures_rejected(self):
        for options in ({'wrong_unlock': True}, {'correct_unlock': False}, {'resume': False}, {'restoration_fails': True}):
            with self.subTest(options=options), self.assertRaises(AssertionError):
                self.fixture(**options)
        with self.assertRaisesRegex(RuntimeError, 'stop failed'):
            self.fixture(stop_error=True)

    def test_two_lines_from_one_dim_callback_cannot_prove_activity_restore(self):
        with self.assertRaisesRegex(AssertionError, 'missing activity restore no-op'):
            self.fixture(resume=False, duplicate_dim=True)

    def test_cleanup_cannot_mask_original_failure(self):
        with self.assertRaises(AssertionError):
            self.fixture(dim=30, stop_error=True)

    def test_real_adapters_validate_guest_script_argv_events_and_identity(self):
        spec = importlib.util.spec_from_file_location('idle_roundtrip', Path(__file__).with_name('idle_roundtrip.py'))
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        commands = []
        keys = []
        identities = []
        frames = {'calls': 0, 'never_blank': False}
        names = []
        argv = ['swayidle', '-w', '-C', '/dev/null',
                'timeout', '300', 'realm-backlight dim', 'resume', 'realm-backlight restore',
                'timeout', '600', 'systemctl --user start realm-lock.service',
                'before-sleep', 'systemctl --user start realm-lock.service',
                'lock', 'systemctl --user start realm-lock.service']
        def guest(command, script=None):
            commands.append(command)
            if script is not None:
                compile(script, '<guest metadata>', 'exec')
                return json.dumps({'argv': argv, 'start_time': 44, 'executable': '/usr/bin/swayidle'})
            if '-p MainPID' in command:
                return '22'
            if 'time.monotonic' in command:
                return '100'
            if command == 'id -u alice':
                return '1001'
            if 'journalctl' in command:
                self.assertIn(' -b ', command)
                self.assertIn('_UID=1001', command)
                entries = [
                    {'__MONOTONIC_TIMESTAMP': str(t * 1000000),
                     'MESSAGE': message,
                     'SYSLOG_IDENTIFIER': 'realm-idle', '_UID': '1001',
                     '_BOOT_ID': 'fixture-boot',
                     **({'_SYSTEMD_USER_UNIT': 'realm-idle.service'} if child else {})}
                    for t in (90, 400)
                    for child, message in (
                        (True, "Failed to read any devices of class 'backlight'."),
                        (False, 'realm: backlight adjustment unavailable; idle locking remains enabled'),
                    )
                ]
                entries.append({
                    '__MONOTONIC_TIMESTAMP': '410000000',
                    'MESSAGE': 'realm: backlight adjustment unavailable; unrelated diagnostic',
                    'SYSLOG_IDENTIFIER': 'realm-idle', '_UID': '1001',
                    '_BOOT_ID': 'fixture-boot',
                })
                if '_SYSTEMD_USER_UNIT=' in command:
                    entries = [entry for entry in entries if '_SYSTEMD_USER_UNIT' in entry]
                return '\n'.join(json.dumps(entry) for entry in entries)
            if '-p ActiveEnterTimestampMonotonic' in command:
                return '700000000'
            if '-p ActiveState' in command:
                return 'inactive'
            return ''
        def exercise(start, observe, snapshot, key, password, suppressed, gone,
                     screenshot, blank, restored, stop, journal, evidence):
            self.assertEqual(start()['baseline'], 100)
            self.assertEqual(observe()['events'], [400])
            self.assertEqual(observe()['lock_time'], 700)
            blank()
            self.assertEqual(frames['calls'], 2, 'first transitional frame must be retried')
            self.assertEqual(names, ['idle-locked-000', 'idle-locked-001'])
            retained = json.loads((evidence / 'idle-blank-attempts.json').read_text())
            self.assertEqual([attempt['uniform'] for attempt in retained], [False, True])
            for attempt in retained:
                self.assertEqual(attempt['sha256'], hashlib.sha256((evidence / attempt['path']).read_bytes()).hexdigest())
                self.assertLessEqual(attempt['started_monotonic'], attempt['completed_monotonic'])
            frames['never_blank'] = True
            with patch.object(probe.time, 'monotonic', side_effect=[0, 0, 6]), self.assertRaisesRegex(AssertionError, 'uniformly opaque'):
                blank()
            stop()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'blank.ppm'
            def screenshot(name):
                names.append(name)
                path = Path(directory) / (name + '.ppm')
                frames['calls'] += 1
                pixels = b'\x22' * 6 if frames['calls'] > 1 and not frames['never_blank'] else b'\x22' * 3 + b'\x33' * 3
                path.write_bytes(b'P6\n2 1\n255\n' + pixels)
                return path
            with patch.object(probe, 'timed_roundtrip', side_effect=exercise), patch.object(probe.time, 'sleep'):
                probe.run(guest, 'USER', keys.append, lambda text: None,
                    lambda: {'state': 'inactive'}, lambda: None,
                    identities.append, screenshot, lambda present: None, Path(directory))
        self.assertEqual(keys, ['esc'])
        self.assertEqual(identities, [{'22': {'start_time': 44}}])
        self.assertIn('USER timeout 30 systemctl --user start realm-idle.service', commands)
        self.assertIn('USER timeout 30 systemctl --user stop realm-idle.service', commands)
        self.assertIn('id -u alice', commands)
        self.assertIn('sudo journalctl -b -n 200 --no-pager -o json SYSLOG_IDENTIFIER=realm-idle _UID=1001', commands)


if __name__ == '__main__':
    unittest.main()
