#!/usr/bin/env python3
"""Validate and stage the retained Helm workspace Cargo authority."""
import importlib.util
import sys
from pathlib import Path


def linkage_module():
    checker = Path(__file__).with_name("check-bundle-linkage.py")
    specification = importlib.util.spec_from_file_location("helm_bundle_linkage", checker)
    if specification is None or specification.loader is None:
        raise SystemExit("cannot load Helm bundle linkage checker")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: stage-helm-workspace.py BUNDLE DESTINATION")
    bundle = Path(sys.argv[1])
    destination = Path(sys.argv[2])
    source = linkage_module().validate_bundle(bundle, destination)
    print(source)


if __name__ == "__main__":
    main()
