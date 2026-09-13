#!/usr/bin/env python3
"""Enforce same-run artifacts and the no-rebuild native VM boundary."""

import re
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def job(source: str, name: str) -> str:
    match = re.search(
        rf"(?ms)^  {re.escape(name)}:\n(.*?)(?=^  [a-zA-Z0-9_-]+:\n|\Z)",
        source,
    )
    if match is None:
        fail(f"required workflow job is absent: {name}")
    return match.group(0)


def validate(path: Path) -> None:
    source = path.read_text(encoding="utf-8")
    ubuntu = job(source, "ubuntu-river-debian")
    fedora = job(source, "fedora-rpm-package")
    native = job(source, "native-session-vm")
    fast_contract = job(source, "fedora-baseline-contract")

    if "name: realm-native-ubuntu-${{ github.sha }}" not in ubuntu:
        fail("Ubuntu producer does not publish a commit-bound artifact")
    if "name: realm-native-fedora-${{ github.sha }}" not in fedora:
        fail("Fedora producer does not publish a commit-bound artifact")
    if "needs: [ubuntu-river-debian, fedora-rpm-package]" not in native:
        fail("native VM must depend on both native package producers")
    for target in ("ubuntu-24.04-x86_64", "fedora-44-x86_64"):
        if target not in native:
            fail(f"native VM matrix target is absent: {target}")
    for artifact in (
        "name: realm-native-ubuntu-${{ github.sha }}",
        "name: realm-native-fedora-${{ github.sha }}",
    ):
        if artifact not in native:
            fail(f"native VM does not download exact producer artifact: {artifact}")
    forbidden = re.compile(
        r"(?m)(?:^|[;&|\s])"
        r"(?:cargo|dpkg-buildpackage|rpmbuild|build-native-source-kits\.sh|"
        r"build-noble-river-source-kit\.sh)(?:\s|$)"
    )
    if forbidden.search(native):
        fail("native VM job contains a forbidden package build command")
    if "packaging/native-vm/run-native-session-vm.sh" not in native:
        fail("native VM job does not run the admitted KVM harness")
    if "if: always()" not in native or "realm-native-session-${{ matrix.target }}-${{ github.sha }}" not in native:
        fail("native VM failure evidence is not retained")
    if "python3 packaging/native-vm/check_workflow.py .github/workflows/distro.yml" not in fast_contract:
        fail("fast contract lane does not invoke the native workflow guard")


def main(arguments: list[str]) -> int:
    if len(arguments) != 1:
        print("usage: check_workflow.py WORKFLOW", file=sys.stderr)
        return 2
    try:
        validate(Path(arguments[0]))
    except (OSError, UnicodeError, ValueError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    print("native VM workflow contract passed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
