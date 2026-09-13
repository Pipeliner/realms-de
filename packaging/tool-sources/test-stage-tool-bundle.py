#!/usr/bin/env python3
"""Synthetic tests for the one record-bound selected-tool source patch."""

import hashlib
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "packaging/tool-sources/check-bundle-linkage.py"
sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("realm_bundle_linkage", CHECKER)
assert SPEC is not None and SPEC.loader is not None
LINKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(LINKAGE)


PREIMAGE = b"fn main() {\n    build();\n}\n"
POSTIMAGE = b"fn main() {\n    deny_tree();\n    build();\n}\n"
PATCH = b"""--- a/build.rs
+++ b/build.rs
@@ -1,3 +1,4 @@
 fn main() {
+    deny_tree();
     build();
 }
"""


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


class BoundSourcePatchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.bundle = self.root / "bundle"
        self.source = self.root / "source"
        self.vendor = self.root / "vendor"
        self.bundle.mkdir()
        self.source.mkdir()
        self.vendor.mkdir()
        (self.source / "build.rs").write_bytes(PREIMAGE)
        (self.bundle / "deny-tree.patch").write_bytes(PATCH)
        self.config = self.root / "config.toml"
        self.config.write_text("[source.crates-io]\nreplace-with = 'retained'\n")
        self.record = {
            "source_patch": "deny-tree.patch",
            "source_patch_sha256": digest(PATCH),
            "source_patch_target": "build.rs",
            "source_patch_pre_sha256": digest(PREIMAGE),
            "source_patch_post_sha256": digest(POSTIMAGE),
        }

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_materialization_reapplies_patch_to_original_source(self) -> None:
        destination = self.root / "stage"
        for _ in range(2):
            staged = LINKAGE.materialize_stage(
                self.source,
                self.vendor,
                self.config,
                destination,
                self.bundle,
                self.record,
            )
            self.assertEqual((staged / "build.rs").read_bytes(), POSTIMAGE)
            self.assertEqual((self.source / "build.rs").read_bytes(), PREIMAGE)

    def test_failed_replacement_restores_previous_complete_stage(self) -> None:
        destination = self.root / "stage"
        destination.mkdir()
        (destination / "prior-marker").write_text("complete\n")
        original_rename = Path.rename

        def fail_new_publication(path: Path, target: Path) -> Path:
            if path.name == "staged" and Path(target) == destination:
                raise OSError("fixture publication failure")
            return original_rename(path, target)

        with mock.patch.object(Path, "rename", fail_new_publication):
            with self.assertRaisesRegex(OSError, "fixture publication failure"):
                LINKAGE.materialize_stage(
                    self.source,
                    self.vendor,
                    self.config,
                    destination,
                    self.bundle,
                    self.record,
                )
        self.assertEqual((destination / "prior-marker").read_text(), "complete\n")
        self.assertFalse((destination / "build.rs").exists())

    def test_patch_metadata_is_fail_closed(self) -> None:
        cases = {
            "source patch SHA-256 mismatch": {"source_patch_sha256": "0" * 64},
            "source patch preimage SHA-256 mismatch": {
                "source_patch_pre_sha256": "0" * 64
            },
            "source patch postimage SHA-256 mismatch": {
                "source_patch_post_sha256": "0" * 64
            },
            "bundle source_patch path escapes bundle root": {
                "source_patch": "../deny-tree.patch"
            },
            "source patch target differs from build.rs": {
                "source_patch_target": "src/main.rs"
            },
        }
        for expected, changes in cases.items():
            with self.subTest(expected=expected):
                record = self.record | changes
                with self.assertRaisesRegex(SystemExit, expected):
                    LINKAGE.apply_bound_source_patch(
                        self.source, self.bundle, record
                    )

    def test_symlinked_patch_is_rejected(self) -> None:
        (self.bundle / "deny-tree.patch").unlink()
        (self.bundle / "patch-target").write_bytes(PATCH)
        (self.bundle / "deny-tree.patch").symlink_to("patch-target")
        with self.assertRaisesRegex(SystemExit, "source patch is missing or symlinked"):
            LINKAGE.apply_bound_source_patch(self.source, self.bundle, self.record)


if __name__ == "__main__":
    unittest.main()
