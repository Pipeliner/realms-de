#!/usr/bin/env python3
"""Validate the native realm-river binary package identity and payload."""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

REQUIRED = {
    "libwlroots": re.compile(r"libwlroots-0\.20\.so(?:\..*)?\Z"),
    "libwayland-server": re.compile(r"libwayland-server\.so(?:\..*)?\Z"),
    "libwayland-client": re.compile(r"libwayland-client\.so(?:\..*)?\Z"),
    "libdrm": re.compile(r"libdrm\.so(?:\..*)?\Z"),
    "libinput": re.compile(r"libinput\.so(?:\..*)?\Z"),
    "libpixman": re.compile(r"libpixman-1\.so(?:\..*)?\Z"),
    "libxkbcommon": re.compile(r"libxkbcommon\.so(?:\..*)?\Z"),
    "libdisplay-info": re.compile(r"libdisplay-info\.so(?:\..*)?\Z"),
}
LIBRARY_FILE = re.compile(r"[^/]+\.so(?:\.[0-9]+)*\Z")
DECLARED_DOCS = {"changelog.Debian.gz", "copyright"}
PRIVATE_DEPENDENCIES = (
    re.compile(r"libwlroots(?:[-.0-9].*)?\Z"),
    re.compile(r"libwayland-(?:client|server)\d+\Z"),
    re.compile(r"libdrm\d+\Z"),
    re.compile(r"libdrm-(?:amdgpu|intel|nouveau|radeon)\d+\Z"),
    re.compile(r"libinput\d+\Z"),
    re.compile(r"libpixman-1-\d+\Z"),
    re.compile(r"libxkbcommon\d+\Z"),
    re.compile(r"libdisplay-info\d+\Z"),
)


def allowed_library(entry: Path, library_root: Path) -> bool:
    if entry.parent != library_root or not LIBRARY_FILE.fullmatch(entry.name):
        return False
    if not entry.is_symlink():
        return entry.is_file()
    try:
        resolved = entry.resolve(strict=True)
    except (OSError, RuntimeError):
        return False
    return resolved.parent == library_root.resolve() and resolved.is_file()


def dependency_names(value: str) -> list[str]:
    names = []
    for group in value.split(","):
        for alternative in group.split("|"):
            atom = alternative.strip().split(maxsplit=1)[0]
            names.append(atom.split(":", maxsplit=1)[0])
    return names


def field(package: Path, name: str) -> str:
    result = subprocess.run(
        ["dpkg-deb", "-f", str(package), name],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} PACKAGE.deb")
    package = Path(sys.argv[1]).resolve()
    expected = {
        "Package": "realm-river",
        "Version": "0.4.8-1",
        "Architecture": "amd64",
        "Provides": "river (= 0.4.8)",
    }
    for name, value in expected.items():
        if field(package, name) != value:
            raise SystemExit(f"unexpected package field: {name}")
    depends = field(package, "Depends")
    if "libinput-bin (>= 1.25.0)" not in depends:
        raise SystemExit("package dependency omits Noble libinput host data")
    for dependency in dependency_names(depends):
        if any(pattern.fullmatch(dependency) for pattern in PRIVATE_DEPENDENCIES):
            raise SystemExit(
                f"dependency on bundled private library is forbidden: {dependency}"
            )

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        subprocess.run(["dpkg-deb", "-x", str(package), str(root)], check=True)
        river = root / "usr/lib/realm/bin/river"
        if not river.is_file() or not river.stat().st_mode & 0o111:
            raise SystemExit("required private payload absent: river")
        seen: set[str] = set()
        quirks = 0
        library_root = root / "usr/lib/realm/lib"
        for entry in root.rglob("*"):
            relative = entry.relative_to(root)
            logical = "/" + relative.as_posix()
            if entry.is_dir() and not entry.is_symlink():
                continue
            if not (
                logical.startswith("/usr/lib/realm/")
                or logical.startswith("/usr/share/doc/realm-river/")
            ):
                raise SystemExit(f"payload path outside private runtime: {logical}")
            if (
                "/include/" in logical
                or logical.endswith((".pc", ".a", ".la"))
                or "/udev/" in logical
                or "/libexec/" in logical
            ):
                raise SystemExit(f"development or build payload is forbidden: {logical}")
            allowed = False
            if logical == "/usr/lib/realm/bin/river":
                allowed = entry.is_file() and not entry.is_symlink()
            elif allowed_library(entry, library_root):
                allowed = True
            elif (
                entry.parent == root / "usr/lib/realm/share/libinput"
                and entry.name.endswith(".quirks")
                and entry.is_file()
                and not entry.is_symlink()
            ):
                quirks += 1
                allowed = True
            elif (
                entry.parent == root / "usr/share/doc/realm-river"
                and entry.name in DECLARED_DOCS
                and entry.is_file()
                and not entry.is_symlink()
            ):
                allowed = True
            if not allowed:
                raise SystemExit(f"payload path is not an allowed runtime file: {logical}")
            if entry.parent == library_root:
                for family, pattern in REQUIRED.items():
                    if pattern.fullmatch(entry.name):
                        seen.add(family)
        for family in REQUIRED:
            if family not in seen:
                raise SystemExit(f"required private payload absent: {family}")
        if quirks == 0:
            raise SystemExit("required private payload absent: libinput quirks")


if __name__ == "__main__":
    main()
