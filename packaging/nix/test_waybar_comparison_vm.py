"""Source checks for the explicit Waybar VM probe."""

import json
import tempfile
import unittest
from pathlib import Path

import waybar_comparison_vm as probe


class SelectionTests(unittest.TestCase):
    def test_palette_comes_from_login_record_generation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            runtime = root / "runtime"
            record = runtime / "realm" / "session-theme.json"
            record.parent.mkdir(parents=True)
            record.write_text(json.dumps({"config_root": str(root / "config"), "generation": "A"}))
            expected = root / "config" / "realm" / "generated" / "generations" / "A" / "realm" / "palette.toml"
            expected.parent.mkdir(parents=True)
            expected.write_text("selected")
            current = root / "config" / "realm" / "generated" / "current"
            current.write_text("B")
            self.assertEqual(probe.selected_palette_path(runtime), expected)

    def test_missing_selected_palette_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            record = root / "realm" / "session-theme.json"
            record.parent.mkdir(parents=True)
            record.write_text(json.dumps({"config_root": str(root / "config"), "generation": "A"}))
            with self.assertRaises(FileNotFoundError):
                probe.selected_palette_path(root)


class MeasurementTests(unittest.TestCase):
    def test_cpu_delta_preserves_raw_samples_and_rss_caveat(self):
        before = {"realm-bar": {"pid": 10, "ticks": 120, "rss_kib": 80},
                  "waybar": {"pid": 20, "ticks": 50, "rss_kib": 100}}
        after = {"realm-bar": {"pid": 10, "ticks": 150, "rss_kib": 84},
                 "waybar": {"pid": 20, "ticks": 60, "rss_kib": 110}}
        result = probe.cpu_delta(before, after, 100)
        self.assertEqual(result["cpu_seconds"], {"realm-bar": 0.3, "waybar": 0.1})
        self.assertEqual(result["start"], before)
        self.assertEqual(result["end"], after)
        self.assertIn("double", result["rss_caveat"])

    def test_idle_span_reports_observed_time_not_nominal_sleep(self):
        self.assertEqual(probe.observed_idle_span([10_000_000_000, 11_200_000_000,
                                                   12_500_000_000]), {
            "elapsed_seconds": 2.5,
            "interval_seconds": [1.2, 1.3],
        })

    def test_fullscreen_wait_rejects_stale_ledger_before_capture(self):
        seen = []

        def wait(predicate, label):
            stale = {"ledger": [{"active": True, "fullscreen": None}]}
            changed = {"ledger": [{"active": True, "fullscreen": 7}]}
            seen.extend([predicate(stale), predicate(changed)])
            return changed

        self.assertEqual(probe.wait_fullscreen(wait, True)["ledger"][0]["fullscreen"], 7)
        self.assertEqual(seen, [False, True])


class RecoveryTests(unittest.TestCase):
    def test_collected_transient_units_do_not_fail_cleanup(self):
        class Machine:
            def __init__(self):
                self.stopped = []

            def execute(self, command, timeout=None):
                if "is-active" in command:
                    return (3, "inactive") if "-1." in command else (0, "active")
                self.stopped.append(command)
                return 0, ""

        machine = Machine()
        result = probe.stop_candidate_units(machine, ["realm-waybar-comparison-1",
                                                      "realm-waybar-comparison-2"])
        self.assertEqual(result["stop_status"], 0)
        self.assertEqual(len(machine.stopped), 1)
        self.assertIn("-2.service", machine.stopped[0])

    def test_recovery_requires_toggle_and_different_focused_window(self):
        def snapshot(whichkey, focus):
            return {"state": {"whichkey": whichkey, "focused_title": focus},
                    "ledger": [{"active": True, "windows": [
                        {"id": 1, "focused": focus == "A"},
                        {"id": 2, "focused": focus == "B"},
                    ]}]}

        before = snapshot(True, "A")
        self.assertFalse(probe.help_toggled(before, snapshot(True, "A")))
        self.assertTrue(probe.help_toggled(before, snapshot(False, "A")))
        self.assertFalse(probe.focus_changed(before, snapshot(True, "A")))
        self.assertTrue(probe.focus_changed(before, snapshot(True, "B")))


class WiringTests(unittest.TestCase):
    def test_ocr_probe_does_not_claim_candidate_first_frame(self):
        source = Path(probe.__file__).read_text()
        self.assertNotIn("first_matching_ocr_screenshot", source)
        self.assertNotIn('"upper_bound_seconds"', source)
        self.assertIn('"candidate_region_verified": False', source)

    def test_terminal_stays_interactive_for_later_title_probe(self):
        source = Path(probe.__file__).read_text()
        self.assertNotIn("exec sleep infinity", source)

    def test_failed_vm_artifacts_use_explicit_driver_output(self):
        workflow = Path(probe.__file__).resolve().parents[2] / ".github/workflows/distro.yml"
        source = workflow.read_text()
        section = source.split("- name: Run explicit Waybar comparison VM", 1)[1]
        self.assertIn("waybar-comparison-vm.driver", section)
        self.assertIn("--output_directory", section)
        self.assertNotIn("cp -RL result-waybar-comparison", section)


if __name__ == "__main__":
    unittest.main()
