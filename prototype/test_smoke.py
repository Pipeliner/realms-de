"""Lightweight regressions for interpreting real compositor evidence."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

spec = importlib.util.spec_from_file_location("ci_smoke", Path(__file__).with_name("ci-smoke.py"))
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class SmokeTests(unittest.TestCase):
    def test_application_process_match_uses_exact_comm_not_arguments(self):
        self.assertTrue(callable(getattr(smoke, "process_ids", None)),
                        "exact application process evidence is missing")
        processes = ("PID PPID COMMAND COMMAND\n"
                     "10 1 sway sway --debug\n"
                     "20 10 fuzzel fuzzel\n"
                     "30 10 foot foot -T fuzzel\n"
                     "40 10 thunar thunar\n")
        self.assertEqual(smoke.process_ids(processes, "fuzzel"), {20})
        self.assertEqual(smoke.process_ids(processes, "thunar"), {40})

    def test_application_window_match_requires_process_pid_and_mapped_window(self):
        self.assertTrue(callable(getattr(smoke, "process_windows", None)),
                        "mapped application PID evidence is missing")
        tree = {"pid": 40, "nodes": [
            {"id": 5, "pid": 30, "app_id": "foot", "name": "Thunar"},
            {"id": 6, "pid": 40, "app_id": "thunar", "name": "Home"},
        ], "floating_nodes": [
            {"id": 7, "pid": 40, "window": 100, "window_properties": {"class": "Thunar"}},
        ]}
        self.assertEqual([node["id"] for node in smoke.process_windows(tree, {40})], [6, 7])

    def shutdown(self, client_error, compositor_status):
        self.assertTrue(callable(getattr(smoke, "shutdown", None)), "shutdown contract is missing")
        session = subprocess.Popen([sys.executable, "-c",
                                    f"import time,sys; time.sleep(.05); sys.exit({compositor_status})"])
        def run(argv, check=True):
            self.assertEqual(argv, ["swaymsg", "-r", "-t", "command", "exit"])
            return subprocess.run([sys.executable, "-c",
                                   "import sys; print(sys.argv[1], file=sys.stderr); sys.exit(1)",
                                   client_error], capture_output=True, text=True)
        try:
            return smoke.shutdown(session, run)
        finally:
            if session.poll() is None:
                session.terminate()
            session.wait()

    def test_exit_accepts_closed_ipc_only_when_compositor_exits_zero(self):
        self.assertEqual(self.shutdown("Unable to receive IPC response", 0), 0)

    def test_closed_ipc_does_not_hide_compositor_failure(self):
        with self.assertRaises(RuntimeError):
            self.shutdown("Unable to receive IPC response", 3)

    def test_other_ipc_failure_is_rejected_even_if_compositor_exits_zero(self):
        with self.assertRaises(RuntimeError):
            self.shutdown("Unable to connect to IPC socket", 0)

    def test_workspace_count_excludes_foot_on_other_workspaces(self):
        self.assertTrue(callable(getattr(smoke, "workspace_terminals", None)),
                        "workspace-scoped window evidence is missing")
        tree = {"nodes": [
            {"type": "workspace", "num": 1, "nodes": [{"id": 5, "app_id": "foot"}]},
            {"type": "workspace", "num": 2, "nodes": [{"id": 6, "app_id": "foot"}]},
        ]}
        self.assertEqual([node["id"] for node in smoke.workspace_terminals(tree, 1)], [5])
        tree["nodes"][0]["floating_nodes"] = [{"id": 7, "app_id": "foot"}]
        self.assertEqual([node["id"] for node in smoke.workspace_terminals(tree, 1)], [5, 7])


if __name__ == "__main__":
    unittest.main()
