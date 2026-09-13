#!/usr/bin/env python3
"""Relocate only staged private pkg-config paths for the Noble package build."""

from __future__ import annotations

import re
import sys
from pathlib import Path


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} LOGICAL-PREFIX STAGED-PREFIX")

    logical_prefix = sys.argv[1]
    if logical_prefix != "/usr/lib/realm":
        raise SystemExit(f"unexpected logical prefix: {logical_prefix}")
    staged_prefix = Path(sys.argv[2]).resolve(strict=True)
    path_token = re.compile(
        rf"(?P<lead>^|[=:\s]|-[IL]){re.escape(logical_prefix)}(?=$|[/\s])",
        re.MULTILINE,
    )

    for relative in ("lib/pkgconfig", "share/pkgconfig"):
        directory = staged_prefix / relative
        if not directory.exists():
            continue
        for record in directory.glob("*.pc"):
            text = record.read_text(encoding="utf-8")
            relocated = path_token.sub(
                lambda match: match.group("lead") + str(staged_prefix), text
            )
            if relocated != text:
                record.write_text(relocated, encoding="utf-8")


if __name__ == "__main__":
    main()
