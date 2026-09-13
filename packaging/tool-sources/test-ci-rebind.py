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
