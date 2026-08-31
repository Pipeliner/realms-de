#!/usr/bin/env python3
"""Validate one retained Cargo bundle without consulting the network."""
import hashlib
import subprocess
import sys
import tempfile
from pathlib import Path


def quoted_record(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("["):
            continue
        if " = " not in line:
            raise SystemExit("bundle manifest has unsupported TOML syntax")
        key, value = line.split(" = ", 1)
        if not (value.startswith('"') and value.endswith('"')):
            raise SystemExit("bundle manifest values must be quoted strings")
        values[key] = value[1:-1]
    return values


root = Path(sys.argv[1]).resolve() if len(sys.argv) == 2 else Path(__file__).resolve().parent
record = quoted_record(root / "bundle.toml")
required = {"name", "lockfile", "cargo_config", "license_report"}
vendor_fields = {"vendor"}
archive_fields = {"vendor_archive", "vendor_archive_sha256", "vendor_archive_format"}
bound_fields = {
    "version", "commit", "commit_timestamp", "source", "source_sha256",
    "lockfile_sha256", "cargo_config_sha256", "license_report_sha256",
}
basic_fields = required | vendor_fields
basic_archive_fields = required | archive_fields
bound_archive_fields = basic_archive_fields | bound_fields
if set(record) != basic_fields and set(record) != basic_archive_fields and set(record) != bound_archive_fields:
    raise SystemExit("bundle manifest fields differ from policy")

paths = {key: root / record[key] for key in required - {"name"}}
if "source" in record:
    paths["source"] = root / record["source"]
for key, path in paths.items():
    if not path.exists() or path.is_symlink():
        raise SystemExit(f"bundle {key} is missing or symlinked")
for key in ("source", "lockfile", "cargo_config", "license_report"):
    hash_key = f"{key}_sha256"
    if hash_key in record and hashlib.sha256(paths[key].read_bytes()).hexdigest() != record[hash_key]:
        raise SystemExit(f"{key} SHA-256 mismatch")

config = paths["cargo_config"].read_text()
temporary = None
if "vendor" in record:
    vendor = root / record["vendor"]
else:
    archive = root / record["vendor_archive"]
    if archive.is_symlink() or not archive.is_file():
        raise SystemExit("vendor archive is missing or symlinked")
    if record["vendor_archive_format"] != "tar.zst":
        raise SystemExit("vendor archive format is not tar.zst")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != record["vendor_archive_sha256"]:
        raise SystemExit("vendor archive SHA-256 mismatch")
    temporary = tempfile.TemporaryDirectory()
    unpacked = Path(temporary.name)
    subprocess.run(["tar", "--zstd", "-xf", str(archive), "-C", str(unpacked)], check=True)
    vendor = unpacked / "vendor"
    if not vendor.is_dir() or vendor.is_symlink():
        raise SystemExit("vendor archive did not unpack a regular vendor tree")
if 'replace-with = "vendored-sources"' not in config or 'directory = "vendor"' not in config:
    raise SystemExit("Cargo source replacement does not select vendor directory")
if not any(vendor.glob("*/.cargo-checksum.json")):
    raise SystemExit("vendor tree has no Cargo checksum records")
if not paths["license_report"].read_text().strip():
    raise SystemExit("dependency license report is empty")
for raw in paths["lockfile"].read_text().splitlines():
    line = raw.strip()
    if not line.startswith("source = "):
        continue
    source = line.split("=", 1)[1].strip().strip('"')
    if source.startswith("registry+"):
        continue
    raise SystemExit("Cargo.lock source is not represented by vendor configuration")
