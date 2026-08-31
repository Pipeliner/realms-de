#!/usr/bin/env python3
"""Validate retained Yazi and Starship source inputs without network access."""
import hashlib
import stat
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else Path(__file__).resolve().parent
records = []
record = None
for raw in (root / "manifest.toml").read_text().splitlines():
    line = raw.strip()
    if not line or line.startswith("#"):
        continue
    if line == "[[input]]":
        if record is not None:
            records.append(record)
        record = {}
        continue
    if record is None or " = " not in line:
        raise SystemExit("manifest has unsupported TOML syntax")
    key, value = line.split(" = ", 1)
    if not (value.startswith('"') and value.endswith('"')):
        raise SystemExit("manifest values must be quoted strings")
    record[key] = value[1:-1]
if record is not None:
    records.append(record)
required = {"tool", "version", "source", "sha256", "source_url", "license", "notice", "intake_date", "provenance", "rollback_to"}
errors = []
if {record.get("tool") for record in records} != {"yazi", "starship"} or len(records) != 2:
    errors.append("manifest must contain exactly one yazi and one starship input")
seen = set()
declared_inputs = set()
declared_notices = set()
for record in records:
    if set(record) != required:
        errors.append(f"{record.get('tool', '?')}: record fields differ from policy")
        continue
    key = (record["tool"], record["version"])
    if key in seen:
        errors.append(f"{record['tool']}: duplicate version")
    seen.add(key)
    source_path = Path(record["source"])
    notice_path = Path(record["notice"])
    if source_path.parent != Path("inputs") or notice_path.parent != Path("notices"):
        errors.append(f"{record['tool']}: retained paths must be direct inputs or notices")
        continue
    source = root / source_path
    notice = root / notice_path
    declared_inputs.add(source_path)
    declared_notices.add(notice_path)
    if source.is_symlink() or not source.exists() or not stat.S_ISREG(source.stat(follow_symlinks=False).st_mode):
        errors.append(f"{record['tool']}: retained source must be a regular non-symlink file")
        continue
    if notice.is_symlink() or not notice.exists() or not stat.S_ISREG(notice.stat(follow_symlinks=False).st_mode):
        errors.append(f"{record['tool']}: notice must be a regular non-symlink file")
        continue
    if not notice.read_text().strip():
        errors.append(f"{record['tool']}: missing retained source or notice")
        continue
    digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if digest != record["sha256"]:
        errors.append(f"{record['tool']}: SHA-256 mismatch")
    if not record["source_url"].startswith("https://") or record["rollback_to"] != "none":
        errors.append(f"{record['tool']}: invalid provenance or initial rollback relation")
for directory, declared in ((Path("inputs"), declared_inputs), (Path("notices"), declared_notices)):
    actual = {path.relative_to(root) for path in (root / directory).iterdir()}
    if actual != declared:
        errors.append("retained input inventory differs from manifest")
if errors:
    print("\n".join(errors), file=sys.stderr)
    raise SystemExit(1)
