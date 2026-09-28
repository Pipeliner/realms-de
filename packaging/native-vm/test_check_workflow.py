#!/usr/bin/env python3
"""Behavioral tests for the native graphical-session workflow admission."""

import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "packaging/native-vm/check_workflow.py"
WORKFLOW = ROOT / ".github/workflows/distro.yml"
KVM_ADMISSION = """\
          if [[ -c /dev/kvm && (! -r /dev/kvm || ! -w /dev/kvm) ]]; then
            sudo setfacl -m "u:$(id -u):rw" /dev/kvm
          fi
"""


def run_checker(workflow: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(CHECKER), str(workflow)],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


class NativeVmWorkflowTests(unittest.TestCase):
    def test_sync_diagnostic_cannot_replace_or_rescue_ordinary_acceptance(self):
        source = WORKFLOW.read_text()
        diagnostic = '      - name: Diagnose Ubuntu llvmpipe synchronization (not acceptance)\n'
        self.assertIn(diagnostic, source)
        ordinary, extra = source.split(diagnostic, 1)
        self.assertIn('id: native-acceptance', ordinary)
        self.assertIn('id: ubuntu-packages', ordinary)
        self.assertIn('name: realm-native-session-${{ matrix.target }}-${{ github.sha }}', ordinary)
        self.assertNotIn('continue-on-error: true', ordinary.split('      - name: Boot installed Realm graphical session', 1)[1])
        step = extra.split('      - name:', 1)[0]
        self.assertIn("if: always() && matrix.target == 'ubuntu-24.04-x86_64' && steps.native-acceptance.outcome == 'failure' && steps.ubuntu-packages.outcome == 'success'", step)
        self.assertIn('timeout-minutes: 25', step)
        self.assertIn('continue-on-error: true', step)
        self.assertIn('"$RUNNER_TEMP/realm-native-packages"', step)
        self.assertIn('--llvmpipe-sync-diagnostic', step)
        self.assertIn('realm-native-sync-diagnostic-ubuntu-${{ github.sha }}', extra)

    def test_checked_in_workflow_consumes_exact_producer_artifacts_without_rebuild(self):
        completed = run_checker(WORKFLOW)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_native_vm_job_without_both_producer_dependencies_is_rejected(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            changed.write_text(
                source.replace(
                    "    needs: [ubuntu-river-debian, fedora-rpm-package]\n",
                    "    needs: ubuntu-river-debian\n",
                    1,
                ),
                encoding="utf-8",
            )
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("both native package producers", completed.stderr)

    def test_native_vm_job_cannot_rebuild_the_downloaded_artifacts(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            marker = "  native-session-vm:\n"
            self.assertIn(marker, source)
            changed.write_text(
                source.replace(marker, marker + "    # cargo build --workspace\n", 1),
                encoding="utf-8",
            )
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("build command", completed.stderr)

    def test_native_vm_job_grants_only_runner_kvm_access_before_harness(self):
        source = WORKFLOW.read_text(encoding="utf-8")
        self.assertIn("            acl \\\n", source)
        self.assertIn(KVM_ADMISSION, source)
        self.assertLess(
            source.index(KVM_ADMISSION),
            source.index("          packaging/native-vm/run-native-session-vm.sh"),
        )
        self.assertNotIn(
            "sudo packaging/native-vm/run-native-session-vm.sh",
            source,
        )

    def test_native_vm_job_without_scoped_kvm_admission_is_rejected(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            self.assertIn(KVM_ADMISSION, source)
            changed.write_text(
                source.replace(KVM_ADMISSION, "", 1),
                encoding="utf-8",
            )
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("runner-UID KVM ACL", completed.stderr)

    def test_native_vm_job_cannot_elevate_the_harness(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            invocation = "          packaging/native-vm/run-native-session-vm.sh"
            self.assertIn(invocation, source)
            changed.write_text(
                source.replace(
                    invocation,
                    "          sudo packaging/native-vm/run-native-session-vm.sh",
                    1,
                ),
                encoding="utf-8",
            )
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("unprivileged", completed.stderr)

    def test_native_vm_job_cannot_broaden_kvm_device_mode(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            self.assertIn(KVM_ADMISSION, source)
            changed.write_text(
                source.replace(
                    KVM_ADMISSION,
                    KVM_ADMISSION + "          sudo chmod a+rw /dev/kvm\n",
                    1,
                ),
                encoding="utf-8",
            )
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("ownership or mode", completed.stderr)

    def test_fast_contract_lane_must_invoke_the_native_workflow_guard(self):
        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "distro.yml"
            source = WORKFLOW.read_text(encoding="utf-8")
            invocation = "          python3 packaging/native-vm/check_workflow.py .github/workflows/distro.yml\n"
            self.assertIn(invocation, source)
            changed.write_text(source.replace(invocation, "", 1), encoding="utf-8")
            completed = run_checker(changed)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("fast contract lane", completed.stderr)


if __name__ == "__main__":
    unittest.main()
