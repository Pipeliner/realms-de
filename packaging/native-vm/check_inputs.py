#!/usr/bin/env python3
"""Admit exact native-VM inputs and validate retained session evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable


CHECK_IDS = [
    "session/socket",
    "session/protocol-version",
    "session/degraded",
    "wm/attached",
    "wm/layer-shell",
    "wm/capabilities",
    "wm/protocol-version",
    "env/identity",
    "env/wayland-display/process",
    "env/wayland-display/systemd",
    "env/wayland-display/dbus",
    "env/desktop/systemd",
    "env/desktop/dbus",
    "env/agree",
    "env/stale",
    "env/cursor",
    "env/xwayland",
    "env/list-matches-entry",
    "units/target",
    "units/wm",
    "units/bar",
    "units/restart-policy",
    "units/idle-lock",
    "portal/answers",
    "portal/config",
    "portal/filechooser",
    "portal/screencast",
    "palette/lint",
    "theme/outputs",
    "fonts/glyphs",
    "fonts/attribution",
    "tools/floors",
]
SKIP_IDS = {"units/idle-lock", "portal/filechooser"}
ALLOWED_WARN_IDS = {
    "session/degraded",
    "wm/capabilities",
    "env/xwayland",
    "palette/lint",
    "theme/outputs",
    "fonts/glyphs",
    "fonts/attribution",
    "tools/floors",
}
REQUIRED_OK_IDS = {
    "session/socket",
    "session/protocol-version",
    "wm/attached",
    "wm/layer-shell",
    "portal/answers",
    "portal/config",
    "portal/screencast",
}
TARGETS = {"ubuntu-24.04-x86_64", "fedora-44-x86_64"}
HEX_SHA256 = re.compile(r"[0-9a-f]{64}")


def _load_json(path: Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read valid JSON from {path}: {error}") from error


def _target(manifest: Path, name: str) -> dict[str, str]:
    document = _load_json(manifest)
    if not isinstance(document, dict) or document.get("schema") != "realm-native-vm-images/v1":
        raise ValueError("unexpected native VM image manifest schema")
    targets = document.get("targets")
    if not isinstance(targets, dict):
        raise ValueError("native VM image manifest has no target inventory")
    selected = targets.get(name)
    if not isinstance(selected, dict) or set(selected) != {"url", "sha256"}:
        raise ValueError(f"native VM image target is malformed: {name}")
    url = selected.get("url")
    digest = selected.get("sha256")
    if not isinstance(url, str) or not url.startswith("https://"):
        raise ValueError(f"native VM image URL is not HTTPS: {name}")
    if not isinstance(digest, str) or HEX_SHA256.fullmatch(digest) is None:
        raise ValueError(f"native VM image SHA-256 is malformed: {name}")
    return {"url": url, "sha256": digest}


def validate_manifest(manifest: Path) -> None:
    """Require the production authority to name only the accepted matrix."""

    document = _load_json(manifest)
    if not isinstance(document, dict) or document.get("schema") != "realm-native-vm-images/v1":
        raise ValueError("unexpected native VM image manifest schema")
    targets = document.get("targets")
    if not isinstance(targets, dict) or set(targets) != TARGETS:
        raise ValueError("native VM image manifest has an unexpected target inventory")
    for name in sorted(TARGETS):
        _target(manifest, name)


def image_url(manifest: Path, name: str) -> str:
    """Return the admitted HTTPS URL for one closed-set target."""

    return _target(manifest, name)["url"]


def verify_image(manifest: Path, name: str, image: Path) -> dict[str, str]:
    """Hash one regular image before any VM operation and return its authority."""

    selected = _target(manifest, name)
    if image.is_symlink() or not image.is_file():
        raise ValueError(f"native VM image is not a regular file: {image}")
    digest = hashlib.sha256()
    with image.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    actual = digest.hexdigest()
    if actual != selected["sha256"]:
        raise ValueError(
            f"image SHA-256 mismatch for {name}: expected {selected['sha256']}, got {actual}"
        )
    return selected


def _deb_facts(path: Path) -> dict[str, str]:
    output = subprocess.run(
        [
            "dpkg-deb",
            "--showformat=${Package}\\n${Version}\\n${Architecture}\\n",
            "--show",
            str(path),
        ],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ).stdout.splitlines()
    if len(output) != 3:
        raise ValueError(f"cannot read Debian package identity: {path}")
    return {"name": output[0], "version": output[1], "arch": output[2]}


def _rpm_facts(path: Path) -> dict[str, str]:
    output = subprocess.run(
        ["rpm", "-qp", "--queryformat", "%{NAME}\n%{VERSION}-%{RELEASE}\n%{ARCH}\n", str(path)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ).stdout.splitlines()
    if len(output) != 3:
        raise ValueError(f"cannot read RPM package identity: {path}")
    return {"name": output[0], "version": output[1], "arch": output[2]}


def select_packages(
    target: str,
    artifact_dir: Path,
    facts_for: Callable[[Path], dict[str, str]] | None = None,
) -> list[Path]:
    """Return the exact package set after format, count and payload checks."""

    if artifact_dir.is_symlink() or not artifact_dir.is_dir():
        raise ValueError(f"artifact directory is unavailable: {artifact_dir}")
    inventory = list(artifact_dir.iterdir())
    if any(path.is_symlink() or not path.is_file() for path in inventory):
        raise ValueError("artifact inventory must contain only regular files")
    entries = sorted(inventory)
    if target == "ubuntu-24.04-x86_64":
        packages = [path for path in entries if path.suffix == ".deb"]
        if len(entries) != 2 or len(packages) != 2:
            raise ValueError("Ubuntu artifact must contain exactly two Debian packages")
        reader = facts_for or _deb_facts
        facts = sorted(((reader(path), path) for path in packages), key=lambda item: item[0]["name"])
        identities = [(item[0]["name"], item[0]["version"], item[0]["arch"]) for item in facts]
        expected = [("realm", "0.1.0", "amd64"), ("realm-river", "0.4.8-1", "amd64")]
        if identities != expected:
            raise ValueError(f"unexpected Debian package identity: {identities}")
        return [item[1] for item in facts]
    if target == "fedora-44-x86_64":
        packages = [path for path in entries if path.suffix == ".rpm"]
        if len(entries) != 1 or len(packages) != 1:
            raise ValueError("Fedora artifact must contain exactly one RPM package")
        facts = (facts_for or _rpm_facts)(packages[0])
        if facts.get("name") != "realm" or facts.get("arch") != "x86_64":
            raise ValueError(f"unexpected RPM package identity: {facts}")
        return packages
    raise ValueError(f"unknown native VM target: {target}")


def validate_doctor(report: object) -> None:
    """Require the healthy installed-session subset and exact declared skips."""

    if not isinstance(report, dict) or not isinstance(report.get("checks"), list):
        raise ValueError("doctor report has no checks array")
    checks = report["checks"]
    if [check.get("id") for check in checks if isinstance(check, dict)] != CHECK_IDS:
        raise ValueError("doctor check IDs are absent or out of order")
    unknown_statuses = sorted(
        repr(check.get("status"))
        for check in checks
        if check.get("status") not in ("ok", "warn", "fail", "skip")
    )
    if unknown_statuses:
        raise ValueError(f"doctor contains unknown status values: {sorted(unknown_statuses)}")
    if any(check.get("status") == "fail" for check in checks):
        raise ValueError("doctor contains failed checks")
    skipped = {check["id"] for check in checks if check.get("status") == "skip"}
    if skipped != SKIP_IDS:
        raise ValueError(f"doctor skip set mismatch: {sorted(skipped)}")
    by_id = {check["id"]: check for check in checks}
    warned = {check["id"] for check in checks if check.get("status") == "warn"}
    unexpected_warnings = sorted(warned - ALLOWED_WARN_IDS)
    if unexpected_warnings:
        raise ValueError(f"unexpected doctor warnings: {unexpected_warnings}")
    tools = by_id["tools/floors"]
    tools_summary = tools.get("summary")
    btop = None
    if isinstance(tools_summary, str):
        match = re.fullmatch(
            r"yazi: not found, btop: (.*), starship: not found; "
            r"install the missing or unparseable tools",
            tools_summary,
            flags=re.DOTALL,
        )
        if match is not None:
            btop = match.group(1)
    if (
        tools.get("status") != "warn"
        or btop is None
        or not any(character.isdigit() for character in btop)
        or "not found" in btop
        or "unparseable" in btop
    ):
        raise ValueError(
            "doctor native tool evidence does not match the accepted package boundary"
        )
    not_ok = sorted(check_id for check_id in REQUIRED_OK_IDS if by_id[check_id].get("status") != "ok")
    if not_ok:
        raise ValueError(f"required doctor checks are not ok: {not_ok}")


def validate_control_frames(frames: object) -> None:
    """Require one protocol-v2 Hello followed by a real state response."""

    if not isinstance(frames, list) or len(frames) != 2:
        raise ValueError("control evidence must contain exactly two frames")
    hello, state = frames
    if (
        not isinstance(hello, dict)
        or hello.get("reply") != "hello"
        or not isinstance(hello.get("data"), dict)
        or hello["data"].get("version") != 2
    ):
        raise ValueError("control evidence did not complete protocol-v2 Hello")
    if not isinstance(state, dict) or state.get("reply") != "state" or not isinstance(state.get("data"), dict):
        raise ValueError("control evidence did not return GetState")


def validate_framebuffer(path: Path) -> None:
    """Require a complete QEMU P6 frame with both Realm UI bands painted."""

    if path.is_symlink() or not path.is_file():
        raise ValueError(f"framebuffer is not a regular file: {path}")
    try:
        with path.open("rb") as source:
            if source.readline().rstrip(b"\r\n") != b"P6":
                raise ValueError("framebuffer is not a P6 image")
            dimensions = source.readline().split()
            if len(dimensions) != 2:
                raise ValueError("framebuffer dimensions are malformed")
            width, height = (int(value) for value in dimensions)
            if width <= 0 or height < 128:
                raise ValueError("framebuffer dimensions are too small")
            if source.readline().rstrip(b"\r\n") != b"255":
                raise ValueError("framebuffer is not 8-bit RGB")
            pixels = source.read()
    except (OSError, UnicodeError, ValueError) as error:
        if isinstance(error, ValueError) and str(error).startswith("framebuffer"):
            raise
        raise ValueError(f"cannot read framebuffer {path}: {error}") from error

    expected_length = width * height * 3
    if len(pixels) != expected_length:
        raise ValueError(
            f"framebuffer payload length mismatch: expected {expected_length}, got {len(pixels)}"
        )

    band_pixels = width * 64
    required_non_black = (band_pixels + 7) // 8

    def count_non_black(payload: bytes) -> int:
        return sum(
            payload[index] != 0
            or payload[index + 1] != 0
            or payload[index + 2] != 0
            for index in range(0, len(payload), 3)
        )

    band_bytes = band_pixels * 3
    top_non_black = count_non_black(pixels[:band_bytes])
    bottom_non_black = count_non_black(pixels[-band_bytes:])
    if top_non_black < required_non_black or bottom_non_black < required_non_black:
        raise ValueError(
            "unpainted framebuffer: "
            f"top band has {top_non_black}/{band_pixels} non-black pixels, "
            f"bottom band has {bottom_non_black}/{band_pixels}; "
            f"each requires {required_non_black}"
        )


def _main(arguments: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest", type=Path, default=Path(__file__).with_name("images.json"))
    subparsers = parser.add_subparsers(dest="command", required=True)
    url_parser = subparsers.add_parser("url")
    url_parser.add_argument("target")
    image_parser = subparsers.add_parser("image")
    image_parser.add_argument("target")
    image_parser.add_argument("path", type=Path)
    package_parser = subparsers.add_parser("packages")
    package_parser.add_argument("target")
    package_parser.add_argument("directory", type=Path)
    doctor_parser = subparsers.add_parser("doctor")
    doctor_parser.add_argument("path", type=Path)
    control_parser = subparsers.add_parser("control")
    control_parser.add_argument("path", type=Path)
    framebuffer_parser = subparsers.add_parser("framebuffer")
    framebuffer_parser.add_argument("path", type=Path)
    options = parser.parse_args(arguments)

    if options.command == "url":
        validate_manifest(options.manifest)
        print(image_url(options.manifest, options.target))
    elif options.command == "image":
        validate_manifest(options.manifest)
        verify_image(options.manifest, options.target, options.path)
    elif options.command == "packages":
        for package in select_packages(options.target, options.directory):
            print(package)
    elif options.command == "doctor":
        validate_doctor(_load_json(options.path))
    elif options.command == "control":
        try:
            frames = [json.loads(line) for line in options.path.read_text(encoding="utf-8").splitlines()]
        except (OSError, UnicodeError, json.JSONDecodeError) as error:
            raise ValueError(f"cannot read control evidence: {error}") from error
        validate_control_frames(frames)
    elif options.command == "framebuffer":
        validate_framebuffer(options.path)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(_main(sys.argv[1:]))
    except (ValueError, subprocess.CalledProcessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
