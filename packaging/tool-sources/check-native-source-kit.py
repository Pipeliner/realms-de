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
FORBIDDEN_WORKSPACE_FILES = {"Cargo.toml"}
FORBIDDEN_WORKSPACE_DIRECTORIES = {".cargo", "crates", "vendor"}


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
        packaging_entries = {"package-docs", "tool-sources"}
    else:
        require_inventory(
            root,
            {"packaging"},
            "RPM source kit top-level inventory differs from policy",
        )
        packaging_entries = {"fedora", "package-docs", "tool-sources"}
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
        root / "packaging" / "package-docs",
        {"INSTALL.md"},
        f"{kind.upper()} source kit package-document inventory differs from policy",
    )
    require_inventory(
        root / "packaging" / "tool-sources" / "bundles",
        {"helm-workspace"},
        f"{kind.upper()} source kit bundle inventory differs from policy",
    )
    canonical_bundle = (
        root / "packaging" / "tool-sources" / "bundles" / "helm-workspace"
    )
    allowed_named_files = {
        canonical_bundle / "Cargo.lock",
        canonical_bundle / "source.tar.gz",
    }
    for entry in root.rglob("*"):
        if entry.is_symlink() or not (entry.is_file() or entry.is_dir()):
            raise SystemExit(f"{kind.upper()} source kit contains unsafe filesystem entry")
        forbidden = entry.name in FORBIDDEN_WORKSPACE_FILES
        forbidden = forbidden or (
            entry.is_dir() and entry.name in FORBIDDEN_WORKSPACE_DIRECTORIES
        )
        forbidden = forbidden or (
            entry.name in {"Cargo.lock", "source.tar.gz"}
            and entry not in allowed_named_files
        )
        if forbidden:
            relative = entry.relative_to(root)
            raise SystemExit(
                f"{kind.upper()} source kit contains forbidden workspace marker: "
                f"{relative}"
            )


if __name__ == "__main__":
    main()
