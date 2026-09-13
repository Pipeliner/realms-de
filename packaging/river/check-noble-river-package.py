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
    if "libinput-bin (>= 1.25.0)" not in field(package, "Depends"):
        raise SystemExit("package dependency omits Noble libinput host data")

    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        subprocess.run(["dpkg-deb", "-x", str(package), str(root)], check=True)
        river = root / "usr/lib/realm/bin/river"
        if not river.is_file() or not river.stat().st_mode & 0o111:
            raise SystemExit("required private payload absent: river")
        seen: set[str] = set()
        quirks = 0
        for entry in root.rglob("*"):
            relative = entry.relative_to(root)
            logical = "/" + relative.as_posix()
            if entry.is_dir():
                continue
            if logical.startswith("/usr/share/doc/realm-river/"):
                continue
            if not logical.startswith("/usr/lib/realm/"):
                raise SystemExit(f"payload path outside private runtime: {logical}")
            if "/include/" in logical or logical.endswith((".pc", ".a", ".la")):
                raise SystemExit(f"development or build payload is forbidden: {logical}")
            if logical.startswith("/usr/lib/realm/bin/") and logical != "/usr/lib/realm/bin/river":
                raise SystemExit(f"development or build payload is forbidden: {logical}")
            if "/udev/" in logical or "/libexec/" in logical:
                raise SystemExit(f"development or build payload is forbidden: {logical}")
            if logical.startswith("/usr/lib/realm/share/libinput/") and logical.endswith(".quirks"):
                quirks += 1
            if logical.startswith("/usr/lib/realm/lib/"):
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
