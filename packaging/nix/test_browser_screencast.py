"""Lightweight checks: incomplete browser evidence must never become success."""
import base64
import importlib.util
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request
from unittest import mock

HERE = pathlib.Path(__file__).parent
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aP1sAAAAASUVORK5CYII=")


class EvidenceTests(unittest.TestCase):
    def test_page_waits_for_advancing_callbacks_and_stops_on_timeout(self):
        completed = subprocess.run([
            os.environ.get("REALM_NODE", "node"),
            str(HERE / "test_browser_frames.js"),
            str(HERE / "browser_screencast.html"),
        ], capture_output=True, text=True, timeout=5)
        self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)

    def test_page_uses_browser_default_colors(self):
        page = (HERE / "browser_screencast.html").read_text()
        self.assertNotRegex(page, r"#[0-9a-fA-F]{6}")
        self.assertNotIn("background:", page)
        self.assertNotIn("color:", page)
        self.assertNotIn("style.background", page)
        self.assertIn('status.textContent = "Realm browser delivered frame "', page)

    def setUp(self):
        path = HERE / "browser_screencast.py"
        self.assertTrue(path.exists(), "browser evidence collector is missing")
        spec = importlib.util.spec_from_file_location("browser_screencast", path)
        self.collector = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.collector)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.output = pathlib.Path(self.tmp.name)

    def result(self):
        return {"frames": [{"mediaTime": 1, "width": 1, "height": 1},
                           {"mediaTime": 2, "width": 1, "height": 1}],
                "stopped": True, "trackStates": ["ended"], "userAgent": "Firefox/test"}

    def frames(self):
        for index in (0, 1):
            (self.output / f"frame-{index}.png").write_bytes(PNG)

    def test_accepts_two_frames_and_retains_digests(self):
        self.frames()
        result = self.collector.validate_result(self.output, self.result())
        self.assertEqual([frame["bytes"] for frame in result["frames"]], [68, 68])
        self.assertEqual(result["frames"][0]["sha256"], "8d9e32c009c7b4b82a3d9915625dafab582d58699f2bc727bd15e5c06e1c4fe1")

    def test_interrupted_publication_keeps_previous_complete_evidence(self):
        self.assertTrue(hasattr(self.collector, "publish_text"), "atomic publication missing")
        target = self.output / "result.json"
        target.write_text('{"previous": true}')
        original = pathlib.Path.write_text

        def interrupted(path, text):
            original(path, text[:3])
            raise OSError("interrupted write")

        with mock.patch.object(pathlib.Path, "write_text", interrupted):
            with self.assertRaises(OSError):
                self.collector.publish_text(target, '{"complete": true}')
        self.assertEqual(target.read_text(), '{"previous": true}')
        self.collector.publish_text(target, '{"complete": true}')
        self.assertEqual(target.read_text(), '{"complete": true}')

    def test_rejects_missing_or_invalid_frame(self):
        self.frames()
        for data in (b"", b"not a PNG"):
            (self.output / "frame-1.png").write_bytes(data)
            with self.assertRaises(ValueError):
                self.collector.validate_result(self.output, self.result())
        (self.output / "frame-1.png").unlink()
        with self.assertRaises(ValueError):
            self.collector.validate_result(self.output, self.result())

    def test_rejects_stationary_time_dimensions_and_live_tracks(self):
        self.frames()
        for mutation in ("time", "dimensions", "stopped", "live"):
            result = self.result()
            if mutation == "time": result["frames"][1]["mediaTime"] = 1
            if mutation == "dimensions": result["frames"][1]["width"] = 0
            if mutation == "stopped": result["stopped"] = False
            if mutation == "live": result["trackStates"] = ["live"]
            with self.assertRaises(ValueError, msg=mutation):
                self.collector.validate_result(self.output, result)

    def test_http_page_frames_and_result_are_retained(self):
        process = subprocess.Popen([
            sys.executable, str(HERE / "browser_screencast.py"),
            "--page", str(HERE / "browser_screencast.html"),
            "--output", str(self.output), "--port", "0",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            while not (self.output / "ready").exists():
                self.assertIsNone(process.poll(), "collector exited before listening")
                self.assertLess(time.monotonic(), deadline, "collector readiness timeout")
                time.sleep(0.01)
            origin = "http://127.0.0.1:" + (self.output / "ready").read_text()
            with urllib.request.urlopen(origin) as response:
                self.assertEqual(response.read(), (HERE / "browser_screencast.html").read_bytes())
            for index in (0, 1):
                with urllib.request.urlopen(origin + f"/frame-{index}.png", data=PNG) as response:
                    self.assertEqual(response.status, 204)
            with urllib.request.urlopen(origin + "/result", data=json.dumps(self.result()).encode()) as response:
                self.assertEqual(response.status, 204)
            self.assertTrue(json.loads((self.output / "result.json").read_text())["stopped"])
            self.assertEqual((self.output / "frame-1.png").read_bytes(), PNG)
            invalid = self.result()
            invalid["frames"][1]["mediaTime"] = 1
            with self.assertRaises(urllib.error.HTTPError):
                urllib.request.urlopen(origin + "/result", data=json.dumps(invalid).encode())
            with urllib.request.urlopen(origin + "/error", data=b'{"error":"collector /result: 400"}'):
                pass
            error = json.loads((self.output / "error.json").read_text())
            self.assertEqual(error["error"], "video time must advance")
            self.assertEqual(error["rejected"]["frames"], invalid["frames"])
        finally:
            process.terminate()
            process.communicate(timeout=5)


if __name__ == "__main__":
    unittest.main()
