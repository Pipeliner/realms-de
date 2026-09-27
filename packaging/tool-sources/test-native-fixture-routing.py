#!/usr/bin/env python3
"""Source-only helper execution; no packages, bundles, or compilation."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class FixtureRouting(unittest.TestCase):
    def test_support_survives_inaccessible_checkout_ancestor(self):
        source = Path(__file__).with_name("test-native-builds.sh").read_text()
        functions = []
        for name in ("stage_native_support", "make_sentinels", "preflight_native_support"):
            self.assertTrue(name + "() {" in source, f"missing {name}")
            body = source.split(name + "() {", 1)[1].split("\n}", 1)[0]
            functions.append(name + "() {" + body + "\n}\n")
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            checkout = base / "private-checkout"
            support_source = checkout / "packaging/tool-sources"
            support_source.mkdir(parents=True)
            for name in ("native-command-sentinel.sh", "native-cargo-wrapper.sh", "check-yazi-reproducibility.sh"):
                shutil.copy2(Path(__file__).with_name(name), support_source / name)
            script = "set -eu\n" + "".join(functions)
            script += '''
run_isolated() { "$@"; }
root=$1
tmp=$2
real_cargo=/bin/true
real_rustc=/bin/true
stage_native_support
chmod 000 "$root"
test ! -r "$root/packaging/tool-sources/native-command-sentinel.sh"
preflight_native_support
make_sentinels "$tmp/case-sentinels"
status=0
REALM_SENTINEL_LOG="$tmp/log" "$tmp/case-sentinels/git" fetch https://example.invalid/realm || status=$?
[ "$status" = 97 ]
grep '^forbidden|command=git|' "$tmp/log"
status=0
sh "$native_support/check-yazi-reproducibility.sh" 2>"$tmp/helper-error" || status=$?
[ "$status" = 2 ]
grep '^usage:' "$tmp/helper-error"
'''
            scratch = base / "scratch"
            scratch.mkdir()
            try:
                result = subprocess.run(["sh", "-c", script, "fixture", str(checkout), str(scratch)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn("forbidden|command=git|", result.stdout)
                self.assertFalse((scratch / "case-sentinels/git").is_symlink())
            finally:
                checkout.chmod(0o700)

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
