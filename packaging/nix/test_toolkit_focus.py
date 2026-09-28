"""Keep toolkit protocol-title focus distinct from visible OCR acceptance."""

import ast
from pathlib import Path
import shlex
import subprocess
import tempfile
import textwrap
import unittest


SOURCE = Path(__file__).with_name("checks.nix").read_text()
MODULE = Path(__file__).with_name("nixos-module.nix").read_text()


def toolkit_calls():
    """Parse the seven direct/launcher toolkit probes without evaluating Nix."""
    calls = []
    for block in SOURCE.split("      exercise_toolkit(")[1:]:
        invocation = "exercise_toolkit(" + block.split("\n      )", 1)[0] + "\n)"
        calls.append(ast.parse(textwrap.dedent(invocation)).body[0].value)
    return calls


class ToolkitFocusTests(unittest.TestCase):
    def test_home_manager_unit_delivers_qt6_plugin_and_preserves_path(self):
        module = Path(__file__).with_name("home-manager-module.nix").read_text()
        self.assertRegex(module, r"home\.packages\s*=\s*\[[^\]]*pkgs\.qt6Packages\.qt6ct")
        self.assertRegex(module, r'Environment\s*=\s*\[\s*"PATH=')
        self.assertIn('"QT_PLUGIN_PATH=${pkgs.qt6Packages.qt6ct}/${pkgs.qt6Packages.qtbase.qtPluginPrefix}"', module)

    def test_qt6_plugin_is_a_native_runtime_dependency(self):
        root = Path(__file__).resolve().parents[2]
        control = (root / "packaging/debian/control").read_text()
        depends = control.split("\nDepends:", 1)[1].split("\n#", 1)[0]
        self.assertIn("qt6ct", [part.strip() for part in depends.split(",")])
        spec = (root / "packaging/fedora/realm.spec").read_text()
        self.assertRegex(spec, r"(?m)^Requires:\s+qt6ct\s*$")

    def test_nix_module_delivers_discoverable_qt6_plugin_without_fixture_install(self):
        self.assertIn("pkgs.qt6Packages.qt6ct", MODULE)
        self.assertRegex(MODULE, r"realm-wm\.environment\.QT_PLUGIN_PATH\s*=\s*"
                         r'"\$\{pkgs.qt6Packages.qt6ct\}/\$\{pkgs.qt6Packages.qtbase.qtPluginPrefix\}";')
        self.assertNotIn("          pkgs.qt6Packages.qt6ct\n", SOURCE)
        self.assertIn('wm_environment["QT_PLUGIN_PATH"]', SOURCE)
        self.assertIn('zsh_environment["QT_PLUGIN_PATH"]', SOURCE)
        self.assertIn("/platformthemes/libqt6ct.so", SOURCE)

    def test_svg_loader_is_registered_and_scoped_to_realm_wm(self):
        self.assertIn("programs.gdk-pixbuf.modulePackages = [ pkgs.librsvg ];", MODULE)
        self.assertRegex(
            MODULE,
            r"systemd\.user\.services\.realm-wm\.environment\.GDK_PIXBUF_MODULE_FILE"
            r"\s*=\s*config\.environment\.sessionVariables\.GDK_PIXBUF_MODULE_FILE;",
        )

    def test_vm_requires_live_wm_and_terminal_svg_loader_cache(self):
        self.assertIn('wm_environment["GDK_PIXBUF_MODULE_FILE"]', SOURCE)
        self.assertIn('zsh_environment["GDK_PIXBUF_MODULE_FILE"]', SOURCE)

    def test_vm_accepts_pinned_librsvg_svg_entry_and_logs_bounded_absence(self):
        start = SOURCE.index('      svg_loader_cache = wm_environment["GDK_PIXBUF_MODULE_FILE"]')
        end = SOURCE.index('      # Check the user-manager publication', start)
        source = textwrap.dedent(SOURCE[start:end])

        class LocalMachine:
            def __init__(self):
                self.logs = []

            def execute(self, command, timeout):
                completed = subprocess.run(
                    ["/bin/sh", "-c", command], text=True, capture_output=True,
                    timeout=timeout, check=False,
                )
                return completed.returncode, completed.stdout + completed.stderr

            def succeed(self, command):
                status, output = self.execute(command, 5)
                if status != 0:
                    raise AssertionError(f"SVG cache command failed: {command}")
                return output

            def log(self, message):
                self.logs.append(message)

        machine = LocalMachine()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "loaders.cache"
            namespace = {
                "machine": machine, "wm_environment": {"GDK_PIXBUF_MODULE_FILE": str(path)},
                "shlex": shlex, "DIAGNOSTIC_TIMEOUT": 5,
            }
            path.write_text(
                '"/nix/store/librsvg/lib/gdk-pixbuf-2.0/loaders/libpixbufloader_svg.so"\n'
                '"svg" 2 "gdk-pixbuf" "Scalable Vector Graphics"\n'
            )
            exec(compile(source, "<svg-loader-cache>", "exec"), namespace)
            self.assertEqual(machine.logs, [])

            path.write_text(
                '"/nix/store/old/lib/gdk-pixbuf-2.0/loaders/libpixbufloader-svg.so"\n'
                + "x" * 12000
            )
            with self.assertRaisesRegex(AssertionError, "SVG cache command failed"):
                exec(compile(source, "<svg-loader-cache>", "exec"), namespace)
            self.assertEqual(len(machine.logs), 1)
            self.assertIn("libpixbufloader-svg.so", machine.logs[0])
            self.assertLessEqual(len(machine.logs[0]), 4300)
            machine.logs.clear()
            path.write_text('"/nix/store/incorrect/libpixbufloader_svg.so.backup"\n')
            with self.assertRaisesRegex(AssertionError, "SVG cache command failed"):
                exec(compile(source, "<svg-loader-cache>", "exec"), namespace)
            self.assertIn("libpixbufloader_svg.so.backup", machine.logs[0])

    def test_toolkit_failure_logs_bounded_related_trace_and_child_selectors(self):
        self.assertIn("relevant GTK openat", SOURCE)
        self.assertIn("GTK_THEME", SOURCE)
        self.assertIn("GDK_PIXBUF_MODULE_FILE", SOURCE)

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
            trace.write_text(
                f'openat(3, "{css}", O_RDONLY) = 7\n'
                'openat(3, "/nix/store/example-gdk-pixbuf/loaders.cache", O_RDONLY) = 8\n'
            )
            with self.assertRaisesRegex(AssertionError, "diagnostic grep unexpectedly succeeded"):
                check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            output = "\n".join(machine.logs)
            self.assertIn("theme CSS warning in selected palette", output)
            self.assertIn(f'"{css}"', output)
            self.assertIn("loaders.cache", output)
            machine.logs.clear()
            stderr.write_text("Gtk-WARNING: theme CSS warning " + "x" * 20000 + "\n")
            trace.write_text(
                f'openat(3, "{css}", O_RDONLY) = 7\n' + "y" * 20000 + "\n"
            )
            with self.assertRaisesRegex(AssertionError, "diagnostic grep unexpectedly succeeded"):
                check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            self.assertEqual(len(machine.logs), 4)
            self.assertLessEqual(len(machine.logs[0]), 8300)
            self.assertLessEqual(len(machine.logs[1]), 8300)
            self.assertLessEqual(len(machine.logs[2]), 8300)
            self.assertLessEqual(len(machine.logs[3]), 8300)
            machine.logs.clear()
            stderr.write_text("normal toolkit startup\n")
            check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            self.assertEqual(machine.logs, [])
            trace.write_text(
                'openat(3, "/other/theme/gtk.css", O_RDONLY) = 9\n'
                'openat(3, "/nix/store/example-gdk-pixbuf/loaders.cache", O_RDONLY) = 8\n'
            )
            check("gtk3-toolkit", str(stderr), str(trace), [css], pattern)
            missing_output = "\n".join(machine.logs)
            self.assertIn("<no matching openat line>", missing_output)
            self.assertIn("/other/theme/gtk.css", missing_output)
            self.assertIn("loaders.cache", missing_output)

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
