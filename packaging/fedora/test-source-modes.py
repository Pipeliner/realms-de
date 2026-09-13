"""Rust attributes in debug sources must not become executable shebangs."""
from pathlib import Path
import subprocess
import tempfile
import unittest


class SourceModes(unittest.TestCase):
    def test_rust_sources_are_data_but_scripts_stay_executable(self):
        helper = Path(__file__).with_name("normalize-source-modes.sh")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "vendor with spaces" / "line.rs"
            source.parent.mkdir()
            content = b"#![deny(missing_docs)]\npub struct Line;\n"
            source.write_bytes(content)
            source.chmod(0o755)
            script = root / "build-helper"
            script.write_text("#!/bin/sh\nexit 0\n")
            script.chmod(0o755)
            subprocess.run(["sh", str(helper), str(root)], check=True)
            self.assertEqual(source.stat().st_mode & 0o777, 0o644)
            self.assertEqual(source.read_bytes(), content)
            self.assertEqual(script.stat().st_mode & 0o777, 0o755)


if __name__ == "__main__":
    unittest.main()
