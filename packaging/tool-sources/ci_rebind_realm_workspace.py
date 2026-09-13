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
    commit_value: str, output_value: Path, repository_value: Path
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
    require_same_lockfile((bundle / "Cargo.lock").read_bytes(), committed_lock)

    shutil.copytree(bundle, output)
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
    arguments = parser.parse_args()
    print(
        json.dumps(
            produce(arguments.commit, arguments.output, arguments.repo),
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
