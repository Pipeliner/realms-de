#!/usr/bin/env python3
"""Real Unix-socket roundtrip tests for the native VM control probe."""

import json
import socket
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PROBE = ROOT / "packaging/native-vm/control_get_state.py"


class ControlProbeTests(unittest.TestCase):
    def test_probe_sends_v2_hello_then_get_state_and_retains_frames(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            socket_path = root / "ctl.sock"
            output = root / "control.ndjson"
            received = []

            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(str(socket_path))
            listener.listen(1)

            def serve():
                connection, _ = listener.accept()
                with connection, connection.makefile("rwb", buffering=0) as stream:
                    received.append(json.loads(stream.readline()))
                    stream.write(
                        b'{"reply":"hello","data":{"version":2,"session":"fixture"}}\n'
                    )
                    received.append(json.loads(stream.readline()))
                    stream.write(b'{"reply":"state","data":{"orbits":[]}}\n')

            server = threading.Thread(target=serve)
            server.start()
            completed = subprocess.run(
                ["python3", str(PROBE), str(socket_path), str(output)],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                timeout=3,
            )
            server.join(timeout=1)
            listener.close()

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(
                received,
                [
                    {
                        "cmd": "hello",
                        "arg": {"version": 2, "client": "realm-native-vm"},
                    },
                    {"cmd": "get-state"},
                ],
            )
            self.assertEqual(len(output.read_text(encoding="utf-8").splitlines()), 2)


if __name__ == "__main__":
    unittest.main()
