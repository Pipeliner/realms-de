"""Exercise native acceptance decisions without launching a VM."""
import importlib.util
from pathlib import Path
import tempfile
import socket
import threading
import time
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("lock_roundtrip", Path(__file__).with_name("lock_roundtrip.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class RoundtripTests(unittest.TestCase):
    def test_fresh_capture_rejects_old_complete_frame_before_monitor_command(self):
        self.assertTrue(hasattr(probe, 'fresh_screenshot'), 'capture must reject an old complete frame')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'frame.ppm'
            path.write_bytes(b'P6\n2 1\n255\n' + b'\0' * 6)
            with self.assertRaisesRegex(AssertionError, 'already exists'):
                probe.fresh_screenshot(path, lambda: self.fail('old frame admitted'))
            path.unlink()
            result = probe.fresh_screenshot(path, lambda: path.write_bytes(b'P6\n2 1\n255\n' + b'\0' * 6))
            self.assertEqual(result, path)
            path.unlink()
            with patch.object(probe.time, 'monotonic', side_effect=[0, 6]), self.assertRaisesRegex(AssertionError, 'incomplete'):
                probe.fresh_screenshot(path, lambda: path.write_bytes(b'P6\n2 1\n255\n' + b'\0' * 5))

    def test_real_unix_monitor_handshake_split_reply_and_eof(self):
        for premature_eof in (False, True):
            with self.subTest(premature_eof=premature_eof), tempfile.TemporaryDirectory() as directory:
                path = str(Path(directory) / 'monitor')
                failures = []
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                    listener.bind(path)
                    listener.listen(1)
                    listener.settimeout(2)
                    def server():
                        try:
                            with listener.accept()[0] as connection:
                                connection.settimeout(0.03)
                                # The client must consume the greeting before sending.
                                with self.assertRaises(socket.timeout):
                                    connection.recv(1)
                                connection.sendall(b'QEMU fixture\r\n(qe')
                                time.sleep(0.02)
                                connection.sendall(b'mu) ')
                                connection.settimeout(2)
                                received = b''
                                while not received.endswith(b'\n'):
                                    chunk = connection.recv(64)
                                    self.assertTrue(chunk)
                                    received += chunk
                                self.assertEqual(received, b'sendkey ret\n')
                                connection.sendall(b'sendkey ret\r\n')
                                if not premature_eof:
                                    connection.sendall(b'(qe')
                                    time.sleep(0.02)
                                    connection.sendall(b'mu) ')
                        except BaseException as error:
                            failures.append(error)
                    thread = threading.Thread(target=server, daemon=True)
                    thread.start()
                    try:
                        if premature_eof:
                            with self.assertRaisesRegex(RuntimeError, 'before command completion'):
                                probe.monitor_command(path, 'sendkey ret')
                        else:
                            self.assertIn('sendkey ret', probe.monitor_command(path, 'sendkey ret'))
                    finally:
                        thread.join(timeout=3)
                    self.assertFalse(thread.is_alive(), 'fake monitor did not terminate')
                    if failures:
                        raise failures[0]

    def run_fixture(self, wrong_unlock=False, duplicate_change=False,
                    correct_stays_locked=False, binding_leaks=False, survivor=False,
                    restoration_fails=False):
        state = {"active": False, "keys": [], "cycle": 0}
        def snapshot():
            return {"state": "active" if state["active"] else "inactive",
                    "main_pid": 0,
                    "processes": {"20": {"start_time": 17, "executable": "/usr/bin/swaylock"},
                                  "21": {"start_time": 18, "executable": "/usr/bin/swaylock"}}
                    if state["active"] else {}}
        def start():
            if state["active"] and duplicate_change:
                state["active"] = False
            else:
                state["active"] = True
        def key(value):
            state["keys"].append(value)
        def password(value):
            if (value == "realmtest" and not correct_stays_locked) or wrong_unlock:
                state["active"] = False
        def suppressed():
            assert not binding_leaks, "launcher appeared while locked"
        def gone(old):
            assert not survivor, "original locker identity survives"
        def restored(present):
            assert not restoration_fails, "launcher did not restore"
            self.assertEqual(state['keys'][-1], 'meta_l-d' if present else 'esc')
        with tempfile.TemporaryDirectory() as directory:
            results = probe.roundtrips(start, snapshot, key, password,
                suppressed, gone, lambda name: None,
                lambda seconds: None, Path(directory), restored)
        return results

    def test_two_cycles_keep_entire_pam_process_tree(self):
        results = self.run_fixture()
        self.assertEqual(len(results), 2)
        self.assertEqual(set(results[0]["processes"]), {"20", "21"})
        self.assertTrue(all(result["password_unlock"] for result in results))
        self.assertTrue(all(result["launcher_binding_restored"] for result in results))

    def test_hmp_waits_for_complete_prompt_and_rejects_eof(self):
        connection = Mock()
        connection.recv.side_effect = [b'reply\r\n(qe', b'mu) ']
        self.assertEqual(probe.read_prompt(connection, float('inf')), b'reply\r\n(qemu) ')
        connection.recv.side_effect = [b'partial', b'']
        with self.assertRaises(RuntimeError):
            probe.read_prompt(connection, float('inf'))
        with self.assertRaises(TimeoutError):
            probe.read_prompt(connection, 0)

    def test_hmp_consumes_greeting_then_post_command_prompt(self):
        connection = Mock()
        connection.recv.side_effect = [b'QEMU\r\n(qemu) ', b'sendkey ret\r\n', b'(qemu) ']
        context = Mock()
        context.__enter__ = Mock(return_value=connection)
        context.__exit__ = Mock(return_value=False)
        with patch.object(probe.socket, 'socket', return_value=context):
            probe.monitor_command('/fixture/monitor', 'sendkey ret')
        self.assertEqual(connection.recv.call_count, 3)
        connection.sendall.assert_called_once_with(b'sendkey ret\n')

    def test_hmp_command_error_is_failure(self):
        connection = Mock()
        connection.recv.side_effect = [b'(qemu) ', b'Error: invalid key\r\n(qemu) ']
        context = Mock()
        context.__enter__ = Mock(return_value=connection)
        context.__exit__ = Mock(return_value=False)
        with patch.object(probe.socket, 'socket', return_value=context), self.assertRaises(AssertionError):
            probe.monitor_command('/fixture/monitor', 'sendkey invalid')

    def test_screenshot_requires_complete_pixels_including_uniform_lock(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'frame.ppm'
            path.write_bytes(b'P6\n2 1\n255\n' + b'\0' * 6)
            self.assertTrue(probe.complete_ppm(path))
            path.write_bytes(b'P6\n2 1\n255\n' + b'\0' * 5)
            self.assertFalse(probe.complete_ppm(path))

    def test_wrong_password_unlock_is_failure(self):
        with self.assertRaises(AssertionError):
            self.run_fixture(wrong_unlock=True)

    def test_duplicate_start_changes_are_failure(self):
        with self.assertRaises(AssertionError):
            self.run_fixture(duplicate_change=True)

    def test_correct_password_must_unlock(self):
        with self.assertRaisesRegex(AssertionError, "correct password"):
            self.run_fixture(correct_stays_locked=True)

    def test_launcher_must_restore_after_unlock(self):
        with self.assertRaisesRegex(AssertionError, 'did not restore'):
            self.run_fixture(restoration_fails=True)

    def test_binding_or_surviving_process_failure_propagates(self):
        for option in ("binding_leaks", "survivor"):
            with self.subTest(option=option), self.assertRaises(AssertionError):
                self.run_fixture(**{option: True})

    def test_failure_retains_structured_results(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(AssertionError):
                probe.roundtrips(lambda: None,
                    lambda: {"state": "inactive", "main_pid": 0, "processes": {}},
                    lambda key: None, lambda password: None, lambda: None,
                    lambda old: None, lambda name: None, lambda seconds: None,
                    Path(directory), lambda present: None)
            self.assertEqual((Path(directory) / "lock-roundtrip.json").read_text(), "[]\n")


if __name__ == "__main__":
    unittest.main()
