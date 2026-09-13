#!/usr/bin/env python3
"""Contract tests for native graphical-session VM admission and evidence."""

import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "packaging/native-vm/check_inputs.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_inputs", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError(f"cannot load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class NativeVmInputTests(unittest.TestCase):
    def setUp(self):
        self.module = load_module()

    def native_doctor_checks(self):
        checks = [
            {"id": check_id, "status": "ok", "summary": "fixture"}
            for check_id in self.module.CHECK_IDS
        ]
        by_id = {check["id"]: check for check in checks}
        for check_id in {"units/idle-lock", "portal/filechooser"}:
            by_id[check_id]["status"] = "skip"
        by_id["tools/floors"].update(
            status="warn",
            summary=(
                "yazi: not found, btop: btop version: 1.3.0, "
                "starship: not found; install the missing or unparseable tools"
            ),
        )
        return checks

    def test_image_bytes_must_match_the_selected_target_digest(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            image = root / "image.qcow2"
            image.write_bytes(b"official-image")
            manifest = root / "images.json"
            manifest.write_text(
                json.dumps(
                    {
                        "schema": "realm-native-vm-images/v1",
                        "targets": {
                            "fixture": {
                                "url": "https://images.example.invalid/image.qcow2",
                                "sha256": hashlib.sha256(b"official-image").hexdigest(),
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )

            selected = self.module.verify_image(manifest, "fixture", image)
            self.assertEqual(selected["url"], "https://images.example.invalid/image.qcow2")

            image.write_bytes(b"official-image-mutated")
            with self.assertRaisesRegex(ValueError, "image SHA-256 mismatch"):
                self.module.verify_image(manifest, "fixture", image)

    def test_production_manifest_has_only_the_two_accepted_targets(self):
        manifest = ROOT / "packaging/native-vm/images.json"
        self.module.validate_manifest(manifest)

        with tempfile.TemporaryDirectory() as raw:
            changed = Path(raw) / "images.json"
            document = json.loads(manifest.read_text(encoding="utf-8"))
            document["targets"]["unreviewed-image"] = document["targets"][
                "ubuntu-24.04-x86_64"
            ]
            changed.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "unexpected target inventory"):
                self.module.validate_manifest(changed)

    def test_ubuntu_inventory_requires_exact_realm_and_realm_river_debs(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            realm = root / "realm_0.1.0_amd64.deb"
            river = root / "realm-river_0.4.8-1_amd64.deb"
            realm.touch()
            river.touch()
            facts = {
                realm: {"name": "realm", "version": "0.1.0", "arch": "amd64"},
                river: {"name": "realm-river", "version": "0.4.8-1", "arch": "amd64"},
            }

            selected = self.module.select_packages("ubuntu-24.04-x86_64", root, facts.__getitem__)
            self.assertEqual(selected, [realm, river])

            extra = root / "other_1_amd64.deb"
            extra.touch()
            facts[extra] = {"name": "other", "version": "1", "arch": "amd64"}
            with self.assertRaisesRegex(ValueError, "exactly two Debian packages"):
                self.module.select_packages("ubuntu-24.04-x86_64", root, facts.__getitem__)

    def test_debian_identity_uses_unlabelled_show_format_records(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            package = root / "realm.deb"
            package.touch()
            fake_bin = root / "bin"
            fake_bin.mkdir()
            dpkg_deb = fake_bin / "dpkg-deb"
            dpkg_deb.write_text(
                """#!/bin/sh
if [ "$1" = "-f" ]; then
    printf 'Package: realm\nVersion: 0.1.0\nArchitecture: amd64\n'
    exit 0
fi
if [ "$1" = '--showformat=${Package}\\n${Version}\\n${Architecture}\\n' ] \
    && [ "$2" = "--show" ]; then
    printf 'realm\n0.1.0\namd64\n'
    exit 0
fi
exit 97
""",
                encoding="utf-8",
            )
            dpkg_deb.chmod(0o755)
            path = f"{fake_bin}:{os.environ['PATH']}"
            with mock.patch.dict(os.environ, {"PATH": path}):
                facts = self.module._deb_facts(package)
            self.assertEqual(
                facts,
                {"name": "realm", "version": "0.1.0", "arch": "amd64"},
            )

    def test_non_regular_artifact_entry_is_rejected_not_ignored(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            realm = root / "realm_0.1.0_amd64.deb"
            river = root / "realm-river_0.4.8-1_amd64.deb"
            realm.touch()
            river.touch()
            (root / "unexpected-link.deb").symlink_to(realm)
            facts = {
                realm: {"name": "realm", "version": "0.1.0", "arch": "amd64"},
                river: {
                    "name": "realm-river",
                    "version": "0.4.8-1",
                    "arch": "amd64",
                },
            }
            with self.assertRaisesRegex(ValueError, "regular files"):
                self.module.select_packages("ubuntu-24.04-x86_64", root, facts.__getitem__)

    def test_installed_payload_identity_cannot_be_masked_by_valid_filenames(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            realm = root / "realm_0.1.0_amd64.deb"
            river = root / "realm-river_0.4.8-1_amd64.deb"
            realm.touch()
            river.touch()
            facts = {
                realm: {"name": "not-realm", "version": "0.1.0", "arch": "amd64"},
                river: {"name": "realm-river", "version": "0.4.8-1", "arch": "amd64"},
            }

            with self.assertRaisesRegex(ValueError, "unexpected Debian package identity"):
                self.module.select_packages("ubuntu-24.04-x86_64", root, facts.__getitem__)

    def test_fedora_inventory_requires_one_x86_64_realm_rpm(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            realm = root / "realm-0.1.0-1.fc44.x86_64.rpm"
            realm.touch()

            selected = self.module.select_packages(
                "fedora-44-x86_64",
                root,
                lambda _path: {"name": "realm", "version": "0.1.0-1.fc44", "arch": "x86_64"},
            )
            self.assertEqual(selected, [realm])

            with self.assertRaisesRegex(ValueError, "unexpected RPM package identity"):
                self.module.select_packages(
                    "fedora-44-x86_64",
                    root,
                    lambda _path: {"name": "realm", "version": "0.1.0-1.fc44", "arch": "aarch64"},
                )

    def test_doctor_requires_ordered_healthy_session_and_exact_skips(self):
        checks = self.native_doctor_checks()
        report = {"checks": checks}

        self.module.validate_doctor(report)

        checks[0]["status"] = "fail"
        with self.assertRaisesRegex(ValueError, "doctor contains failed checks"):
            self.module.validate_doctor(report)

        checks[0]["status"] = "invented"
        with self.assertRaisesRegex(ValueError, "unknown status"):
            self.module.validate_doctor(report)

    def test_doctor_accepts_only_the_declared_native_tool_warning(self):
        checks = self.native_doctor_checks()
        report = {"checks": checks}
        by_id = {check["id"]: check for check in checks}

        by_id["tools/floors"]["status"] = "skip"
        with self.assertRaisesRegex(ValueError, "doctor skip set mismatch"):
            self.module.validate_doctor(report)

        checks = self.native_doctor_checks()
        report = {"checks": checks}
        by_id = {check["id"]: check for check in checks}
        by_id["tools/floors"]["summary"] = (
            "yazi: not found, btop: not found, starship: not found; "
            "install the missing or unparseable tools"
        )
        with self.assertRaisesRegex(ValueError, "native tool evidence"):
            self.module.validate_doctor(report)

        checks = self.native_doctor_checks()
        report = {"checks": checks}
        by_id = {check["id"]: check for check in checks}
        by_id["tools/floors"]["summary"] = (
            "yazi: yazi 26.8.15, btop: btop version: 1.3.0, "
            "starship: not found; install the missing or unparseable tools"
        )
        with self.assertRaisesRegex(ValueError, "native tool evidence"):
            self.module.validate_doctor(report)

        checks = self.native_doctor_checks()
        report = {"checks": checks}
        by_id = {check["id"]: check for check in checks}
        by_id["units/bar"]["status"] = "warn"
        with self.assertRaisesRegex(ValueError, "unexpected doctor warnings"):
            self.module.validate_doctor(report)

    def test_guest_evidence_validation_does_not_require_image_manifest(self):
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            copied_probe = root / "check_inputs.py"
            shutil.copyfile(MODULE_PATH, copied_probe)
            checks = self.native_doctor_checks()
            report = root / "doctor.json"
            report.write_text(json.dumps({"checks": checks}), encoding="utf-8")

            completed = subprocess.run(
                ["python3", str(copied_probe), "doctor", str(report)],
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_get_state_requires_v2_hello_then_state(self):
        frames = [
            {"reply": "hello", "data": {"version": 2, "session": "0.1.0"}},
            {"reply": "state", "data": {"orbits": [], "active": 1}},
        ]
        self.module.validate_control_frames(frames)

        frames[0]["data"]["version"] = 1
        with self.assertRaisesRegex(ValueError, "protocol-v2 Hello"):
            self.module.validate_control_frames(frames)


if __name__ == "__main__":
    unittest.main()
