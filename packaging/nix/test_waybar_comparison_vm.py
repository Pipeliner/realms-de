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


if __name__ == "__main__":
    unittest.main()
