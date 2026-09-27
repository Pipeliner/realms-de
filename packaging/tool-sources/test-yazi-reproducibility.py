#!/usr/bin/env python3
"""Lightweight command-routing tests; never builds or extracts packages."""
import os
import fnmatch
from pathlib import Path
import subprocess
import tempfile
import unittest

HELPER = Path(__file__).with_name("check-yazi-reproducibility.sh")


class Reproducibility(unittest.TestCase):
    def test_direct_rules_export_distro_flags_and_preserve_overrides(self):
        flags = ("CFLAGS", "CPPFLAGS", "CXXFLAGS", "LDFLAGS", "RUSTFLAGS")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            bindir = root / "usr/lib/rust-1.90/bin"
            bindir.mkdir(parents=True)
            for name in ("cargo", "rustc"):
                (bindir / name).symlink_to("/bin/true")
            provider = bindir / "dpkg-buildflags"
            provider.write_text('#!/bin/sh\nprintf "fixture-%s\\n" "$2"\n')
            provider.chmod(0o755)
            rules = HELPER.parents[1] / "debian/rules"
            for overrides in ({}, {flag: "caller-" + flag for flag in flags}):
                environment = {key: value for key, value in os.environ.items() if key not in flags}
                environment.update(overrides)
                environment["PATH"] = str(bindir) + ":" + os.environ["PATH"]
                result = subprocess.run(
                    ["make", "--no-print-directory", "-f", str(rules),
                     "REALM_RUST_VERSIONED_ROOT=" + str(root),
                     "--eval=print-flags:\n\t@env", "print-flags"],
                    cwd=root, env=environment, capture_output=True, text=True, check=True)
                actual = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
                for flag in flags:
                    self.assertEqual(actual.get(flag), overrides.get(flag, "fixture-" + flag), flag)

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
                'for number in $(seq 1 80); do printf "build-line-%s\\n" "$number"; done\n'
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
        self.assertIn("result=different:ya", result.stderr)
        self.assertIn("/release/ya", result.stderr)
        self.assertLessEqual(len(result.stderr.splitlines()), 41)
        self.assertNotIn("build-line-1", result.stderr.splitlines())

    def test_ci_retains_comparison_report_without_binary_artifacts(self):
        workflow = HELPER.parents[2] / ".github/workflows/ci.yml"
        upload = workflow.read_text().split("- name: Retain native fixture diagnostics", 1)[1].split("- run:", 1)[0]
        patterns = [line.split("/evidence/", 1)[1].strip()
                    for line in upload.splitlines() if "/evidence/" in line]
        files = ["debian-valid.out", "debian-repro.log", "yazi-reproducibility.txt", "yazi", "package.deb"]
        retained = [name for name in files if any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)]
        self.assertEqual(retained, files[:3])

    def test_rejects_reused_target(self):
        result, _ = self.exercise(preexisting=True)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
