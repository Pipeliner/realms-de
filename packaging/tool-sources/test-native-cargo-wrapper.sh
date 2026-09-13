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
if [ "$#" -eq 1 ] && [ "$1" = -V ]; then
    printf 'cargo 1.90.0 (fixture)\n'
    exit 0
fi
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
    shift 3
    REALM_FAKE_OUTPUTS="$outputs" \
        REALM_REAL_CARGO="$tmp/fake-cargo" \
        REALM_SENTINEL_LOG="$log" \
        REALM_EXPECTED_CARGO_CONFIG="$tmp/config.toml" \
        CARGO_HOME="$tmp/cargo-home" \
        CARGO_TARGET_DIR="$target" \
        "$wrapper" build --release --frozen --offline --locked "$@"
}

complete_log=$tmp/complete.log
run_wrapper 'realmctl realm-wm realm-bar' "$tmp/complete-target" "$complete_log" --workspace
rm -rf "$tmp/complete-target"
for bin in realmctl realm-wm realm-bar; do
    if ! grep -F -x "cargo-output|binary=$bin|executable=yes" \
        "$complete_log" >/dev/null; then
        echo "completed build output was not retained across RPM cleanup: $bin" >&2
        exit 1
    fi
done

missing_log=$tmp/missing.log
if run_wrapper 'realmctl realm-wm' "$tmp/missing-target" "$missing_log" --workspace; then
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

for fixture in 'yazi ya|--package yazi-fm --package yazi-cli' \
    'starship|--bin starship'; do
    outputs=${fixture%%|*}
    selection=${fixture#*|}
    log=$tmp/$(printf '%s' "$outputs" | tr ' ' '-').log
    # Intentional splitting: these are fixed fixture arguments, not user input.
    # shellcheck disable=SC2086
    run_wrapper "$outputs" "$tmp/tool-target" "$log" $selection
    for bin in $outputs; do
        grep -F -x "cargo-output|binary=$bin|executable=yes" "$log" >/dev/null
    done
    rm -rf "$tmp/tool-target"
done

# Pinned shadow-rs makes this one metadata query after the outer Starship
# Cargo process has populated its retained home. It is not another build.
metadata_home=$tmp/starship-metadata-home
metadata_source=$tmp/starship-source
mkdir -p "$metadata_home/registry" "$metadata_source"
cp "$tmp/config.toml" "$metadata_home/config.toml"
metadata_log=$tmp/starship-metadata.log
(
    cd "$metadata_source"
    REALM_FAKE_OUTPUTS='' \
        REALM_REAL_CARGO="$tmp/fake-cargo" \
        REALM_SENTINEL_LOG="$metadata_log" \
        REALM_EXPECTED_CARGO_CONFIG="$tmp/config.toml" \
        REALM_EXPECTED_STARSHIP_SOURCE="$metadata_source" \
        REALM_EXPECTED_STARSHIP_CARGO_HOME="$metadata_home" \
        CARGO_HOME="$metadata_home" \
        CARGO_TARGET_DIR="$tmp/metadata-target" \
        "$wrapper" -V
)
grep -F -x 'cargo-metadata|kind=starship-version|status=0' \
    "$metadata_log" >/dev/null

if (
    cd "$metadata_source"
    REALM_FAKE_OUTPUTS='' \
        REALM_REAL_CARGO="$tmp/fake-cargo" \
        REALM_SENTINEL_LOG="$tmp/tree.log" \
        REALM_EXPECTED_CARGO_CONFIG="$tmp/config.toml" \
        REALM_EXPECTED_STARSHIP_SOURCE="$metadata_source" \
        REALM_EXPECTED_STARSHIP_CARGO_HOME="$metadata_home" \
        CARGO_HOME="$metadata_home" \
        CARGO_TARGET_DIR="$tmp/tree-target" \
        "$wrapper" tree
); then
    echo "native Cargo wrapper accepted Starship cargo tree metadata" >&2
    exit 1
fi

echo "PASS: native Cargo output lifetime"
