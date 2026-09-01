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

[[package]]
name = "beta"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
EOF
    cat >"$tmp/bundle/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
EOF
    printf '{"files":{}}\n' >"$tmp/bundle/vendor/alpha-1.0.0/.cargo-checksum.json"
    printf '{"files":{}}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"
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
printf '{"files":{}}\n' >"$tmp/bundle/vendor/beta-2.0.0/.cargo-checksum.json"

# A report with a non-empty row still fails if another resolved crate is absent.
sed -i '/^beta\t/d' "$tmp/bundle/licenses.tsv"
rejects 'dependency license report lacks beta 2.0.0' "$tmp"
printf 'beta\t2.0.0\tMIT\tvendor/beta-2.0.0/Cargo.toml\n' >>"$tmp/bundle/licenses.tsv"

archive_vendor
"$checker" "$tmp"
cp "$tmp/bundle/vendor.tar.zst" "$tmp/valid-vendor.tar.zst"

# Reject traversal based on archive metadata, before tar extraction can act.
mkdir -p "$tmp/traversal/vendor/alpha-1.0.0"
printf x >"$tmp/traversal/vendor/alpha-1.0.0/file"
tar --sort=name --transform='s|^vendor/|vendor/../|' -C "$tmp/traversal" -cf - vendor \
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

if [ "$failures" -ne 0 ]; then
    exit 1
fi
