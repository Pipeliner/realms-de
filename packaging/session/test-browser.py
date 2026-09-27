"""Exercise browser dispatch without changing the host's default application."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


HELPER = Path(__file__).with_name("realm-browser")


class BrowserLaunch(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.calls = self.root / "calls"
        self.env = dict(os.environ, PATH=str(self.root), CALLS=str(self.calls),
                        DEFAULT="example.desktop", QUERY_EXIT="0", LAUNCH_EXIT="0")
        for name, body in {
            "xdg-settings": 'printf "query:%s:%s\\n" "$1" "$2" >> "$CALLS"\nprintf "%s\\n" "$DEFAULT"\nexit "$QUERY_EXIT"\n',
            "gtk-launch": 'printf "launch:%s:%s\\n" "$#" "$1" >> "$CALLS"\nexit "$LAUNCH_EXIT"\n',
        }.items():
            path = self.root / name
            path.write_text("#!/bin/sh\n" + body)
            path.chmod(0o755)

    def run_helper(self, *args):
        self.assertTrue(HELPER.is_file(), "shipped realm-browser helper is missing")
        return subprocess.run(["/bin/sh", str(HELPER), *args], env=self.env,
                              capture_output=True, text=True, timeout=3)

    def test_launches_exact_default_without_url(self):
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text(),
                         "query:get:default-web-browser\nlaunch:1:example.desktop\n")
        self.assertEqual(result.stdout, "")

    def test_rejects_absent_or_malformed_default_without_launch(self):
        for value in ["", "-bad.desktop", "../bad.desktop", "two words.desktop",
                      "a.desktop\nb.desktop", "browser", "a\tb.desktop"]:
            with self.subTest(value=value):
                self.env["DEFAULT"] = value
                self.calls.unlink(missing_ok=True)
                result = self.run_helper()
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue(result.stderr)
                self.assertNotIn("launch:", self.calls.read_text())

    def test_query_failure_does_not_launch(self):
        self.env["QUERY_EXIT"] = "4"
        result = self.run_helper()
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(result.stderr)
        self.assertNotIn("launch:", self.calls.read_text())

    def test_launch_failure_is_not_success(self):
        self.env["LAUNCH_EXIT"] = "7"
        self.assertNotEqual(self.run_helper().returncode, 0)

    def test_arguments_are_rejected_before_query(self):
        self.assertNotEqual(self.run_helper("https://example.invalid").returncode, 0)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
