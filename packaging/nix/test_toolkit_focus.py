"""Keep toolkit protocol-title focus distinct from visible OCR acceptance."""

import ast
from pathlib import Path
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
            "gtk3-toolkit": ("gtk3-widget-factory", "Page 1"),
            "gtk4-toolkit": ("GTK Widget Factory", "Page 1"),
            "qt6-toolkit": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
            "gtk3-launcher": ("gtk3-widget-factory", "Page 1"),
            "gtk4-launcher": ("GTK Widget Factory", "Page 1"),
            "qt6-launcher": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
            "qt6-user-override": ("Qt6 Configuration Tool", "Qt6 Configuration Tool"),
        })

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
