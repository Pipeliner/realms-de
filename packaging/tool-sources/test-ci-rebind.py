#!/usr/bin/env python3
"""Pure contract tests for CI-only Realm workspace source rebinding."""

from __future__ import annotations

import importlib.util
import os
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
PRODUCER = ROOT / "packaging/tool-sources/ci_rebind_realm_workspace.py"
WORKFLOW = ROOT / ".github/workflows/rebind-realm-workspace.yml"


def load_producer():
    if not PRODUCER.is_file():
        raise AssertionError(f"missing CI rebind producer: {PRODUCER}")
    spec = importlib.util.spec_from_file_location("ci_rebind_realm_workspace", PRODUCER)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RebindTransformContract(unittest.TestCase):
    def test_workspace_edge_refresh_preserves_external_records_and_identities(self):
        producer = load_producer()
        old = b'''version = 4

[[package]]
name = "realm-test"
version = "0.1.0"
dependencies = [
 "serde",
]

[[package]]
name = "serde"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
'''
        changed = old.replace(b' "serde",', b' "serde",\n "realm-other",')
        producer.require_workspace_edge_refresh(old, changed)
        for invalid in (
            changed.replace(b'version = "1.0.0"', b'version = "1.0.1"'),
            changed.replace(b'checksum = "a', b'checksum = "b'),
            changed.replace(b'name = "realm-test"', b'name = "realm-renamed"'),
            changed.replace(b'version = "0.1.0"', b'version = "0.2.0"'),
            changed.replace(b'version = 4', b'version = 3'),
            changed + b'unknown = "field"\n',
            changed + changed[changed.index(b'[[package]]'):],
        ):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                producer.require_workspace_edge_refresh(old, invalid)

    def test_workspace_refresh_still_refuses_local_production(self):
        producer = load_producer()
        output = ROOT / ".ci-refresh-test-output"
        with mock.patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(
            RuntimeError, "only in CI"
        ):
            producer.produce("a" * 40, output, ROOT, refresh_workspace_lock=True)
        self.assertFalse(output.exists())

    def test_retained_payload_digest_is_required(self):
        producer = load_producer()
        import hashlib
        payload = b"retained payload"
        digest = hashlib.sha256(payload).hexdigest()
        manifest = f'vendor_archive_sha256 = "{digest}"\n'
        producer.require_payload_digest(manifest, "vendor_archive", payload)
        for invalid in (b"changed", b""):
            with self.assertRaises(ValueError):
                producer.require_payload_digest(manifest, "vendor_archive", invalid)
        with self.assertRaises(ValueError):
            producer.require_payload_digest(manifest + manifest, "vendor_archive", payload)

    def test_requires_an_exact_full_commit_id(self):
        producer = load_producer()
        commit = "a" * 40
        self.assertEqual(producer.normalize_commit(commit), commit)
        for invalid in ("main", "a" * 39, "A" * 40, "a" * 41, "g" * 40):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                producer.normalize_commit(invalid)

    def test_updates_only_bound_source_fields(self):
        producer = load_producer()
        old_commit = "1" * 40
        new_commit = "2" * 40
        old_source = "a" * 64
        new_source = "b" * 64
        old_provenance = "c" * 64
        new_provenance = "d" * 64
        manifest = f'''[bundle]
name = "realm-workspace"
commit = "{old_commit}"
commit_timestamp = "2026-09-13T08:00:00Z"
source_sha256 = "{old_source}"
source_provenance_sha256 = "{old_provenance}"
lockfile_sha256 = "unchanged"
'''
        updated = producer.update_manifest(
            manifest,
            commit=new_commit,
            timestamp="2026-09-13T09:00:00Z",
            source_sha256=new_source,
            provenance_sha256=new_provenance,
        )
        self.assertIn(f'commit = "{new_commit}"', updated)
        self.assertIn('commit_timestamp = "2026-09-13T09:00:00Z"', updated)
        self.assertIn(f'source_sha256 = "{new_source}"', updated)
        self.assertIn(f'source_provenance_sha256 = "{new_provenance}"', updated)
        self.assertIn('lockfile_sha256 = "unchanged"', updated)

    def test_updates_only_bound_provenance_fields(self):
        producer = load_producer()
        text = '''# Provenance
- Commit: `1111111111111111111111111111111111111111`
- Commit timestamp: `2026-09-13T08:00:00Z` (`100`)
- Canonical archive SHA-256:
  `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`

Dependency statement remains byte-exact.
'''
        updated = producer.update_provenance(
            text,
            commit="2" * 40,
            timestamp="2026-09-13T09:00:00Z",
            epoch="200",
            source_sha256="b" * 64,
        )
        self.assertIn(f'- Commit: `{"2" * 40}`', updated)
        self.assertIn('- Commit timestamp: `2026-09-13T09:00:00Z` (`200`)', updated)
        self.assertIn(f'  `{"b" * 64}`', updated)
        self.assertIn("Dependency statement remains byte-exact.", updated)

    def test_rejects_a_lockfile_refresh(self):
        producer = load_producer()
        producer.require_same_lockfile(b"same\n", b"same\n")
        with self.assertRaisesRegex(ValueError, "dependency-closure refresh"):
            producer.require_same_lockfile(b"retained\n", b"changed\n")

    def test_producer_refuses_to_create_a_local_candidate(self):
        producer = load_producer()
        output = ROOT / ".ci-rebind-test-output"
        with mock.patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(
            RuntimeError, "only in CI"
        ):
            producer.produce("a" * 40, output, ROOT)
        self.assertFalse(output.exists())


class RebindWorkflowContract(unittest.TestCase):
    def test_native_evidence_has_namespace_preflight_and_matching_upload(self):
        import yaml

        document = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
        steps = document["jobs"]["docs"]["steps"]
        fixture = next(s for s in steps if s.get("name") == "Check network-isolated native package paths")
        evidence = '${{ steps.native-temp.outputs.path }}/evidence'
        self.assertIn('REALM_NATIVE_EVIDENCE_DIR="' + evidence + '"', fixture["run"])
        probe = next(s for s in steps if s.get("name") == "Probe native evidence from build namespace")
        self.assertLess(steps.index(probe), steps.index(fixture))
        self.assertIn("unshare --user --map-root-user --net", probe["run"])
        self.assertIn(evidence, probe["run"])
        upload = next(s for s in steps if s.get("name") == "Retain native fixture diagnostics")
        self.assertEqual(upload["if"], "always()")
        # The launch-policy test owns the exact artifact allowlist. This test
        # verifies its relationship to the namespace's evidence root.
        paths = upload["with"]["path"].splitlines()
        self.assertTrue(paths)
        for path in paths:
            self.assertEqual(path.rsplit("/", 1)[0], evidence)

    def test_preparation_validates_before_staging_and_retains_provenance(self):
        text = (ROOT / ".github/actions/prepare-realm-source/action.yml").read_text()
        self.assertIn("source_commit=$(git rev-parse HEAD)", text)
        self.assertLess(text.index('git config --global --add safe.directory "$GITHUB_WORKSPACE"'),
                        text.index("source_commit=$(git rev-parse HEAD)"))
        self.assertLess(text.index("ci_rebind_realm_workspace.py"),
                        text.index("check-bundle-linkage.py"))
        self.assertLess(text.index("check-bundle-linkage.py"), text.index('cp "$candidate/$file"'))
        self.assertIn("for file in source.tar.gz bundle.toml provenance.md", text)
        self.assertIn("actions/upload-artifact@", text)
        self.assertIn("set -euo pipefail", text)

    def test_consumers_prepare_binding_before_package_checks(self):
        import yaml

        consumers = {"ci.yml": ["docs"], "distro.yml": [
            "ubuntu-river-debian", "ubuntu-debian-package", "fedora-rpm-package", "nix"
        ]}
        for workflow, jobs in consumers.items():
            document = yaml.safe_load((ROOT / ".github/workflows" / workflow).read_text())
            for job in jobs:
                steps = document["jobs"][job]["steps"]
                preparation = [i for i, step in enumerate(steps)
                               if step.get("uses") == "./.github/actions/prepare-realm-source"]
                self.assertEqual(len(preparation), 1, (workflow, job))
                consumers_at = [i for i, step in enumerate(steps)
                                if any(command in step.get("run", "") for command in (
                                    "test-native-source-kits.sh", "build-native-source-kits.sh",
                                    "nix build", "nix flake check"))]
                self.assertTrue(consumers_at, (workflow, job))
                self.assertLess(preparation[0], min(consumers_at))

    def test_workflow_is_read_only_and_retains_exact_candidate_files(self):
        text = WORKFLOW.read_text(encoding="utf-8")
        for required in (
            "workflow_dispatch:",
            "pull_request:",
            "source_commit:",
            "permissions:\n  contents: read",
            "fetch-depth: 0",
            "path: tooling",
            "path: source",
            "ref: ${{ inputs.source_commit || github.event.pull_request.head.sha }}",
            "SOURCE_COMMIT: ${{",
            "python3 tooling/packaging/tool-sources/ci_rebind_realm_workspace.py",
            '--repo "$GITHUB_WORKSPACE/source"',
            '--commit "$SOURCE_COMMIT"',
            "python3 tooling/packaging/tool-sources/check-bundle-linkage.py",
            "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
            "source.tar.gz",
            "bundle.toml",
            "provenance.md",
        ):
            with self.subTest(required=required):
                self.assertIn(required, text)
        for forbidden in (
            "contents: write",
            "git push",
            "git commit",
            "--commit '${{ inputs.source_commit }}'",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, text)


if __name__ == "__main__":
    unittest.main()
