#!/usr/bin/env python3
"""Reject any selected private-library family resolved outside its prefix."""

from __future__ import annotations

import re
import sys
from pathlib import Path

LINE = re.compile(r"^\s*(lib[^\s]+)\s+=>\s+(\S+)")
PRIVATE = (
    re.compile(r"libwlroots-0\.20\.so(?:\..*)?\Z"),
    re.compile(r"libwayland-(?:server|client)\.so(?:\..*)?\Z"),
    re.compile(r"libdrm(?:_[A-Za-z0-9_-]+)?\.so(?:\..*)?\Z"),
    re.compile(r"libinput\.so(?:\..*)?\Z"),
    re.compile(r"libpixman-1\.so(?:\..*)?\Z"),
    re.compile(r"libxkbcommon\.so(?:\..*)?\Z"),
    re.compile(r"libdisplay-info\.so(?:\..*)?\Z"),
)
REQUIRED = {
    "libwlroots": re.compile(r"libwlroots-0\.20\.so(?:\..*)?\Z"),
    "libwayland-server": re.compile(r"libwayland-server\.so(?:\..*)?\Z"),
    "libdrm": re.compile(r"libdrm\.so(?:\..*)?\Z"),
    "libinput": re.compile(r"libinput\.so(?:\..*)?\Z"),
    "libpixman": re.compile(r"libpixman-1\.so(?:\..*)?\Z"),
    "libxkbcommon": re.compile(r"libxkbcommon\.so(?:\..*)?\Z"),
    "libdisplay-info": re.compile(r"libdisplay-info\.so(?:\..*)?\Z"),
}


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} PRIVATE-PREFIX LDD-LOG", file=sys.stderr)
        return 2
    libdir = (Path(sys.argv[1]).resolve() / "lib").resolve()
    log = Path(sys.argv[2])
    seen: set[str] = set()
    try:
        lines = log.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        print(f"private resolution: cannot read ldd log: {error}", file=sys.stderr)
        return 1

    for line in lines:
        match = LINE.match(line)
        if match is None:
            continue
        soname, destination = match.groups()
        if not any(pattern.fullmatch(soname) for pattern in PRIVATE):
            continue
        for family, pattern in REQUIRED.items():
            if pattern.fullmatch(soname):
                seen.add(family)
        resolved = Path(destination).resolve(strict=False)
        if not resolved.is_relative_to(libdir):
            print(
                f"private resolution: private-family resolution escaped prefix: {soname}",
                file=sys.stderr,
            )
            return 1

    for family in REQUIRED:
        if family not in seen:
            print(f"private resolution: required private family absent: {family}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
