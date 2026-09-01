#!/bin/sh
# B1/B5: a Cargo bundle must account for every lockfile source and license.
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
checker=$root/packaging/tool-sources/check-bundle-linkage.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-bundle-linkage.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

"$checker" "$root/packaging/tool-sources/bundles/starship-1.23.0"
"$checker" "$root/packaging/tool-sources/bundles/yazi-25.4.8"

mkdir -p "$tmp/bundle/vendor/example-1.0.0" "$tmp/bundle/.cargo"
printf 'source = "registry+https://github.com/rust-lang/crates.io-index"\n' >"$tmp/bundle/Cargo.lock"
cat >"$tmp/bundle/.cargo/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
EOF
printf '{"files":{}}\n' >"$tmp/bundle/vendor/example-1.0.0/.cargo-checksum.json"
printf 'example-1.0.0|MIT|LICENSE\n' >"$tmp/bundle/licenses.tsv"
cat >"$tmp/bundle.toml" <<'EOF'
[bundle]
name = "fixture"
lockfile = "bundle/Cargo.lock"
vendor = "bundle/vendor"
cargo_config = "bundle/.cargo/config.toml"
license_report = "bundle/licenses.tsv"
EOF

"$checker" "$tmp"

tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/bundle" -cf - vendor | zstd -3 -q -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
rm -rf "$tmp/bundle/vendor"
sed -i '/^vendor = /d' "$tmp/bundle.toml"
cat >>"$tmp/bundle.toml" <<EOF
vendor_archive = "bundle/vendor.tar.zst"
vendor_archive_sha256 = "$archive_sha256"
vendor_archive_format = "tar.zst"
EOF

"$checker" "$tmp"

cp "$tmp/bundle/vendor.tar.zst" "$tmp/valid-vendor.tar.zst"

rm -f "$tmp/bundle/vendor.tar.zst"
mkdir -p "$tmp/malicious/vendor" "$tmp/malicious/target"
printf '{"files":{}}\n' >"$tmp/malicious/target/.cargo-checksum.json"
ln -s ../target "$tmp/malicious/vendor/example-1.0.0"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$tmp/malicious" -cf - vendor | zstd -3 -q -o "$tmp/bundle/vendor.tar.zst"
archive_sha256=$(sha256sum "$tmp/bundle/vendor.tar.zst" | awk '{print $1}')
sed -i "s/^vendor_archive_sha256 = .*/vendor_archive_sha256 = \"$archive_sha256\"/" "$tmp/bundle.toml"
if "$checker" "$tmp" >"$tmp/out" 2>"$tmp/err"; then
    echo 'vendor archive with symlinked crate unexpectedly passed' >&2
    exit 1
fi
grep -F 'vendor tree contains symlink' "$tmp/err"

# Restore the valid archive before exercising independent source-accounting
# failures.  Otherwise the earlier malicious fixture masks every later check.
cp "$tmp/valid-vendor.tar.zst" "$tmp/bundle/vendor.tar.zst"

printf 'source = "git+https://example.invalid/unrepresented#abc"\n' >>"$tmp/bundle/Cargo.lock"
if "$checker" "$tmp" >"$tmp/out" 2>"$tmp/err"; then
    echo 'unrepresented Cargo source unexpectedly passed' >&2
    exit 1
fi
grep -F 'Cargo.lock source is not represented by vendor configuration' "$tmp/err"
sed -i '/git+https/d' "$tmp/bundle/Cargo.lock"
printf '\n' >"$tmp/bundle/licenses.tsv"
if "$checker" "$tmp" >"$tmp/out" 2>"$tmp/err"; then
    echo 'empty dependency license report unexpectedly passed' >&2
    exit 1
fi
grep -F 'dependency license report is empty' "$tmp/err"
