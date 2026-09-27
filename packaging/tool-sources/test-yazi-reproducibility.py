#!/usr/bin/env python3
"""Lightweight command-routing tests; never builds or extracts packages."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HELPER = Path(__file__).with_name("check-yazi-reproducibility.sh")


class Reproducibility(unittest.TestCase):
    def exercise(self, different=False, preexisting=False):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first, second = root / "first", root / "second"
            target = first / "debian/yazi-target/release"
            target.mkdir(parents=True)
            second.mkdir()
            for name in ("yazi", "ya"):
                (target / name).write_text(name)
            if preexisting:
                (second / "debian/yazi-target").mkdir(parents=True)
            bindir = root / "bin"
            bindir.mkdir()
            make = bindir / "make"
            make.write_text(
                "#!/bin/sh\nset -eu\n"
                '[ "$1" = -f ] && [ "$2" = debian/rules ] && [ "$3" = realm-build-yazi ]\n'
                'mkdir -p debian/yazi-target/release\n'
                'printf yazi > debian/yazi-target/release/yazi\n'
                + ('printf different' if different else 'printf ya')
                + ' > debian/yazi-target/release/ya\n'
            )
            make.chmod(0o755)
            evidence = root / "evidence.txt"
            result = subprocess.run(
                ["sh", str(HELPER), str(first), str(second), str(evidence)],
                env={**os.environ, "PATH": f"{bindir}:{os.environ['PATH']}"},
                capture_output=True, text=True,
            )
            return result, evidence.read_text() if evidence.exists() else ""

    def test_runs_production_target_and_compares_both_binaries(self):
        result, evidence = self.exercise()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("result=identical", evidence)
        self.assertIn("/release/ya", evidence)

    def test_changed_cli_binary_fails(self):
        result, evidence = self.exercise(different=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("result=different:ya", evidence)

    def test_rejects_reused_target(self):
        result, _ = self.exercise(preexisting=True)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
