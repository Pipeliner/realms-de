"""Source tests for the CI-only, event-fed Waybar candidate."""

import io
import json
import socket
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

import waybar_compare


def state(title="Terminal", revision=1):
    return {
        "revision": revision,
        "orbits": [
            {"number": n, "rune": rune, "display": display, "windows": windows}
            for n, rune, display, windows in [
                (1, "ᚠ", "active", 2), (2, "ᚢ", "occupied", 1),
                (3, "ᚦ", "empty", 0), (4, "ᚨ", "empty", 0),
                (5, "ᚱ", "empty", 0), (6, "ᚲ", "empty", 0),
            ]
        ],
        "layout": "triptych", "mode": "resize",
        "focused_title": title, "chord_echo": "mod+ ▸ awaiting chord",
        "whichkey": True, "grimoire": False, "modules": [],
    }


class FlushedOutput(io.StringIO):
    def __init__(self):
        super().__init__()
        self.flushes = 0

    def flush(self):
        self.flushes += 1
        super().flush()


def frame(value):
    return (json.dumps(value, ensure_ascii=False) + "\n").encode()


class RenderTests(unittest.TestCase):
    def test_six_orbits_and_realm_fields(self):
        # Catches dropped/renumbered cells and missing layout/mode/chord/title.
        self.assertEqual(waybar_compare.render_state(state()), {
            "text": "[1ᚠ] ●2ᚢ ·3ᚦ ·4ᚨ ·5ᚱ ·6ᚲ | TRIPTYCH | RESIZE | Terminal | mod+ ▸ awaiting chord",
            "tooltip": "Orbit 1: active (2 windows)\nOrbit 2: occupied (1 window)\nOrbit 3: empty (0 windows)\nOrbit 4: empty (0 windows)\nOrbit 5: empty (0 windows)\nOrbit 6: empty (0 windows)",
            "class": "realm",
        })

    def test_empty_and_markup_like_titles_are_literal_json_text(self):
        # Catches dropping an empty title or interpreting title text as markup.
        empty = waybar_compare.render_state(state(""))
        self.assertIn(" | untitled | ", empty["text"])
        marked = waybar_compare.render_state(state('<b>& "hi"</b>'))
        self.assertIn('<b>& "hi"</b>', json.loads(json.dumps(marked))["text"])
        trailing = state("pipe |")
        trailing["chord_echo"] = ""
        self.assertTrue(waybar_compare.render_state(trailing)["text"].endswith("pipe |"))

    def test_long_title_is_bounded_for_narrow_bar(self):
        # Catches unbounded user-controlled title growth.
        shown = waybar_compare.render_state(state("界" * 200))["text"]
        self.assertIn("界" * 31 + "…", shown)
        self.assertNotIn("界" * 32, shown)

    def test_css_uses_selected_palette_values(self):
        # Catches a copied fixture palette or missing login-palette CSS mapping.
        palette_path = Path(__file__).resolve().parents[2] / "palette.toml"
        palette = waybar_compare.tomllib.loads(palette_path.read_text(encoding="utf-8"))
        css = waybar_compare.render_css(palette)
        for value in (palette["background"]["bar_top"], palette["text"]["normal"],
                      palette["text"]["bright"], palette["accent"]["violet"],
                      palette["accent"]["starlight"], *palette["typography"]["fallback"],
                      str(palette["typography"]["size_meta"]) + "px",
                      str(palette["metrics"]["bar_height"]) + "px"):
            self.assertIn(value, css)
        palette["background"]["bar_top"] = palette["accent"]["gold"]
        self.assertIn(palette["accent"]["gold"], waybar_compare.render_css(palette))
        self.assertNotIn(palette["accent"]["gold"], css)
        self.assertNotIn("transition", css)
        self.assertNotIn("animation", css)


class ProtocolTests(unittest.TestCase):
    def exchange(self, frames, *, hello=None, expected_error=None):
        client, server = socket.socketpair()
        output = FlushedOutput()
        errors = []
        received = []

        def adapter():
            try:
                waybar_compare.run(client, output)
            except Exception as error:
                errors.append(error)
            finally:
                client.close()

        thread = threading.Thread(target=adapter, daemon=True)
        thread.start()
        server.settimeout(2)
        with server, server.makefile("rb") as reader:
            received.append(json.loads(reader.readline()))
            server.sendall(frame(hello or {"reply": "hello", "data": {"version": 2, "session": "test"}}))
            if expected_error != "hello":
                received.append(json.loads(reader.readline()))
                for item in frames:
                    server.sendall(item)
        thread.join(2)
        self.assertFalse(thread.is_alive(), "adapter remained blocked after fixture closed")
        return received, output, errors

    def test_initial_update_shutdown_and_no_polling(self):
        # Catches polling, missed State, and a missing flush on each changed line.
        first = state()
        same_display = state(revision=2)
        changed = state(title="Browser", revision=3)
        requests, output, errors = self.exchange([
            frame({"event": "state", "data": first}),
            frame({"event": "state", "data": same_display}),
            frame({"event": "state", "data": changed}),
            frame({"event": "shutdown"}),
        ])
        self.assertEqual(requests, [
            {"cmd": "hello", "arg": {"version": 2, "client": "realm-waybar-compare"}},
            {"cmd": "subscribe"},
        ])
        lines = output.getvalue().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertEqual(output.flushes, 2)
        self.assertEqual(json.loads(lines[0]), waybar_compare.render_state(first))
        self.assertIn("Browser", json.loads(lines[1])["text"])
        self.assertEqual(errors, [])

    def test_trace_records_receive_and_flush_for_changed_state(self):
        client, server = socket.socketpair()
        output = FlushedOutput()
        trace = io.StringIO()
        errors = []

        def adapter():
            try:
                waybar_compare.run(client, output, trace=trace)
            except Exception as error:
                errors.append(error)
            finally:
                client.close()

        thread = threading.Thread(target=adapter, daemon=True)
        thread.start()
        server.settimeout(2)
        with server, server.makefile("rb") as reader:
            self.assertEqual(json.loads(reader.readline())["cmd"], "hello")
            server.sendall(frame({"reply": "hello", "data": {"version": 2, "session": "test"}}))
            self.assertEqual(json.loads(reader.readline())["cmd"], "subscribe")
            server.sendall(frame({"event": "state", "data": state()}))
            server.sendall(frame({"event": "shutdown"}))
        thread.join(2)
        self.assertFalse(thread.is_alive())
        self.assertEqual(errors, [])
        records = [json.loads(line) for line in trace.getvalue().splitlines()]
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["revision"], 1)
        self.assertGreaterEqual(records[0]["flushed_monotonic_ns"], records[0]["received_monotonic_ns"])
        self.assertEqual(records[0]["text"], json.loads(output.getvalue())["text"])

    def test_version_mismatch_fails_before_subscribe(self):
        requests, output, errors = self.exchange([], hello={
            "reply": "hello", "data": {"version": 3, "session": "test"}
        }, expected_error="hello")
        self.assertEqual(len(requests), 1)
        self.assertEqual(output.getvalue(), "")
        self.assertEqual(len(errors), 1)

    def test_malformed_frame_and_eof_fail(self):
        for frames in ([b"{bad json}\n"], []):
            with self.subTest(frames=frames):
                _, _, errors = self.exchange(frames)
                self.assertEqual(len(errors), 1)

    def test_oversize_frame_fails(self):
        _, _, errors = self.exchange([b" " * 65536 + b"\n"])
        self.assertEqual(len(errors), 1)

    def test_cli_reports_malformed_stream_and_exits_nonzero(self):
        # Catches silently successful exits after a broken subscribed stream.
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "control.sock"
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
                listener.bind(str(path))
                listener.listen(1)
                listener.settimeout(2)
                child = subprocess.Popen(
                    [sys.executable, str(Path(waybar_compare.__file__)), "--socket", str(path)],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                try:
                    peer, _ = listener.accept()
                except socket.timeout:
                    child.terminate()
                    stdout, stderr = child.communicate(timeout=2)
                    self.fail(f"adapter never connected: exit={child.returncode}, stdout={stdout!r}, stderr={stderr!r}")
                peer.settimeout(2)
                with peer, peer.makefile("rb") as reader:
                    self.assertEqual(json.loads(reader.readline())["cmd"], "hello")
                    peer.sendall(frame({"reply": "hello", "data": {"version": 2, "session": "test"}}))
                    self.assertEqual(json.loads(reader.readline()), {"cmd": "subscribe"})
                    peer.sendall(b"{bad json}\n")
                stdout, stderr = child.communicate(timeout=2)
        self.assertEqual(child.returncode, 1)
        self.assertEqual(stdout, b"")
        self.assertIn(b"waybar compare: malformed Realm frame", stderr)


if __name__ == "__main__":
    unittest.main()
