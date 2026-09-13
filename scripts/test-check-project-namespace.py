#!/usr/bin/env python3
"""Regression fixtures for the project-namespace byte boundary."""

from __future__ import annotations

import gzip
import io
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/check-project-namespace"
ARCHIVE = Path("packaging/tool-sources/bundles/realm-workspace/source.tar.gz")
RETIRED = bytes((104, 101, 108, 109))


def write_archive(
    path: Path,
    *,
    payload: bytes = b"clean\n",
    link_target: str | None = None,
) -> None:
    tar_bytes = io.BytesIO()
    with tarfile.open(fileobj=tar_bytes, mode="w") as contents:
        regular = tarfile.TarInfo("realm-workspace/clean.txt")
        regular.size = len(payload)
        contents.addfile(regular, io.BytesIO(payload))
        if link_target is not None:
            link = tarfile.TarInfo("realm-workspace/link")
            link.type = tarfile.SYMTYPE
            link.linkname = link_target
            contents.addfile(link)
    path.parent.mkdir(parents=True)
    path.write_bytes(gzip.compress(tar_bytes.getvalue(), mtime=0))


def add_gzip_comment(path: Path, comment: bytes) -> None:
    compressed = bytearray(path.read_bytes())
    if compressed[:3] != bytes((31, 139, 8)) or compressed[3] != 0:
        raise AssertionError("fixture gzip header is not the expected minimal form")
    compressed[3] = 16
    path.write_bytes(compressed[:10] + comment + b"\0" + compressed[10:])


def run_checker(repo: Path) -> subprocess.CompletedProcess[str]:
    subprocess.run(["git", "init", "--quiet"], cwd=repo, check=True)
    subprocess.run(["git", "add", "--all"], cwd=repo, check=True)
    return subprocess.run(
        ["python3", str(CHECKER)],
        cwd=repo,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


class ProjectNamespaceTests(unittest.TestCase):
    def test_opaque_gzip_envelope_bytes_are_not_project_text(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repo = Path(raw)
            archive = repo / ARCHIVE
            write_archive(archive)
            add_gzip_comment(archive, RETIRED)
            self.assertIn(RETIRED, archive.read_bytes())
            completed = run_checker(repo)
            self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_decoded_archive_content_is_checked(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repo = Path(raw)
            write_archive(repo / ARCHIVE, payload=b"prefix-" + RETIRED)
            completed = run_checker(repo)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("retired namespace in archive content", completed.stderr)

    def test_decoded_archive_link_target_is_checked(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repo = Path(raw)
            target = (b"prefix-" + RETIRED).decode("ascii")
            write_archive(repo / ARCHIVE, link_target=target)
            completed = run_checker(repo)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("retired namespace in archive link target", completed.stderr)

    def test_other_tracked_binary_bytes_remain_checked(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            repo = Path(raw)
            path = repo / "opaque.bin"
            path.write_bytes(bytes((0, 255)) + RETIRED + bytes((0,)))
            completed = run_checker(repo)
            self.assertNotEqual(completed.returncode, 0)
            self.assertIn("retired namespace in content: opaque.bin", completed.stderr)


if __name__ == "__main__":
    unittest.main()
