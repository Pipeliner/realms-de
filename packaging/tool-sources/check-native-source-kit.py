#!/usr/bin/env python3
"""Reject native package source inputs that contain a second workspace tree."""
import sys
from pathlib import Path


TOOL_SOURCE_ENTRIES = {
    "bundles",
    "check-bundle-linkage.py",
    "check-native-source-kit.py",
    "stage-helm-workspace.py",
}


def names(path: Path) -> set[str]:
    if path.is_symlink() or not path.is_dir():
        return set()
    return {entry.name for entry in path.iterdir()}


def require_inventory(path: Path, expected: set[str], message: str) -> None:
    if names(path) != expected:
        raise SystemExit(message)


def main() -> None:
    if len(sys.argv) != 3 or sys.argv[1] not in {"debian", "rpm"}:
        raise SystemExit("usage: check-native-source-kit.py debian|rpm ROOT")
    kind = sys.argv[1]
    root = Path(sys.argv[2]).resolve()
    if kind == "debian":
        require_inventory(
            root,
            {"debian", "packaging"},
            "Debian source kit top-level inventory differs from policy",
        )
        packaging_entries = {"tool-sources"}
    else:
        require_inventory(
            root,
            {"packaging"},
            "RPM source kit top-level inventory differs from policy",
        )
        packaging_entries = {"fedora", "tool-sources"}
    require_inventory(
        root / "packaging",
        packaging_entries,
        f"{kind.upper()} source kit packaging inventory differs from policy",
    )
    require_inventory(
        root / "packaging" / "tool-sources",
        TOOL_SOURCE_ENTRIES,
        f"{kind.upper()} source kit helper inventory differs from policy",
    )
    require_inventory(
        root / "packaging" / "tool-sources" / "bundles",
        {"helm-workspace"},
        f"{kind.upper()} source kit bundle inventory differs from policy",
    )
    for entry in root.rglob("*"):
        if entry.is_symlink() or not (entry.is_file() or entry.is_dir()):
            raise SystemExit(f"{kind.upper()} source kit contains unsafe filesystem entry")


if __name__ == "__main__":
    main()
