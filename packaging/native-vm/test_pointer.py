"""Real bounded QMP transport and shared native chooser input regression."""
import json
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
from unittest.mock import Mock

from pointer import qmp_command, position_pointer, click_pointer


class PointerTests(unittest.TestCase):
    def test_active_absolute_pointer_and_real_button_events(self):
        send = Mock(side_effect=[
            [{'index': 2, 'current': False, 'absolute': False},
             {'index': 7, 'current': True, 'absolute': True}], {}, {}, {}])
        evidence = {}
        position_pointer('socket', evidence, send)
        click_pointer('socket', evidence, send)
        self.assertEqual(send.call_args_list[1].args[2]['events'], [
            {'type': 'abs', 'data': {'axis': 'x', 'value': 16384}},
            {'type': 'abs', 'data': {'axis': 'y', 'value': 16384}}])
        self.assertEqual([call.args[2]['events'][0]['data']['down']
                          for call in send.call_args_list[2:]], [True, False])
        self.assertEqual(evidence['pointer']['index'], 7)

    def test_relative_or_missing_active_pointer_fails_before_input(self):
        for mice in ([], [{'current': True, 'absolute': False}]):
            send = Mock(return_value=mice)
            with self.assertRaises(AssertionError):
                position_pointer('socket', {}, send)
            self.assertEqual(send.call_count, 1)

    def exchange(self, mode):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'qmp')
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(path)
                listener.listen()
                def server():
                    with listener.accept()[0] as peer:
                        if mode == 'silent':
                            time.sleep(0.15)
                            return
                        peer.sendall(b'{"QMP":')
                        peer.sendall(b'{}}\r\n')
                        stream = peer.makefile('rb')
                        self.assertEqual(json.loads(stream.readline()),
                                         {'execute': 'qmp_capabilities', 'id': 'capabilities'})
                        peer.sendall(b'{"return":{},"id":"capabilities"}\r\n')
                        self.assertEqual(json.loads(stream.readline()),
                                         {'execute': 'query-mice', 'id': 'input'})
                        if mode == 'eof':
                            return
                        payload = {'error': {'desc': 'denied'}} if mode == 'error' else {'return': [7]}
                        payload['id'] = 'wrong' if mode == 'wrong-id' else 'input'
                        peer.sendall(b'{"event":"RESET"}\r\n' + json.dumps(payload).encode() + b'\r\n')
                thread = threading.Thread(target=server)
                thread.start()
                try:
                    return qmp_command(path, 'query-mice', timeout=0.05)
                finally:
                    thread.join(timeout=1)
                    self.assertFalse(thread.is_alive())

    def test_real_qmp_handshake_and_event_filtering(self):
        self.assertEqual(self.exchange('success'), [7])

    def test_real_qmp_errors_eof_and_silent_peer_are_bounded(self):
        for mode, error in [('error', RuntimeError), ('wrong-id', RuntimeError),
                            ('eof', RuntimeError), ('silent', TimeoutError)]:
            with self.subTest(mode=mode), self.assertRaises(error):
                self.exchange(mode)

    def test_both_consumers_and_vm_use_shared_qmp_socket(self):
        root = Path(__file__).parent
        for name in ('portal_roundtrip.py', 'browser_roundtrip.py'):
            source = (root / name).read_text()
            self.assertIn('position_pointer(monitor + ".qmp", result)', source)
            self.assertIn('click_pointer(monitor + ".qmp", result)', source)
            self.assertNotIn('mouse_set', source)
        self.assertIn('-qmp "unix:${monitor}.qmp,server=on,wait=off"',
                      (root / 'run-native-session-vm.sh').read_text())


if __name__ == '__main__':
    unittest.main()
