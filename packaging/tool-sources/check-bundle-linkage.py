#!/usr/bin/env python3
"""Validate one retained Cargo bundle without consulting the network."""
import sys
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
required = {"name", "lockfile", "vendor", "cargo_config", "license_report"}
if set(record) != required:
    raise SystemExit("bundle manifest fields differ from policy")

paths = {key: root / record[key] for key in required - {"name"}}
for key, path in paths.items():
    if not path.exists() or path.is_symlink():
        raise SystemExit(f"bundle {key} is missing or symlinked")

config = paths["cargo_config"].read_text()
vendor = paths["vendor"]
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
