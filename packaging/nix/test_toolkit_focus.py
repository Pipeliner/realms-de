"""Keep toolkit protocol-title focus distinct from visible OCR acceptance."""

import ast
from pathlib import Path
import shlex
import subprocess
import tempfile
import textwrap
import unittest


SOURCE = Path(__file__).with_name("checks.nix").read_text()


def toolkit_calls():
    """Parse the seven direct/launcher toolkit probes without evaluating Nix."""
    calls = []
    for block in SOURCE.split("      exercise_toolkit(")[1:]:
        invocation = "exercise_toolkit(" + block.split("\n      )", 1)[0] + "\n)"
        calls.append(ast.parse(textwrap.dedent(invocation)).body[0].value)
    return calls


class ToolkitFocusTests(unittest.TestCase):
    def test_css_diagnostic_logs_offending_line_and_required_open_before_rejection(self):
        self.assertTrue(
            "      def assert_toolkit_diagnostics_clean(" in SOURCE,
            "toolkit diagnostic gate must log matched stderr before rejection",
        )
        start = SOURCE.index("      def assert_toolkit_diagnostics_clean(")
        end = SOURCE.index("      def exercise_toolkit(", start)

        class LocalMachine:
            def __init__(self):
                self.logs = []

            def execute(self, command, timeout):
                completed = subprocess.run(
                    ["/bin/sh", "-c", command], text=True, capture_output=True,
                    timeout=timeout, check=False,
                )
                return completed.returncode, completed.stdout + completed.stderr

            def fail(self, command):
                status, output = self.execute(command, 5)
                if status == 0:
                    raise AssertionError("diagnostic grep unexpectedly succeeded")
                return output

            def log(self, message):
                self.logs.append(message)

        machine = LocalMachine()
        namespace = {"machine": machine, "shlex": shlex, "DIAGNOSTIC_TIMEOUT": 5}
        exec(compile(textwrap.dedent(SOURCE[start:end]), "<toolkit-diagnostics>", "exec"), namespace)
        check = namespace["assert_toolkit_diagnostics_clean"]
        pattern = r"(css|theme).*(error|failed|invalid|not found|unable|warning)"
        with tempfile.TemporaryDirectory() as directory:
            stderr = Path(directory) / "toolkit.stderr"
            trace = Path(directory) / "toolkit.trace"
            css = "/selected/gtk-3.0/gtk.css"
            stderr.write_text("Gtk-WARNING: theme CSS warning in selected palette\n")
            trace.write_text(f'openat(3, "{css}", O_RDONLY) = 7\n')
            with self.assertRaisesRegex(AssertionError, "diagnostic grep unexpectedly succeeded"):
                check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            output = "\n".join(machine.logs)
            self.assertIn("theme CSS warning in selected palette", output)
            self.assertIn(f'"{css}"', output)
            machine.logs.clear()
            stderr.write_text("Gtk-WARNING: theme CSS warning " + "x" * 20000 + "\n")
            trace.write_text(f'openat(3, "{css}", O_RDONLY) = 7 ' + "y" * 20000 + "\n")
            with self.assertRaisesRegex(AssertionError, "diagnostic grep unexpectedly succeeded"):
                check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            self.assertEqual(len(machine.logs), 2)
            self.assertLessEqual(len(machine.logs[0]), 8300)
            self.assertLessEqual(len(machine.logs[1]), 8300)
            machine.logs.clear()
            stderr.write_text("normal toolkit startup\n")
            check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            self.assertEqual(machine.logs, [])

    def test_source_regression_runs_in_nix_lightweight_checks(self):
        self.assertIn(
            '${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_toolkit_focus.py"}',
            SOURCE,
        )

    def test_protocol_title_is_an_explicit_expectation_at_every_probe(self):
        calls = toolkit_calls()
        self.assertEqual(len(calls), 7)
        observed = {
            call.args[1].value: (call.args[3].value, call.args[4].value)
            for call in calls
        }
        self.assertEqual(observed, {
            "gtk3-toolkit": ("gtk3-widget-factory", "togglebutton"),
            "gtk4-toolkit": ("GTK Widget Factory", "Page 1"),
            "qt6-toolkit": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
            "gtk3-launcher": ("gtk3-widget-factory", "togglebutton"),
            "gtk4-launcher": ("GTK Widget Factory", "Page 1"),
            "qt6-launcher": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
            "qt6-user-override": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
        })

    def test_gtk3_visual_anchor_is_in_widget_ocr_not_terminal_only(self):
        calls = toolkit_calls()
        gtk3_visual = [
            call.args[4].value for call in calls
            if call.args[1].value in ("gtk3-toolkit", "gtk3-launcher")
        ]
        # CI run 36354366061: the narrow tab was read as Pagel/Pace 1l,
        # while this GTK3 control label was recognized repeatedly.
        widget_ocr = "gtk3-widget-factory\nPagel\nPage2\ntogglebutton\ncheckbutton"
        terminal_only = (
            "foot\nalice@machine\n"
            "strace -f -qq -e trace=openat gtk3-widget-factory"
        )
        self.assertEqual(len(gtk3_visual), 2)
        for visual in gtk3_visual:
            self.assertIn(visual, widget_ocr)
            self.assertNotIn(visual, terminal_only)

    def test_focus_requires_exact_protocol_title_and_two_managed_windows(self):
        start = SOURCE.index("      def toolkit_focus_matches(")
        end = SOURCE.index("      def exercise_toolkit(", start)
        namespace = {}
        exec(compile(textwrap.dedent(SOURCE[start:end]), "<toolkit-focus>", "exec"), namespace)
        matches = namespace["toolkit_focus_matches"]
        response = {"data": {
            "orbits": [{"windows": 1}, {"windows": 1}],
            "focused_title": "gtk3-widget-factory",
        }}
        self.assertTrue(matches(response, "gtk3-widget-factory"))
        self.assertFalse(matches(response, "Widget Factory"))
        response["data"]["focused_title"] = "foot"
        self.assertFalse(matches(response, "gtk3-widget-factory"))
        response["data"]["focused_title"] = "gtk3-widget-factory"
        response["data"]["orbits"][1]["windows"] = 2
        self.assertFalse(matches(response, "gtk3-widget-factory"))

    def test_visual_and_consumer_checks_remain_after_focus(self):
        start = SOURCE.index("      def exercise_toolkit(")
        end = SOURCE.index("      gtk4_css =", start)
        block = SOURCE[start:end]
        self.assertIn("toolkit_focus_matches(response, expected_title)", block)
        self.assertIn("machine.wait_for_text(expected_text, timeout=OCR_TIMEOUT)", block)
        self.assertIn("machine.screenshot(screenshot)", block)
        self.assertIn("grep -F {quoted_match}", block)
        self.assertIn("assert machine.succeed(f\"cat {shlex.quote(done)}\").strip() == \"0\"", block)


if __name__ == "__main__":
    unittest.main()
