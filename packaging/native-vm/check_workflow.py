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
    harness = "          packaging/native-vm/run-native-session-vm.sh"
    privileged_harness = re.compile(
        r"(?m)^\s*sudo\s+(?:--\S+\s+)*"
        r"(?:packaging/native-vm/run-native-session-vm\.sh|qemu-system)"
    )
    if privileged_harness.search(native):
        fail("native VM harness and QEMU must remain unprivileged")
    if harness not in native:
        fail("native VM job does not run the admitted KVM harness")
    if "            acl \\\n" not in native:
        fail("native VM job does not install its explicit ACL prerequisite")
    kvm_admission = """\
          if [[ -c /dev/kvm && (! -r /dev/kvm || ! -w /dev/kvm) ]]; then
            sudo setfacl -m "u:$(id -u):rw" /dev/kvm
          fi
"""
    if native.count("setfacl") != 1 or kvm_admission not in native:
        fail("native VM job does not use the scoped runner-UID KVM ACL")
    if native.index(kvm_admission) > native.index(harness):
        fail("runner-UID KVM ACL must precede the KVM harness")
    if re.search(r"(?m)^\s*(?:sudo\s+)?(?:chmod|chown)\s+.*?/dev/kvm", native):
        fail("native VM job must not change /dev/kvm ownership or mode")
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
