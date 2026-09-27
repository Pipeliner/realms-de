#!/usr/bin/env python3
"""Produce a Realm workspace source-binding candidate in GitHub Actions."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path


BUNDLE_RELATIVE = Path("packaging/tool-sources/bundles/realm-workspace")
FULL_COMMIT = re.compile(r"[0-9a-f]{40}")


def normalize_commit(value: str) -> str:
    """Return one exact lowercase full commit ID or reject it."""
    if FULL_COMMIT.fullmatch(value) is None:
        raise ValueError("source commit must be exactly 40 lowercase hexadecimal digits")
    return value


def _replace_unique(text: str, pattern: str, replacement: str, field: str) -> str:
    updated, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != 1:
        raise ValueError(f"expected exactly one {field} field, found {count}")
    return updated


def update_manifest(
    text: str,
    *,
    commit: str,
    timestamp: str,
    source_sha256: str,
    provenance_sha256: str,
) -> str:
    """Update only source-identity fields in one existing bundle record."""
    fields = {
        "commit": commit,
        "commit_timestamp": timestamp,
        "source_sha256": source_sha256,
        "source_provenance_sha256": provenance_sha256,
    }
    for field, value in fields.items():
        text = _replace_unique(
            text,
            rf'^{field} = "[^"]*"$',
            f'{field} = "{value}"',
            field,
        )
    return text


def update_provenance(
    text: str,
    *,
    commit: str,
    timestamp: str,
    epoch: str,
    source_sha256: str,
) -> str:
    """Update only the bound source identity in existing provenance prose."""
    text = _replace_unique(
        text,
        r"^- Commit: `[0-9a-f]{40}`$",
        f"- Commit: `{commit}`",
        "provenance commit",
    )
    text = _replace_unique(
        text,
        r"^- Commit timestamp: `[^`]+` \(`[0-9]+`\)$",
        f"- Commit timestamp: `{timestamp}` (`{epoch}`)",
        "provenance commit timestamp",
    )
    return _replace_unique(
        text,
        r"(?m)^(  `)[0-9a-f]{64}(`)$",
        rf"\g<1>{source_sha256}\g<2>",
        "canonical archive SHA-256",
    )


def require_same_lockfile(retained: bytes, committed: bytes) -> None:
    """Keep dependency changes on the separate controlled closure path."""
    if retained != committed:
        raise ValueError(
            "source Cargo.lock changed; a controlled dependency-closure refresh is required"
        )


def require_workspace_edge_refresh(retained: bytes, committed: bytes) -> None:
    """Accept only dependency-list changes in existing workspace packages."""
    def records(raw: bytes):
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ValueError("unsupported lockfile encoding") from error
        parts = text.split("[[package]]\n")
        if not re.fullmatch(r'(?:#[^\n]*\n|\s)*version = [34]\n\s*', parts[0]):
            raise ValueError("unsupported lockfile header")
        packages = {}
        for part in parts[1:]:
            match = re.fullmatch(
                r'name = "([^"\n]+)"\nversion = "([^"\n]+)"\n'
                r'(?:source = "([^"\n]+)"\n)?'
                r'(?:checksum = "[0-9a-f]{64}"\n)?'
                r'(?:dependencies = \[\n(?: "[^"\n]+",\n)*\]\n)?\s*', part
            )
            if match is None:
                raise ValueError("unsupported lockfile package syntax")
            key = match.group(1, 2, 3)
            if key in packages:
                raise ValueError("duplicate lockfile package")
            # External records remain byte-exact. Only local dependency lists
            # are removed for comparison; identities and other fields remain.
            packages[key] = part if key[2] else re.sub(
                r'dependencies = \[\n(?: "[^"\n]+",\n)*\]\n', '', part
            )
        if not packages:
            raise ValueError("lockfile has no packages")
        return parts[0], packages

    if records(retained) != records(committed):
        raise ValueError("workspace refresh requires unchanged external records and package identities")


def require_payload_digest(manifest: str, field: str, payload: bytes) -> None:
    """Verify retained closure bytes against their unique manifest binding."""
    expected = re.findall(rf'^{field}_sha256 = "([0-9a-f]{{64}})"$', manifest, re.MULTILINE)
    if len(expected) != 1 or hashlib.sha256(payload).hexdigest() != expected[0]:
        raise ValueError(f"retained {field} digest mismatch")


def _git(
    repository: Path, *arguments: str, stdout: int | None = None
) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        stdout=stdout,
    )


def _require_ci_output(path: Path) -> Path:
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise RuntimeError("Realm workspace source rebinding is available only in CI")
    runner = os.environ.get("RUNNER_TEMP")
    if not runner:
        raise RuntimeError("RUNNER_TEMP is required in CI")
    output = path.resolve()
    try:
        output.relative_to(Path(runner).resolve())
    except ValueError as error:
        raise RuntimeError("rebind output must remain below RUNNER_TEMP") from error
    if output.exists():
        raise RuntimeError("rebind output must not already exist")
    return output


def produce(
    commit_value: str, output_value: Path, repository_value: Path,
    *, refresh_workspace_lock: bool = False,
) -> dict[str, str]:
    """Create and record one candidate bundle without changing the checkout."""
    commit = normalize_commit(commit_value)
    output = _require_ci_output(output_value)
    repository = repository_value.resolve()
    bundle = repository / BUNDLE_RELATIVE
    resolved = _git(
        repository,
        "rev-parse",
        "--verify",
        f"{commit}^{{commit}}",
        stdout=subprocess.PIPE,
    )
    if resolved.stdout.decode().strip() != commit:
        raise ValueError("source commit did not resolve to the exact requested object")
    checkout = _git(repository, "rev-parse", "HEAD", stdout=subprocess.PIPE)
    if checkout.stdout.decode().strip() != commit:
        raise ValueError("source checkout is not the exact requested commit")

    committed_lock = _git(
        repository, "show", f"{commit}:Cargo.lock", stdout=subprocess.PIPE
    ).stdout
    retained_lock = (bundle / "Cargo.lock").read_bytes()
    if refresh_workspace_lock:
        # The retained authority must itself come from this exact commit.
        _git(repository, "diff", "--exit-code", commit, "--", str(BUNDLE_RELATIVE))
        manifest_text = (bundle / "bundle.toml").read_text(encoding="utf-8")
        for field, filename in (
            ("lockfile", "Cargo.lock"), ("vendor_archive", "vendor.tar.zst"),
            ("cargo_config", "config.toml"), ("license_report", "licenses.tsv"),
            ("source", "source.tar.gz"), ("source_provenance", "provenance.md"),
        ):
            require_payload_digest(manifest_text, field, (bundle / filename).read_bytes())
        require_workspace_edge_refresh(retained_lock, committed_lock)
    else:
        require_same_lockfile(retained_lock, committed_lock)

    shutil.copytree(bundle, output)
    if refresh_workspace_lock:
        (output / "Cargo.lock").write_bytes(committed_lock)
    archive = output / "source.tar.gz"
    _git(
        repository,
        "archive",
        "--format=tar.gz",
        "--prefix=realm-workspace/",
        f"--output={archive}",
        commit,
        ".",
        ":(exclude)packaging/tool-sources/bundles",
    )
    source_sha256 = hashlib.sha256(archive.read_bytes()).hexdigest()

    metadata = _git(
        repository,
        "show",
        "-s",
        "--format=%cI%n%ct",
        commit,
        stdout=subprocess.PIPE,
    ).stdout.decode().splitlines()
    if len(metadata) != 2:
        raise RuntimeError("source commit metadata has an unexpected shape")
    committed_at = dt.datetime.fromisoformat(metadata[0]).astimezone(dt.timezone.utc)
    timestamp = committed_at.strftime("%Y-%m-%dT%H:%M:%SZ")
    epoch = metadata[1]

    provenance = output / "provenance.md"
    provenance.write_text(
        update_provenance(
            provenance.read_text(encoding="utf-8"),
            commit=commit,
            timestamp=timestamp,
            epoch=epoch,
            source_sha256=source_sha256,
        ),
        encoding="utf-8",
    )
    provenance_sha256 = hashlib.sha256(provenance.read_bytes()).hexdigest()

    if refresh_workspace_lock:
        provenance.write_text(
            provenance.read_text(encoding="utf-8")
            + "\n## CI workspace dependency-edge refresh\n\n"
            + f"Commit `{commit}` refreshed workspace dependency lists only. "
            + "External package records and retained vendor, configuration and "
            + "license-report bytes are unchanged and digest-verified.\n"
            + f"- Prior lock SHA-256: `{hashlib.sha256(retained_lock).hexdigest()}`\n"
            + f"- Refreshed lock SHA-256: `{hashlib.sha256(committed_lock).hexdigest()}`\n",
            encoding="utf-8",
        )
        provenance_sha256 = hashlib.sha256(provenance.read_bytes()).hexdigest()

    manifest = output / "bundle.toml"
    manifest.write_text(
        update_manifest(
            manifest.read_text(encoding="utf-8"),
            commit=commit,
            timestamp=timestamp,
            source_sha256=source_sha256,
            provenance_sha256=provenance_sha256,
        ),
        encoding="utf-8",
    )
    if refresh_workspace_lock:
        manifest.write_text(_replace_unique(
            manifest.read_text(encoding="utf-8"),
            r'^lockfile_sha256 = "[0-9a-f]{64}"$',
            f'lockfile_sha256 = "{hashlib.sha256(committed_lock).hexdigest()}"',
            "lockfile digest",
        ), encoding="utf-8")
        for field, filename in (
            ("vendor_archive", "vendor.tar.zst"), ("cargo_config", "config.toml"),
            ("license_report", "licenses.tsv"),
        ):
            require_payload_digest(manifest_text, field, (output / filename).read_bytes())
    return {
        "commit": commit,
        "commit_timestamp": timestamp,
        "source_sha256": source_sha256,
        "source_provenance_sha256": provenance_sha256,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repo", required=True, type=Path)
    parser.add_argument("--refresh-workspace-lock", action="store_true")
    arguments = parser.parse_args()
    print(
        json.dumps(
            produce(arguments.commit, arguments.output, arguments.repo,
                    refresh_workspace_lock=arguments.refresh_workspace_lock),
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
