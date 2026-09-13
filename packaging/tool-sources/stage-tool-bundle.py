#!/usr/bin/env python3
"""Validate and stage one selected Realm-owned native tool bundle."""
import importlib.util
import sys
from pathlib import Path


SELECTED = {("yazi", "25.4.8"), ("starship", "1.23.0")}


def linkage_module():
    # A source kit is a digest-checked input, not a Python cache directory.
    # Import the copied validator without mutating that strict inventory.
    sys.dont_write_bytecode = True
    checker = Path(__file__).with_name("check-bundle-linkage.py")
    specification = importlib.util.spec_from_file_location("realm_bundle_linkage", checker)
    if specification is None or specification.loader is None:
        raise SystemExit("cannot load Realm bundle linkage checker")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: stage-tool-bundle.py BUNDLE DESTINATION")
    bundle = Path(sys.argv[1])
    destination = Path(sys.argv[2])
    linkage = linkage_module()
    record = linkage.quoted_record(bundle / "bundle.toml")
    identity = (record.get("name"), record.get("version"))
    if identity not in SELECTED:
        raise SystemExit("bundle is not a selected native tool version")
    source = linkage.validate_bundle(bundle, destination)
    print(source)


if __name__ == "__main__":
    main()
