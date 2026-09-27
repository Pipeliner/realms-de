#!/usr/bin/env python3
"""Source-only helper execution; no packages, bundles, or compilation."""
from pathlib import Path
import subprocess
import tempfile
import unittest


class FixtureRouting(unittest.TestCase):
    def test_toolchain_helper_preserves_repository_root_across_cases(self):
        source = Path(__file__).with_name("test-native-builds.sh").read_text()
        function = source.split("make_versioned_toolchain_root() {", 1)[1].split("\n}", 1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            script = "set -eu\nmake_versioned_toolchain_root() {" + function + "\n}\n"
            script += 'root=/repository-authority\nmake_versioned_toolchain_root "$1/one" /fake/cargo /fake/rustc\n'
            script += '[ "$root" = /repository-authority ]\nmake_versioned_toolchain_root "$1/two" /fake/cargo /fake/rustc\n'
            script += '[ "$root" = /repository-authority ]\n'
            result = subprocess.run(["sh", "-c", script, "fixture", temporary], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
