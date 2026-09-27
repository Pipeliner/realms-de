#!/usr/bin/env python3
"""Exercise the shared global colour guard against tracked fixture files."""
from pathlib import Path
import subprocess
import tempfile
import unittest

GUARD = Path(__file__).resolve().parents[1] / "scripts/check-colour-literals"


class GlobalColourTests(unittest.TestCase):
    def check(self, files):
        with tempfile.TemporaryDirectory(prefix="realm-colour-guard-") as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", directory], check=True)
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            subprocess.run(["git", "add", "."], cwd=root, check=True)
            return subprocess.run(["bash", str(GUARD)], cwd=root,
                                  capture_output=True, text=True)

    def test_canonical_palette_and_existing_allowances(self):
        result = self.check({
            "palette.toml": 'background = "#123abc"\n',
            "docs/nested/example.md": "#ABCDEF\n",
            "design/example": "#abcdef\n",
            "README.md": "#abcdef\n",
            ".github/example": "#abcdef\n",
            ".claude/example": "#abcdef\n",
            "packaging/river/sources.toml": "#abcdef\n",
            "packaging/tool-sources/bundles/nested/example": "#abcdef\n",
            "example.rs": '#[cfg(test)]\nconst VALUE: &str = "#123abc";\n',
        })
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_rejects_templates_scripts_and_rust_before_test_tail(self):
        for name in ("configs/templates/example.ini", "scripts/example", "example.rs"):
            with self.subTest(name=name):
                result = self.check({name: 'safe\nvalue = "#123abc"\n#[cfg(test)]\n'})
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn(name + ':2:', result.stdout)
                self.assertIn("hex colour literal outside palette.toml", result.stdout)


if __name__ == "__main__":
    unittest.main()
