#!/usr/bin/env python3
"""Launcher boundary tests; real compositor behavior is verified in CI."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


PROTOTYPE = Path(__file__).resolve().parent
REQUIRED = (
    "sway", "swaymsg", "swaybar", "foot", "fuzzel", "thunar",
    "i3status", "swayidle", "swaylock", "xdg-open",
)


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue((PROTOTYPE / "realm-prototype").is_file(),
                        "the relocatable launcher is missing")
        self.assertTrue((PROTOTYPE / "sway.conf").is_file(),
                        "the adjacent Sway configuration is missing")
        self.temp = tempfile.TemporaryDirectory(prefix="realm prototype ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bundle = self.root / "relocated desktop"
        self.bundle.mkdir()
        for filename in ("realm-prototype", "sway.conf"):
            shutil.copy2(PROTOTYPE / filename, self.bundle / filename)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.capture = self.root / "sway invocation.json"
        self.env = dict(os.environ, PATH=str(self.bin),
                        REALM_TEST_CAPTURE=str(self.capture),
                        DBUS_SESSION_BUS_ADDRESS="unix:path=/test-session-bus",
                        XDG_CURRENT_DESKTOP="old-desktop",
                        XDG_SESSION_DESKTOP="old-desktop",
                        XDG_SESSION_TYPE="x11")
        self.env.pop("DISPLAY", None)
        self.env.pop("WAYLAND_DISPLAY", None)
        # Only the external compositor is substituted; the launcher runs unchanged.
        sway = self.bin / "sway"
        sway.write_text(
            f"#!{shutil.which('python3')}\n"
            "import json, os, sys\n"
            "with open(os.environ['REALM_TEST_CAPTURE'], 'w') as capture:\n"
            "    json.dump({'args': sys.argv[1:], 'desktop': "
            "os.environ.get('XDG_CURRENT_DESKTOP'), 'session': "
            "os.environ.get('XDG_SESSION_DESKTOP'), 'type': "
            "os.environ.get('XDG_SESSION_TYPE')}, capture)\n"
            "sys.exit(int(os.environ.get('REALM_TEST_SWAY_EXIT', '0')))\n"
        )
        sway.chmod(0o755)
        for command in REQUIRED[1:]:
            (self.bin / command).symlink_to("/bin/true")

    def launch(self, *args):
        return subprocess.run([str(self.bundle / "realm-prototype"), *args],
                              env=self.env, cwd=self.root, text=True,
                              capture_output=True, timeout=5)

    def test_relocated_launch_uses_adjacent_config_and_preserves_arguments(self):
        result = self.launch("--debug", "argument with spaces", "$(not-a-command)")
        self.assertEqual(result.returncode, 0, result.stderr)
        captured = json.loads(self.capture.read_text())
        self.assertEqual(captured["args"], [
            "--config", str(self.bundle / "sway.conf"),
            "--debug", "argument with spaces", "$(not-a-command)",
        ])
        self.assertEqual(captured["desktop"], "sway")
        self.assertEqual(captured["session"], "sway")
        self.assertEqual(captured["type"], "wayland")

    def test_missing_dependencies_are_named_before_compositor_starts(self):
        for command in REQUIRED:
            with self.subTest(command=command):
                dependency = self.bin / command
                absent = self.bin / f"absent-{command}"
                dependency.rename(absent)
                try:
                    result = self.launch()
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(command, result.stderr)
                    self.assertIn("install", result.stderr.lower())
                    self.assertFalse(self.capture.exists())
                finally:
                    absent.rename(dependency)

    def test_missing_adjacent_config_is_reported(self):
        (self.bundle / "sway.conf").unlink()
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("sway.conf", result.stderr)
        self.assertFalse(self.capture.exists())

    def test_brightnessctl_absence_warns_but_does_not_block(self):
        result = self.launch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("brightnessctl", result.stderr)
        self.assertIn("dim", result.stderr.lower())

    def test_compositor_exit_status_is_preserved(self):
        self.env["REALM_TEST_SWAY_EXIT"] = "23"
        self.assertEqual(self.launch().returncode, 23)

    def test_inherited_desktop_display_is_rejected_before_startup(self):
        for variable, value in (("DISPLAY", ":0"),
                                ("WAYLAND_DISPLAY", "wayland-0")):
            with self.subTest(variable=variable):
                self.env[variable] = value
                try:
                    result = self.launch()
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("TTY", result.stderr)
                    self.assertFalse(self.capture.exists())
                finally:
                    self.env.pop(variable)

    def test_without_session_bus_requires_dbus_run_session(self):
        self.env.pop("DBUS_SESSION_BUS_ADDRESS")
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dbus-run-session", result.stderr)
        self.assertFalse(self.capture.exists())

    def test_without_session_bus_starts_sway_inside_dbus_session(self):
        self.env.pop("DBUS_SESSION_BUS_ADDRESS")
        wrapper = self.bin / "dbus-run-session"
        wrapper.write_text(
            "#!/bin/sh\n"
            "test \"$1\" = -- || exit 91\n"
            "shift\n"
            "export DBUS_SESSION_BUS_ADDRESS=unix:path=/new-session-bus\n"
            "exec \"$@\"\n"
        )
        wrapper.chmod(0o755)
        result = self.launch("--debug", "argument with spaces")
        self.assertEqual(result.returncode, 0, result.stderr)
        captured = json.loads(self.capture.read_text())
        self.assertEqual(captured["args"], [
            "--config", str(self.bundle / "sway.conf"),
            "--debug", "argument with spaces",
        ])


if __name__ == "__main__":
    unittest.main()
