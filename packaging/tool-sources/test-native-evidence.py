"""Exercise evidence routing and preflight shell; no package execution."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import yaml

ROOT = Path(__file__).resolve().parents[2]


class NativeEvidence(unittest.TestCase):
    def setUp(self):
        self.steps = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())["jobs"]["docs"]["steps"]

    def step(self, name):
        return next((step for step in self.steps if step.get("name") == name), None)

    def test_driver_and_artifact_share_traversable_evidence_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sudo = root / "sudo"
            sudo.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n')
            sudo.chmod(0o755)
            command = self.step("Check network-isolated native package paths")["run"]
            command = command.replace("${{ steps.native-temp.outputs.path }}", str(root))
            command = command.replace("${{ runner.temp }}", "/runner/private")
            command = command.replace("${{ steps.native-rust.outputs.path }}", "/unused-toolchain")
            result = subprocess.run(["bash", "-c", command], capture_output=True, text=True,
                                    env=dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"]))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("REALM_NATIVE_EVIDENCE_DIR=" + str(root / "evidence"), result.stdout.splitlines())
            upload = self.step("Retain native fixture diagnostics")["with"]["path"]
            upload = upload.replace("${{ steps.native-temp.outputs.path }}", str(root))
            # Exact allowlist coverage belongs to test-launch-ci-policy.py;
            # exercise the actual runner-path substitution for every entry.
            self.assertTrue(upload.splitlines())
            for path in upload.splitlines():
                self.assertEqual(Path(path).parent, root / "evidence")

    def test_namespace_probe_creates_appends_and_reports_failure(self):
        step = self.step("Probe native evidence from build namespace")
        self.assertIsNotNone(step, "missing build-namespace evidence preflight")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name, source in {
                "sudo": '#!/bin/sh\nexec "$@"\n',
                "unshare": '#!/bin/sh\n[ "$1 $2 $3" = "--user --map-root-user --net" ] || exit 90\nshift 3\n[ "${DENY_PROBE:-0}" = 0 ] || exit 91\nexec "$@"\n',
                "namei": '#!/bin/sh\necho ancestor-permissions\n',
            }.items():
                path = root / name
                path.write_text(source)
                path.chmod(0o755)
            command = step["run"].replace("${{ steps.native-temp.outputs.path }}", str(root))
            for deny in ("0", "1"):
                result = subprocess.run(["bash", "-c", command], capture_output=True, text=True,
                    env=dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"], DENY_PROBE=deny))
                if deny == "0":
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(list((root / "evidence").iterdir()), [])
                else:
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("ancestor-permissions", result.stdout)


if __name__ == "__main__":
    unittest.main()
