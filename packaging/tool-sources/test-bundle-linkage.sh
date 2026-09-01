#!/bin/sh
# B1/B5: a Cargo bundle must bind only retained paths and cover its closure.
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
checker=$root/packaging/tool-sources/check-bundle-linkage.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-bundle-linkage.XXXXXX")
escaped=$(mktemp "${TMPDIR:-/tmp}/helm-bundle-escape.XXXXXX")
trap 'rm -rf "$tmp" "$escaped"' EXIT HUP INT TERM
failures=0

"$checker" "$root/packaging/tool-sources/bundles/starship-1.23.0"
"$checker" "$root/packaging/tool-sources/bundles/yazi-25.4.8"

rejects() {
    expected=$1
    shift
    if "$checker" "$@" >"$tmp/out" 2>"$tmp/err"; then
        echo "fixture unexpectedly passed: $expected" >&2
        failures=$((failures + 1))
    elif ! grep -F "$expected" "$tmp/err" >/dev/null; then
        echo "fixture failed for the wrong reason; expected: $expected" >&2
        cat "$tmp/err" >&2
        failures=$((failures + 1))
    fi
}

write_manifest() {
    cat >"$tmp/bundle.toml" <<'EOF'
[bundle]
name = "fixture"
lockfile = "bundle/Cargo.lock"
vendor = "bundle/vendor"
cargo_config = "bundle/.cargo/config.toml"
license_report = "bundle/licenses.tsv"
EOF
}

write_fixture() {
    mkdir -p "$tmp/bundle/vendor/alpha-1.0.0" "$tmp/bundle/vendor/beta-2.0.0" \
        "$tmp/bundle/.cargo"
    cat >"$tmp/bundle/Cargo.lock" <<'EOF'
[[package]]
name = "alpha"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[[package]]
name = "beta"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
EOF
    cat >"$tmp/bundle/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
EOF
    printf '{"files":{},"package":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}\n' >"$tmp/bundle/vendor/alpha-1.0.0/.cargo-checksum.json"
    printf '{"files":{},"package":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"
    cat >"$tmp/bundle/vendor/alpha-1.0.0/Cargo.toml" <<'EOF'
[package]
name = "alpha"
version = "1.0.0"
license = "MIT"
EOF
    cat >"$tmp/bundle/vendor/beta-2.0.0/Cargo.toml" <<'EOF'
[package]
name = "beta"
version = "2.0.0"
license = "MIT"
EOF
    cat >"$tmp/bundle/licenses.tsv" <<'EOF'
name	version	license	license_source
alpha	1.0.0	MIT	vendor/alpha-1.0.0/Cargo.toml
beta	2.0.0	MIT	vendor/beta-2.0.0/Cargo.toml
EOF
    write_manifest
}

archive_vendor() {
    tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
        -C "$tmp/bundle" -cf - vendor | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
    archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
    rm -rf "$tmp/bundle/vendor"
    sed -i '/^vendor = /d' "$tmp/bundle.toml"
    cat >>"$tmp/bundle.toml" <<EOF
vendor_archive = "bundle/vendor.tar.zst"
vendor_archive_sha256 = "$archive_sha256"
vendor_archive_format = "tar.zst"
EOF
}

write_fixture
"$checker" "$tmp"

# Comments that look like source settings do not configure Cargo.
cp "$tmp/bundle/.cargo/config.toml" "$tmp/clean-config.toml"
cat >"$tmp/bundle/.cargo/config.toml" <<'EOF'
[source.crates-io]
# replace-with = "vendored-sources"
[source.vendored-sources]
# directory = "vendor"
EOF
rejects 'Cargo source replacement does not select vendor directory' "$tmp"
mv "$tmp/clean-config.toml" "$tmp/bundle/.cargo/config.toml"

# A crates-io replacement cannot account for a separate private registry.
cp "$tmp/bundle/Cargo.lock" "$tmp/clean-Cargo.lock"
sed -i 's|registry+https://github.com/rust-lang/crates.io-index|registry+https://example.invalid/private-index|' "$tmp/bundle/Cargo.lock"
rejects 'Cargo.lock registry source is not crates.io' "$tmp"
mv "$tmp/clean-Cargo.lock" "$tmp/bundle/Cargo.lock"

# A vendor directory is unusable until crates-io explicitly replaces itself.
sed -i '/replace-with/d' "$tmp/bundle/.cargo/config.toml"
rejects 'Cargo source replacement does not select vendor directory' "$tmp"
sed -i '/\[source.crates-io\]/a replace-with = "vendored-sources"' "$tmp/bundle/.cargo/config.toml"

# A Git source has no retained source-replacement declaration in this fixture.
cp "$tmp/bundle/Cargo.lock" "$tmp/clean-Cargo.lock"
cat >>"$tmp/bundle/Cargo.lock" <<'EOF'

[[package]]
name = "unrepresented"
version = "1.0.0"
source = "git+https://example.invalid/unrepresented#abc"
EOF
rejects 'Cargo.lock source is not represented by vendor configuration' "$tmp"
mv "$tmp/clean-Cargo.lock" "$tmp/bundle/Cargo.lock"

# A record must not redirect a bundle input to an adjacent file.
cp "$tmp/bundle/Cargo.lock" "$escaped"
sed -i "s|lockfile = \"bundle/Cargo.lock\"|lockfile = \"../$(basename "$escaped")\"|" "$tmp/bundle.toml"
rejects 'bundle lockfile path escapes bundle root' "$tmp"
sed -i 's|lockfile = "../[^\"]*"|lockfile = "bundle/Cargo.lock"|' "$tmp/bundle.toml"

# Checking one checksum is insufficient when Cargo.lock resolves two crates.
rm "$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"
rejects 'vendor tree lacks Cargo checksum for beta 2.0.0' "$tmp"
printf '{"files":{},"package":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"

# Cargo's vendor checksum record must be parseable and bind the lock checksum.
cp "$tmp/bundle/Cargo.lock" "$tmp/clean-Cargo.lock"
sed -i '/^checksum = "bbbb/d' "$tmp/bundle/Cargo.lock"
rejects 'Cargo.lock package lacks checksum for beta 2.0.0' "$tmp"
mv "$tmp/clean-Cargo.lock" "$tmp/bundle/Cargo.lock"
printf '{}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"
rejects 'vendor Cargo checksum is invalid for beta 2.0.0' "$tmp"
printf '{"files":{},"package":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"
rejects 'vendor Cargo checksum disagrees with Cargo.lock for beta 2.0.0' "$tmp"
printf '{"files":{},"package":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"

# A report with a non-empty row still fails if another resolved crate is absent.
sed -i '/^beta\t/d' "$tmp/bundle/licenses.tsv"
rejects 'dependency license report lacks beta 2.0.0' "$tmp"
printf 'beta\t2.0.0\tMIT\tvendor/beta-2.0.0/Cargo.toml\n' >>"$tmp/bundle/licenses.tsv"

archive_vendor
"$checker" "$tmp"
cp "$tmp/bundle/vendor.tar.zst" "$tmp/valid-vendor.tar.zst"
mkdir -p "$tmp/archive-input"
tar --zstd -xf "$tmp/valid-vendor.tar.zst" -C "$tmp/archive-input"

# Deterministic archives sort member names, including their directories.
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/archive-input" -cf - vendor/beta-2.0.0 vendor/alpha-1.0.0 \
    | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive members are not sorted' "$tmp"

# Archive metadata must retain the epoch and numeric uid/gid, not host values.
touch "$tmp/archive-input/vendor/alpha-1.0.0/Cargo.toml"
tar -C "$tmp/archive-input" -cf - vendor | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive member metadata is not deterministic' "$tmp"

# Duplicate archive members make the retained tree ambiguous.
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner --hard-dereference \
    -C "$tmp/archive-input" -cf - vendor vendor | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive has duplicate member' "$tmp"
cp "$tmp/valid-vendor.tar.zst" "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"

# Reject traversal based on archive metadata, before tar extraction can act.
mkdir -p "$tmp/traversal/vendor/alpha-1.0.0"
printf x >"$tmp/traversal/vendor/alpha-1.0.0/file"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    --transform='s|^vendor/|vendor/../|' -C "$tmp/traversal" -cf - vendor \
    | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive member path escapes vendor tree' "$tmp"

# Symlinks and special files are also rejected from archive metadata.
cp "$tmp/valid-vendor.tar.zst" "$tmp/bundle/vendor.tar.zst"
mkdir -p "$tmp/unsafe/vendor/alpha-1.0.0"
printf '{"files":{}}\n' >"$tmp/unsafe/vendor/alpha-1.0.0/.cargo-checksum.json"
ln -s ../alpha-1.0.0 "$tmp/unsafe/vendor/link"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/unsafe" -cf - vendor | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive contains unsafe member' "$tmp"
rm "$tmp/unsafe/vendor/link"
mkfifo "$tmp/unsafe/vendor/pipe"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/unsafe" -cf - vendor | zstd -3 -q -f -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
rejects 'vendor archive contains unsafe member' "$tmp"

write_helm_fixture() {
    mkdir -p "$tmp/helm/bundle/.cargo" "$tmp/helm/source-input/helm-workspace"
    cp "$tmp/bundle/Cargo.lock" "$tmp/helm/bundle/Cargo.lock"
    cp "$tmp/bundle/.cargo/config.toml" "$tmp/helm/bundle/.cargo/config.toml"
    cp "$tmp/bundle/licenses.tsv" "$tmp/helm/bundle/licenses.tsv"
    cp "$tmp/valid-vendor.tar.zst" "$tmp/helm/bundle/vendor.tar.zst"
    cp "$tmp/helm/bundle/Cargo.lock" "$tmp/helm/source-input/helm-workspace/Cargo.lock"
    printf 'fixture source authority\n' >"$tmp/helm/source-input/helm-workspace/README"
    tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
        -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
    printf 'fixture provenance\n' >"$tmp/helm/provenance.md"
    helm_source_sha256=$(sha256sum "$tmp/helm/source.tar.gz" | awk '{print $1}')
    helm_provenance_sha256=$(sha256sum "$tmp/helm/provenance.md" | awk '{print $1}')
    helm_lockfile_sha256=$(sha256sum "$tmp/helm/bundle/Cargo.lock" | awk '{print $1}')
    helm_config_sha256=$(sha256sum "$tmp/helm/bundle/.cargo/config.toml" | awk '{print $1}')
    helm_license_sha256=$(sha256sum "$tmp/helm/bundle/licenses.tsv" | awk '{print $1}')
    helm_vendor_sha256=$(sha256sum "$tmp/helm/bundle/vendor.tar.zst" | awk '{print $1}')
    cat >"$tmp/helm/bundle.toml" <<EOF
[bundle]
name = "helm-workspace"
version = "0.1.0"
commit = "1111111111111111111111111111111111111111"
commit_timestamp = "2026-09-01T00:00:00Z"
source = "source.tar.gz"
source_sha256 = "$helm_source_sha256"
source_archive_format = "tar.gz"
source_provenance = "provenance.md"
source_provenance_sha256 = "$helm_provenance_sha256"
lockfile = "bundle/Cargo.lock"
lockfile_sha256 = "$helm_lockfile_sha256"
cargo_config = "bundle/.cargo/config.toml"
cargo_config_sha256 = "$helm_config_sha256"
license_report = "bundle/licenses.tsv"
license_report_sha256 = "$helm_license_sha256"
vendor_archive = "bundle/vendor.tar.zst"
vendor_archive_sha256 = "$helm_vendor_sha256"
vendor_archive_format = "tar.zst"
EOF
}

helm_source_digest() {
    helm_source_sha256=$(sha256sum "$tmp/helm/source.tar.gz" | awk '{print $1}')
    sed -i "s/^source_sha256 = .*/source_sha256 = \"$helm_source_sha256\"/" "$tmp/helm/bundle.toml"
}

write_helm_source() {
    rm -rf "$tmp/helm/source-input"
    mkdir -p "$tmp/helm/source-input/helm-workspace"
    cp "$tmp/helm/bundle/Cargo.lock" "$tmp/helm/source-input/helm-workspace/Cargo.lock"
    printf 'fixture source authority\n' >"$tmp/helm/source-input/helm-workspace/README"
    tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
        -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
    helm_source_digest
}

race_source_replacement() {
    mode=$1
    source=$tmp/helm/source.tar.gz
    replacement=$tmp/helm/attacker-source.tar.gz
    opened=$tmp/helm/opened-source.tar.gz
    printf attacker >"$replacement"
    "$checker" "$tmp/helm" >"$tmp/out" 2>"$tmp/err" &
    checker_pid=$!
    if ! python3 -c '
import os
import sys
import time

pid, source, replacement, opened, mode = sys.argv[1:]
deadline = time.monotonic() + 10
while time.monotonic() < deadline:
    try:
        descriptors = os.listdir(f"/proc/{pid}/fd")
    except FileNotFoundError:
        break
    for descriptor in descriptors:
        try:
            target = os.readlink(f"/proc/{pid}/fd/{descriptor}")
        except FileNotFoundError:
            continue
        if target != source:
            continue
        os.rename(source, opened)
        if mode == "replace":
            os.rename(replacement, source)
        else:
            os.symlink(replacement, source)
        sys.exit(0)
    time.sleep(0.0001)
sys.exit(1)
' "$checker_pid" "$source" "$replacement" "$opened" "$mode"; then
        echo "fixture did not replace source after descriptor acquisition" >&2
        failures=$((failures + 1))
    fi
    if ! wait "$checker_pid"; then
        echo "fixture rejected its descriptor-staged source after replacement" >&2
        cat "$tmp/err" >&2
        failures=$((failures + 1))
    fi
    if [ "$mode" = symlink ] && [ ! -L "$source" ]; then
        echo "fixture did not swap source pathname for a symlink" >&2
        failures=$((failures + 1))
    fi
    rm -f "$source"
    mv "$opened" "$source"
}

write_helm_fixture
"$checker" "$tmp/helm"

# The descriptor-selected source authority cannot disappear after the manifest binds it.
mv "$tmp/helm/source.tar.gz" "$tmp/helm/source.saved.tar.gz"
rejects 'bundle source is missing or symlinked' "$tmp/helm"
mv "$tmp/helm/source.saved.tar.gz" "$tmp/helm/source.tar.gz"

# A same-name replacement with different bytes must fail the source digest binding.
printf replacement >>"$tmp/helm/source.tar.gz"
rejects 'source SHA-256 mismatch' "$tmp/helm"
write_helm_source

# Provenance is a separately retained, digest-bound source authority record.
printf replacement >>"$tmp/helm/provenance.md"
rejects 'source_provenance SHA-256 mismatch' "$tmp/helm"
printf 'fixture provenance\n' >"$tmp/helm/provenance.md"

# Archive members must not escape, duplicate, introduce unsafe types, or make roots ambiguous.
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    --transform='s|^helm-workspace/|helm-workspace/../|' \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
helm_source_digest
rejects 'source archive member path escapes source root' "$tmp/helm"
write_helm_source
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace helm-workspace
helm_source_digest
rejects 'source archive has duplicate member' "$tmp/helm"
write_helm_source
ln -s Cargo.lock "$tmp/helm/source-input/helm-workspace/unsafe-link"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
helm_source_digest
rejects 'source archive contains unsafe member' "$tmp/helm"
rm "$tmp/helm/source-input/helm-workspace/unsafe-link"
write_helm_source
mkdir -p "$tmp/helm/source-input/another-root"
printf other >"$tmp/helm/source-input/another-root/README"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace another-root
helm_source_digest
rejects 'source archive must contain one regular top-level root' "$tmp/helm"
write_helm_source
printf mismatch >>"$tmp/helm/source-input/helm-workspace/Cargo.lock"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
helm_source_digest
rejects 'source archive Cargo.lock differs from retained lockfile' "$tmp/helm"
write_helm_source

# Replacing the pathname after open cannot substitute bytes for the staged descriptor.
dd if=/dev/urandom of="$tmp/helm/source-input/helm-workspace/padding" bs=1M count=32 status=none
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/helm/source-input" -czf "$tmp/helm/source.tar.gz" helm-workspace
helm_source_digest
race_source_replacement replace
race_source_replacement symlink

if [ "$failures" -ne 0 ]; then
    exit 1
fi
