#!/bin/sh
# The native Cargo recorder must judge build outputs before RPM removes BUILD/.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
wrapper=$root/packaging/tool-sources/native-cargo-wrapper.sh

if [ ! -x "$wrapper" ]; then
    echo "native Cargo wrapper is missing or not executable" >&2
    exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-native-cargo-wrapper.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

mkdir -p "$tmp/cargo-home"
printf '[source.crates-io]\nreplace-with = "retained"\n' >"$tmp/config.toml"
cp "$tmp/config.toml" "$tmp/cargo-home/config.toml"

cat >"$tmp/fake-cargo" <<'EOF'
#!/bin/sh
mkdir -p "${CARGO_TARGET_DIR:?}/release"
for bin in ${REALM_FAKE_OUTPUTS:?}; do
    : >"$CARGO_TARGET_DIR/release/$bin"
    chmod +x "$CARGO_TARGET_DIR/release/$bin"
done
EOF
chmod +x "$tmp/fake-cargo"

run_wrapper() {
    outputs=$1
    target=$2
    log=$3
    REALM_FAKE_OUTPUTS="$outputs" \
        REALM_REAL_CARGO="$tmp/fake-cargo" \
        REALM_SENTINEL_LOG="$log" \
        REALM_EXPECTED_CARGO_CONFIG="$tmp/config.toml" \
        CARGO_HOME="$tmp/cargo-home" \
        CARGO_TARGET_DIR="$target" \
        "$wrapper" build --release --frozen --offline --locked --workspace
}

complete_log=$tmp/complete.log
run_wrapper 'realmctl realm-wm realm-bar' "$tmp/complete-target" "$complete_log"
rm -rf "$tmp/complete-target"
for bin in realmctl realm-wm realm-bar; do
    if ! grep -F -x "cargo-output|binary=$bin|executable=yes" \
        "$complete_log" >/dev/null; then
        echo "completed build output was not retained across RPM cleanup: $bin" >&2
        exit 1
    fi
done

missing_log=$tmp/missing.log
if run_wrapper 'realmctl realm-wm' "$tmp/missing-target" "$missing_log"; then
    echo "native Cargo wrapper accepted a build without realm-bar" >&2
    exit 1
else
    status=$?
fi
if [ "$status" -ne 95 ]; then
    echo "missing build output failed with unexpected status: $status" >&2
    exit 1
fi
grep -F -x 'cargo-output|binary=realm-bar|executable=no' "$missing_log" >/dev/null

echo "PASS: native Cargo output lifetime"
