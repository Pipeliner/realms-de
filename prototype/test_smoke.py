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
