#!/usr/bin/env python3
"""Validate one retained Cargo bundle without consulting the network."""
import hashlib
import json
import os
import stat
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


def source_archive_descriptor(root: Path, value: str) -> int:
    parts = value.split("/")
    if (not value or value.startswith("/") or any(part in {"", ".", ".."} for part in parts)):
        raise SystemExit("bundle source path escapes bundle root")
    directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    file_flags = os.O_RDONLY | os.O_NOFOLLOW
    directory_fd = None
    source_fd = None
    try:
        directory_fd = os.open(root, directory_flags)
        for part in parts[:-1]:
            child_fd = os.open(part, directory_flags, dir_fd=directory_fd)
            os.close(directory_fd)
            directory_fd = child_fd
        source_fd = os.open(parts[-1], file_flags, dir_fd=directory_fd)
        source_stat = os.fstat(source_fd)
        if not stat.S_ISREG(source_stat.st_mode):
            raise OSError
        return source_fd
    except OSError:
        if source_fd is not None:
            os.close(source_fd)
        raise SystemExit("bundle source is missing or symlinked")
    finally:
        if directory_fd is not None:
            os.close(directory_fd)


def stage_source_archive(root: Path, value: str, expected_digest: str, temporary: Path) -> Path:
    source_fd = source_archive_descriptor(root, value)
    staged = temporary / "source.tar.gz"
    digest = hashlib.sha256()
    try:
        with staged.open("xb") as output:
            while data := os.read(source_fd, 1024 * 1024):
                digest.update(data)
                output.write(data)
    finally:
        os.close(source_fd)
    if digest.hexdigest() != expected_digest:
        raise SystemExit("source SHA-256 mismatch")
    return staged


def source_archive_root(archive: Path, destination: Path, lockfile: Path) -> None:
    try:
        with tarfile.open(archive, mode="r:gz") as contents:
            members = contents.getmembers()
            names: set[str] = set()
            roots: set[str] = set()
            root_members = []
            for member in members:
                parts = member.name.split("/")
                if (not member.name or member.name.startswith("/") or
                        any(part in {"", ".", ".."} for part in parts)):
                    raise SystemExit("source archive member path escapes source root")
                if not (member.isdir() or member.isreg()):
                    raise SystemExit("source archive contains unsafe member")
                if member.name in names:
                    raise SystemExit("source archive has duplicate member")
                names.add(member.name)
                roots.add(parts[0])
                if len(parts) == 1:
                    root_members.append(member)
            if len(roots) != 1 or len(root_members) != 1 or not root_members[0].isdir():
                raise SystemExit("source archive must contain one regular top-level root")
            contents.extractall(destination, members=members)
    except SystemExit:
        raise
    except (OSError, tarfile.TarError):
        raise SystemExit("source archive cannot be read")
    root = destination / next(iter(roots))
    unpacked_lockfile = root / "Cargo.lock"
    if (unpacked_lockfile.is_symlink() or not unpacked_lockfile.is_file() or
            unpacked_lockfile.read_bytes() != lockfile.read_bytes()):
        raise SystemExit("source archive Cargo.lock differs from retained lockfile")


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
        if key in {"name", "version", "source", "checksum"} and value.startswith('"') and value.endswith('"'):
            package[key] = value[1:-1]
    return [package for package in packages if "source" in package]


def cargo_source_config(path: Path) -> dict[tuple[str, str], str]:
    values: dict[tuple[str, str], str] = {}
    section = ""
    for raw in path.read_text().splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            continue
        if " = " not in line:
            continue
        key, value = line.split(" = ", 1)
        if value.startswith('"') and value.endswith('"'):
            values[(section, key)] = value[1:-1]
    return values


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


def cargo_checksum(crate: Path, name: str, version: str) -> str:
    path = crate / ".cargo-checksum.json"
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"vendor tree lacks Cargo checksum for {name} {version}")
    try:
        record = json.loads(path.read_text())
    except (json.JSONDecodeError, UnicodeDecodeError):
        raise SystemExit(f"vendor Cargo checksum is invalid for {name} {version}")
    package = record.get("package") if isinstance(record, dict) else None
    files = record.get("files") if isinstance(record, dict) else None
    if not isinstance(package, str) or not package or not isinstance(files, dict):
        raise SystemExit(f"vendor Cargo checksum is invalid for {name} {version}")
    return package


def safe_archive_members(archive: Path) -> None:
    process = subprocess.Popen(["zstd", "-dc", str(archive)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert process.stdout is not None
    error = None
    previous = None
    names: set[str] = set()
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
                if member.name in names:
                    error = "vendor archive has duplicate member"
                    break
                order = tuple(member.name.split("/"))
                if previous is not None and order <= previous:
                    error = "vendor archive members are not sorted"
                    break
                if (member.mtime != 0 or member.uid != 0 or member.gid != 0 or
                        member.uname or member.gname):
                    error = "vendor archive member metadata is not deterministic"
                    break
                names.add(member.name)
                previous = order
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
helm_fields = bound_archive_fields | {
    "source_archive_format", "source_provenance", "source_provenance_sha256",
}
helm_bundle = record.get("name") == "helm-workspace"
if helm_bundle:
    valid_fields = frozenset(record) == frozenset(helm_fields)
else:
    valid_fields = frozenset(record) in {
        frozenset(basic_fields), frozenset(basic_archive_fields), frozenset(bound_archive_fields),
    }
if not valid_fields:
    raise SystemExit("bundle manifest fields differ from policy")

paths = {key: under_root(root, key, record[key]) for key in required - {"name"}}
if helm_bundle:
    paths["source_provenance"] = under_root(root, "source_provenance", record["source_provenance"])
elif "source" in record:
    paths["source"] = under_root(root, "source", record["source"])
for key, path in paths.items():
    if path.is_symlink() or not path.is_file():
        raise SystemExit(f"bundle {key} is missing or symlinked")
for key, path in paths.items():
    hash_key = f"{key}_sha256"
    if hash_key in record and hashlib.sha256(path.read_bytes()).hexdigest() != record[hash_key]:
        raise SystemExit(f"{key} SHA-256 mismatch")

source_temporary = None
if helm_bundle:
    if record["source"] != "source.tar.gz":
        raise SystemExit("Helm source archive path is not source.tar.gz")
    if record["source_archive_format"] != "tar.gz":
        raise SystemExit("source archive format is not tar.gz")
    source_temporary = tempfile.TemporaryDirectory()
    source_directory = Path(source_temporary.name)
    staged_source = stage_source_archive(root, record["source"], record["source_sha256"], source_directory)
    source_archive_root(staged_source, source_directory / "source", paths["lockfile"])

config = cargo_source_config(paths["cargo_config"])
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
if (config.get(("source.crates-io", "replace-with")) != "vendored-sources" or
        config.get(("source.vendored-sources", "directory")) != "vendor"):
    raise SystemExit("Cargo source replacement does not select vendor directory")

resolved = cargo_packages(paths["lockfile"])
for package in resolved:
    if package["source"] != "registry+https://github.com/rust-lang/crates.io-index":
        if package["source"].startswith("registry+"):
            raise SystemExit("Cargo.lock registry source is not crates.io")
        raise SystemExit("Cargo.lock source is not represented by vendor configuration")
packages = vendor_packages(vendor)
for package in resolved:
    key = (package.get("name", ""), package.get("version", ""))
    if key not in packages:
        raise SystemExit(f"vendor tree lacks Cargo checksum for {key[0]} {key[1]}")
    checksum = cargo_checksum(packages[key], key[0], key[1])
    if not package.get("checksum"):
        raise SystemExit(f"Cargo.lock package lacks checksum for {key[0]} {key[1]}")
    if checksum != package["checksum"]:
        raise SystemExit(f"vendor Cargo checksum disagrees with Cargo.lock for {key[0]} {key[1]}")
covered = license_rows(paths["license_report"], vendor)
for package in resolved:
    key = (package.get("name", ""), package.get("version", ""))
    if key not in covered:
        raise SystemExit(f"dependency license report lacks {key[0]} {key[1]}")
