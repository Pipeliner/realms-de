#!/usr/bin/env python3
"""Validate the exact retained-only Noble realm-river source kit."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib  # type: ignore[no-redef]

TOP_LEVEL = {"closure-input", "debian", "flake.lock", "packaging"}
RIVER_FILES = {
    "build-noble-package-closure.sh",
    "check-closure-manifest.py",
    "check-private-resolutions.py",
    "probe-noble-package-closure.sh",
    "sources.toml",
}
DEBIAN_FILES = {"changelog", "control", "copyright", "rules", "source"}


def names(path: Path) -> set[str]:
    if path.is_symlink() or not path.is_dir():
        return set()
    return {entry.name for entry in path.iterdir()}


def require_inventory(path: Path, expected: set[str], message: str) -> None:
    if names(path) != expected:
        raise SystemExit(message)


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} SOURCE-KIT")
    root = Path(sys.argv[1]).resolve()
    require_inventory(root, TOP_LEVEL, "source-kit top-level inventory differs")
    require_inventory(root / "packaging", {"river"}, "source-kit packaging inventory differs")
    river = root / "packaging" / "river"
    require_inventory(river, RIVER_FILES, "source-kit River helper inventory differs")
    require_inventory(root / "debian", DEBIAN_FILES, "source-kit Debian inventory differs")
    require_inventory(root / "debian" / "source", {"format"}, "source-kit Debian source inventory differs")
    closure = root / "closure-input"
    require_inventory(closure, {"archives", "tools", "zig-pkg"}, "closure input inventory differs")
    require_inventory(
        closure / "tools",
        {"zig-x86_64-linux-0.16.0"},
        "selected Zig tool inventory differs",
    )
    zig = closure / "tools" / "zig-x86_64-linux-0.16.0" / "zig"
    if zig.is_symlink() or not zig.is_file() or not zig.stat().st_mode & 0o111:
        raise SystemExit("selected Zig executable is absent or unsafe")

    manifest = river / "sources.toml"
    checker = river / "check-closure-manifest.py"
    result = subprocess.run(
        [
            str(checker),
            "--repo-root",
            str(root),
            "--archive-dir",
            str(closure / "archives"),
            str(manifest),
        ],
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(result.returncode)
    document = tomllib.loads(manifest.read_text(encoding="utf-8"))
    expected_hashes = {item["content_hash"] for item in document["zig_dependency"]}
    require_inventory(
        closure / "zig-pkg",
        expected_hashes,
        "Zig package inventory differs",
    )

    for entry in root.rglob("*"):
        if entry.is_symlink() or not (entry.is_file() or entry.is_dir()):
            raise SystemExit(f"source kit contains unsafe entry: {entry.relative_to(root)}")


if __name__ == "__main__":
    main()
