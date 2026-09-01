#!/usr/bin/env python3
"""Validate one retained Cargo bundle without consulting the network."""
import hashlib
import subprocess
import sys
import tarfile
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


def under_root(root: Path, key: str, value: str) -> Path:
    path = root / value
    try:
        path.resolve().relative_to(root)
    except ValueError:
        raise SystemExit(f"bundle {key} path escapes bundle root")
    return path


def cargo_packages(lockfile: Path) -> list[dict[str, str]]:
    packages: list[dict[str, str]] = []
    package: dict[str, str] = {}
    for raw in lockfile.read_text().splitlines() + ["[[package]]"]:
        line = raw.strip()
        if line == "[[package]]":
            if package:
                packages.append(package)
            package = {}
            continue
        if " = " not in line:
            continue
        key, value = line.split(" = ", 1)
        if key in {"name", "version", "source"} and value.startswith('"') and value.endswith('"'):
            package[key] = value[1:-1]
    return [package for package in packages if "source" in package]


def package_metadata(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    in_package = False
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if line == "[package]":
            in_package = True
            continue
        if in_package and line.startswith("["):
            break
        if not in_package or " = " not in line:
            continue
        key, value = line.split(" = ", 1)
        if key in {"name", "version", "license"} and value.startswith('"') and value.endswith('"'):
            values[key] = value[1:-1]
    return values


def vendor_packages(vendor: Path) -> dict[tuple[str, str], Path]:
    packages: dict[tuple[str, str], Path] = {}
    for crate in vendor.iterdir():
        if crate.is_symlink() or not crate.is_dir():
            raise SystemExit("vendor tree contains unsafe member")
        checksum = crate / ".cargo-checksum.json"
        metadata = crate / "Cargo.toml"
        if checksum.is_symlink() or not checksum.is_file() or metadata.is_symlink() or not metadata.is_file():
            continue
        package = package_metadata(metadata)
        if {"name", "version", "license"} - set(package):
            continue
        key = (package["name"], package["version"])
        if key in packages:
            raise SystemExit(f"vendor tree has duplicate crate metadata for {key[0]} {key[1]}")
        packages[key] = crate
    for entry in vendor.rglob("*"):
        if entry.is_symlink() or not (entry.is_file() or entry.is_dir()):
            raise SystemExit("vendor tree contains unsafe member")
    return packages


def safe_archive_members(archive: Path) -> None:
    process = subprocess.Popen(["zstd", "-dc", str(archive)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert process.stdout is not None
    error = None
    try:
        with tarfile.open(fileobj=process.stdout, mode="r|") as contents:
            for member in contents:
                parts = member.name.split("/")
                if (member.name.startswith("/") or not parts or parts[0] != "vendor" or
                        any(part in {"", ".", ".."} for part in parts)):
                    error = "vendor archive member path escapes vendor tree"
                    break
                if not (member.isdir() or member.isreg()):
                    error = "vendor archive contains unsafe member"
                    break
    except tarfile.TarError:
        error = "vendor archive cannot be read"
    finally:
        if error is not None:
            process.terminate()
        process.stdout.close()
        process.stderr.read()
        if process.wait() != 0 and error is None:
            error = "vendor archive cannot be read"
    if error is not None:
        raise SystemExit(error)


def license_rows(report: Path, vendor: Path) -> set[tuple[str, str]]:
    rows = report.read_text().splitlines()
    if not rows:
        raise SystemExit("dependency license report is empty")
    if rows[0] != "name\tversion\tlicense\tlicense_source":
        raise SystemExit("dependency license report has invalid header")
    covered: set[tuple[str, str]] = set()
    for line in rows[1:]:
        fields = line.split("\t")
        if len(fields) != 4 or not all(fields):
            raise SystemExit("dependency license report has incomplete row")
        name, version, license_name, reference = fields
        reference_path = Path(reference)
        if reference_path.is_absolute() or reference_path.parts[:1] != ("vendor",):
            raise SystemExit("dependency license report has unsafe retained-source reference")
        target = vendor.parent / reference_path
        try:
            target.resolve().relative_to(vendor)
        except ValueError:
            raise SystemExit("dependency license report has unsafe retained-source reference")
        if target.is_symlink() or not target.is_file() or target.name != "Cargo.toml":
            raise SystemExit("dependency license report has invalid retained-source reference")
        metadata = package_metadata(target)
        if metadata.get("name") != name or metadata.get("version") != version or metadata.get("license") != license_name:
            raise SystemExit("dependency license report disagrees with retained source")
        key = (name, version)
        if key in covered:
            raise SystemExit(f"dependency license report duplicates {name} {version}")
        covered.add(key)
    return covered


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
if frozenset(record) not in {frozenset(basic_fields), frozenset(basic_archive_fields), frozenset(bound_archive_fields)}:
    raise SystemExit("bundle manifest fields differ from policy")

paths = {key: under_root(root, key, record[key]) for key in required - {"name"}}
if "source" in record:
    paths["source"] = under_root(root, "source", record["source"])
for key, path in paths.items():
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"bundle {key} is missing or symlinked")
for key in ("source", "lockfile", "cargo_config", "license_report"):
    hash_key = f"{key}_sha256"
    if hash_key in record and hashlib.sha256(paths[key].read_bytes()).hexdigest() != record[hash_key]:
        raise SystemExit(f"{key} SHA-256 mismatch")

config = paths["cargo_config"].read_text()
temporary = None
if "vendor" in record:
    vendor = under_root(root, "vendor", record["vendor"])
    if vendor.is_symlink() or not vendor.is_dir():
        raise SystemExit("vendor tree is missing or symlinked")
else:
    archive = under_root(root, "vendor_archive", record["vendor_archive"])
    if archive.is_symlink() or not archive.is_file():
        raise SystemExit("vendor archive is missing or symlinked")
    if record["vendor_archive_format"] != "tar.zst":
        raise SystemExit("vendor archive format is not tar.zst")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != record["vendor_archive_sha256"]:
        raise SystemExit("vendor archive SHA-256 mismatch")
    safe_archive_members(archive)
    temporary = tempfile.TemporaryDirectory()
    unpacked = Path(temporary.name)
    subprocess.run(["tar", "--zstd", "-xf", str(archive), "-C", str(unpacked)], check=True)
    vendor = unpacked / "vendor"
    if vendor.is_symlink() or not vendor.is_dir():
        raise SystemExit("vendor archive did not unpack a regular vendor tree")
if 'replace-with = "vendored-sources"' not in config or 'directory = "vendor"' not in config:
    raise SystemExit("Cargo source replacement does not select vendor directory")

resolved = cargo_packages(paths["lockfile"])
for package in resolved:
    if not package["source"].startswith("registry+"):
        raise SystemExit("Cargo.lock source is not represented by vendor configuration")
packages = vendor_packages(vendor)
for package in resolved:
    key = (package.get("name", ""), package.get("version", ""))
    if key not in packages:
        raise SystemExit(f"vendor tree lacks Cargo checksum for {key[0]} {key[1]}")
covered = license_rows(paths["license_report"], vendor)
for package in resolved:
    key = (package.get("name", ""), package.get("version", ""))
    if key not in covered:
        raise SystemExit(f"dependency license report lacks {key[0]} {key[1]}")
