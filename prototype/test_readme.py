"""Check the active prototype's dated evidence, not the historical desktop."""

import hashlib
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ReadmeEvidenceTests(unittest.TestCase):
    def test_active_readme_matches_retained_real_capture(self):
        provenance = json.loads((ROOT / "docs/assets/sway-prototype-proof.json").read_text())
        active = (ROOT / "README.md").read_text().split("## Historical Rust/River experiment")[0]
        self.assertIn(provenance["revision"], active)
        self.assertIn(str(provenance["run_id"]), active)
        self.assertIn("docs/assets/sway-prototype.png", active)
        self.assertNotIn("Runtime verification is pending", active)
        screenshot = (ROOT / "docs/assets/sway-prototype.png").read_bytes()
        self.assertEqual(screenshot[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual([int.from_bytes(screenshot[16:20], "big"),
                          int.from_bytes(screenshot[20:24], "big")], [1280, 800])
        self.assertEqual(hashlib.sha256(screenshot).hexdigest(), provenance["screenshot_sha256"])

    def test_active_download_matches_latest_verified_delivery(self):
        provenance = json.loads((ROOT / "docs/assets/sway-prototype-proof.json").read_text())
        delivery = provenance["latest_delivery"]
        active = (ROOT / "README.md").read_text().split("## Historical Rust/River experiment")[0]
        for field in ("revision", "run_id", "bundle_artifact_id", "verified_at", "expires_at"):
            self.assertIn(str(delivery[field]), active)


if __name__ == "__main__":
    unittest.main()
