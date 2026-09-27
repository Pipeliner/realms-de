#!/usr/bin/env python3
"""Validate the exact Ubuntu River closure input manifest and cached archives."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlparse

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 development hosts; Noble has 3.12.
    import tomli as tomllib  # type: ignore[no-redef]

ARCHIVE_NAMES = {
    "zig",
    "meson",
    "wayland",
    "wayland-protocols",
    "libdrm",
    "libinput",
    "pixman",
    "libxkbcommon",
    "libdisplay-info",
    "wlroots",
    "river",
}
ZIG_NAMES = {
    "pixman",
    "wayland",
    "wlroots",
    "xkbcommon",
    "translate-c",
    "aro",
    "xkbcommon-wlroots",
}
HEX_40 = re.compile(r"[0-9a-f]{40}\Z")
HEX_64 = re.compile(r"[0-9a-f]{64}\Z")
ZIG_HASH = re.compile(r"[A-Za-z0-9_-]+-[0-9][A-Za-z0-9._-]*-[A-Za-z0-9_-]{30,}\Z")


class ManifestError(Exception):
    pass


def table_list(document: dict[str, object], key: str) -> list[dict[str, object]]:
    value = document.get(key)
    if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
        raise ManifestError(f"{key} must be an array of tables")
    return value  # type: ignore[return-value]


def require_string(table: dict[str, object], field: str, context: str) -> str:
    value = table.get(field)
    if not isinstance(value, str) or not value:
        raise ManifestError(f"{context} has invalid {field}")
    return value


def validate_names(tables: list[dict[str, object]], selected: set[str], kind: str) -> None:
    names = [require_string(table, "name", kind) for table in tables]
    duplicates = sorted({name for name in names if names.count(name) > 1})
    if duplicates:
        raise ManifestError(f"duplicate {kind} name: {duplicates[0]}")
    actual = set(names)
    if actual != selected:
        missing = ",".join(sorted(selected - actual)) or "-"
        extra = ",".join(sorted(actual - selected)) or "-"
        raise ManifestError(f"{kind} inventory differs: missing={missing} extra={extra}")


def validate_url(value: str, *, git_allowed: bool) -> None:
    checked = value.removeprefix("git+") if git_allowed else value
    parsed = urlparse(checked)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise ManifestError(f"invalid {'Zig ' if git_allowed else ''}URL")
    if value.startswith("git+") and not git_allowed:
        raise ManifestError("archive URL must use HTTPS")


def load_manifest(path: Path, repo_root: Path) -> tuple[list[dict[str, object]], list[dict[str, object]]]:
    try:
        document = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as error:
        raise ManifestError(f"cannot read manifest: {error}") from error
    if set(document) != {"schema", "nixpkgs_revision", "archive", "zig_dependency"}:
        raise ManifestError("manifest has unknown or missing top-level fields")
    if document["schema"] != 1:
        raise ManifestError("unsupported manifest schema")

    revision = document["nixpkgs_revision"]
    if not isinstance(revision, str) or not HEX_40.fullmatch(revision):
        raise ManifestError("invalid nixpkgs revision")
    try:
        lock = json.loads((repo_root / "flake.lock").read_text(encoding="utf-8"))
        locked_revision = lock["nodes"]["nixpkgs"]["locked"]["rev"]
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        raise ManifestError(f"cannot read nixpkgs revision from flake.lock: {error}") from error
    if revision != locked_revision:
        raise ManifestError("manifest nixpkgs revision does not match flake.lock")

    archives = table_list(document, "archive")
    zig_dependencies = table_list(document, "zig_dependency")
    validate_names(archives, ARCHIVE_NAMES, "archive")
    validate_names(zig_dependencies, ZIG_NAMES, "Zig dependency")

    filenames: set[str] = set()
    for archive in archives:
        if set(archive) != {"name", "version", "filename", "url", "sha256", "role"}:
            raise ManifestError("archive has unknown or missing fields")
        context = f"archive {require_string(archive, 'name', 'archive')}"
        require_string(archive, "version", context)
        require_string(archive, "role", context)
        filename = require_string(archive, "filename", context)
        if Path(filename).name != filename or filename in filenames:
            raise ManifestError(f"{context} has invalid or duplicate filename")
        filenames.add(filename)
        validate_url(require_string(archive, "url", context), git_allowed=False)
        if not HEX_64.fullmatch(require_string(archive, "sha256", context)):
            raise ManifestError(f"invalid archive sha256: {context}")

    for dependency in zig_dependencies:
        if set(dependency) != {"name", "url", "content_hash"}:
            raise ManifestError("Zig dependency has unknown or missing fields")
        context = f"Zig dependency {require_string(dependency, 'name', 'Zig dependency')}"
        validate_url(require_string(dependency, "url", context), git_allowed=True)
        if not ZIG_HASH.fullmatch(require_string(dependency, "content_hash", context)):
            raise ManifestError(f"invalid Zig content hash: {context}")
    return archives, zig_dependencies


def validate_archive_cache(archives: list[dict[str, object]], directory: Path) -> None:
    expected = {require_string(item, "filename", "archive") for item in archives}
    try:
        actual = {item.name for item in directory.iterdir()}
    except OSError as error:
        raise ManifestError(f"cannot read archive cache: {error}") from error
    if actual != expected:
        missing = ",".join(sorted(expected - actual)) or "-"
        extra = ",".join(sorted(actual - expected)) or "-"
        raise ManifestError(f"archive cache inventory differs: missing={missing} extra={extra}")
    by_name = {require_string(item, "filename", "archive"): item for item in archives}
    for filename in sorted(expected):
        path = directory / filename
        if path.is_symlink() or not path.is_file():
            raise ManifestError(f"cached archive is not a regular file: {filename}")
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        if digest != require_string(by_name[filename], "sha256", f"archive {filename}"):
            raise ManifestError(f"cached archive digest mismatch: {filename}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, required=True)
    parser.add_argument("--archive-dir", type=Path)
    parser.add_argument("--emit", choices=("archives", "zig-dependencies"))
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    try:
        archives, zig_dependencies = load_manifest(args.manifest, args.repo_root)
        if args.archive_dir is not None:
            validate_archive_cache(archives, args.archive_dir)
        if args.emit == "archives":
            for item in archives:
                print("\t".join(require_string(item, key, "archive") for key in ("name", "filename", "url", "sha256")))
        elif args.emit == "zig-dependencies":
            for item in zig_dependencies:
                print("\t".join(require_string(item, key, "Zig dependency") for key in ("name", "url", "content_hash")))
    except ManifestError as error:
        print(f"closure manifest: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
