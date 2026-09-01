#!/bin/sh
# B2: the real Debian and RPM drivers must consume only the retained Helm authority.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-native-builds.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

for command in dh rpmbuild python3 tar zstd; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "required native-build test command is missing: $command" >&2
        exit 1
    fi
done

run_isolated() {
    if [ "${HELM_NATIVE_NETWORK_ISOLATED:-0}" = 1 ]; then
        "$@"
    elif unshare --user --map-root-user --net true >/dev/null 2>&1; then
        unshare --user --map-root-user --net "$@"
    else
        echo "cannot create the mandatory network namespace" >&2
        exit 1
    fi
}

copy_authority() {
    kit=$1
    mkdir -p "$kit/packaging/tool-sources/bundles"
    cp "$root/packaging/tool-sources/check-bundle-linkage.py" \
        "$kit/packaging/tool-sources/check-bundle-linkage.py"
    if [ -f "$root/packaging/tool-sources/stage-helm-workspace.py" ]; then
        cp "$root/packaging/tool-sources/stage-helm-workspace.py" \
            "$kit/packaging/tool-sources/stage-helm-workspace.py"
    fi
    cp -R "$root/packaging/tool-sources/bundles/helm-workspace" \
        "$kit/packaging/tool-sources/bundles/helm-workspace"
}

make_debian_kit() {
    kit=$1
    mkdir -p "$kit/packaging"
    cp -R "$root/packaging/debian" "$kit/packaging/debian"
    copy_authority "$kit"
    ln -s packaging/debian "$kit/debian"
    if [ -e "$kit/Cargo.toml" ] || [ -e "$kit/crates" ]; then
        echo "Debian source kit contains a second Helm workspace tree" >&2
        exit 1
    fi
}

make_rpm_tree() {
    tree=$1
    kit=$tree/source/helm-0.1.0
    mkdir -p "$kit/packaging" "$tree/top/SOURCES" "$tree/top/SPECS" \
        "$tree/top/BUILD" "$tree/top/BUILDROOT" "$tree/top/RPMS" "$tree/top/SRPMS"
    cp -R "$root/packaging/fedora" "$kit/packaging/fedora"
    copy_authority "$kit"
    cp "$root/packaging/fedora/helm.spec" "$tree/top/SPECS/helm.spec"
    if [ -e "$kit/Cargo.toml" ] || [ -e "$kit/crates" ]; then
        echo "RPM Source0 kit contains a second Helm workspace tree" >&2
        exit 1
    fi
    tar -C "$tree/source" -czf "$tree/top/SOURCES/helm-0.1.0.tar.gz" helm-0.1.0
}

make_sentinels() {
    directory=$1
    mkdir -p "$directory"
    for command in git curl wget ssh scp; do
        sed "s/@COMMAND@/$command/g" >"$directory/$command" <<'EOF'
#!/bin/sh
printf 'forbidden|command=@COMMAND@|cwd=%s|args=%s\n' "$PWD" "$*" >>"${HELM_SENTINEL_LOG:?}"
exit 97
EOF
        chmod +x "$directory/$command"
    done
    cat >"$directory/rustc" <<'EOF'
#!/bin/sh
printf '%s\n' 'rustc 1.97.1 (fixture)'
EOF
    cat >"$directory/cargo" <<'EOF'
#!/bin/sh
printf 'cargo|cwd=%s|home=%s|args=%s\n' "$PWD" "${CARGO_HOME:-}" "$*" >>"${HELM_SENTINEL_LOG:?}"
exit 0
EOF
    chmod +x "$directory/rustc" "$directory/cargo"
}

run_debian() {
    kit=$1
    log=$2
    source=$kit/debian/helm-workspace/source
    cargo_home=$kit/debian/.cargo-home
    sentinels=$kit/sentinels
    make_sentinels "$sentinels"
    mkdir -p "$kit/outer-cargo-home"
    run_isolated env \
        PATH="$sentinels:/usr/bin:/bin" \
        CARGO_HOME="$kit/outer-cargo-home" \
        HELM_EXPECTED_SOURCE="$source" \
        HELM_EXPECTED_CARGO_HOME="$cargo_home" \
        HELM_SENTINEL_LOG="$log" \
        make -C "$kit" -f debian/rules build RUST_VERSIONED_BIN="$sentinels"
}

run_rpm() {
    tree=$1
    log=$2
    top=$tree/top
    source=$top/BUILD/helm-0.1.0/.helm-workspace/source
    cargo_home=$top/BUILD/helm-0.1.0/.cargo-home
    sentinels=$tree/sentinels
    make_sentinels "$sentinels"
    mkdir -p "$tree/home" "$tree/outer-cargo-home"
    run_isolated env \
        HOME="$tree/home" \
        PATH="$sentinels:/usr/bin:/bin" \
        CARGO_HOME="$tree/outer-cargo-home" \
        HELM_EXPECTED_SOURCE="$source" \
        HELM_EXPECTED_CARGO_HOME="$cargo_home" \
        HELM_SENTINEL_LOG="$log" \
        rpmbuild -bb --nodeps \
            --define "_topdir $top" \
            --define "rust_arches $(uname -m)" \
            --define "_userunitdir /usr/lib/systemd/user" \
            "$top/SPECS/helm.spec"
}

rejects_before_cargo() {
    name=$1
    runner=$2
    fixture=$3
    log=$4
    output=$5
    : >"$log"
    if "$runner" "$fixture" "$log" >"$output" 2>&1; then
        fail "$name accepted a same-name source archive with a different digest"
    elif ! grep -F 'source SHA-256 mismatch' "$output" >/dev/null; then
        fail "$name rejected the substitution for the wrong reason"
        sed -n '1,120p' "$output" >&2
    fi
    if grep '^cargo|' "$log" >/dev/null; then
        fail "$name invoked Cargo before source-digest refusal"
    fi
}

accepts_offline_cargo() {
    name=$1
    runner=$2
    fixture=$3
    log=$4
    output=$5
    : >"$log"
    if ! "$runner" "$fixture" "$log" >"$output" 2>&1; then
        fail "$name valid source kit did not complete its native build path"
        sed -n '1,160p' "$output" >&2
        return
    fi
    if grep '^forbidden|' "$log" >/dev/null; then
        fail "$name invoked Git or a network command"
    fi
    cargo_count=$(grep -c '^cargo|' "$log" || true)
    if [ "$cargo_count" -ne 2 ]; then
        fail "$name made $cargo_count Cargo invocations instead of build and test only"
    fi
    build_count=$(grep -c '|args=build ' "$log" || true)
    test_count=$(grep -c '|args=test ' "$log" || true)
    if [ "$build_count" -ne 1 ] || [ "$test_count" -ne 1 ]; then
        fail "$name did not make exactly one Cargo build and one Cargo test invocation"
    fi
    while IFS= read -r invocation; do
        case $invocation in
            *"|cwd=${HELM_EXPECTED_SOURCE}"*) ;;
            *) fail "$name invoked Cargo outside its staged canonical source" ;;
        esac
        case $invocation in
            *"|home=${HELM_EXPECTED_CARGO_HOME}"*) ;;
            *) fail "$name did not use its empty package-local Cargo home" ;;
        esac
        for flag in --frozen --offline --locked; do
            case " $invocation " in
                *" $flag "*) ;;
                *) fail "$name Cargo invocation omitted $flag" ;;
            esac
        done
    done <"$log"
}

rejects_injected_fetch() {
    name=$1
    runner=$2
    fixture=$3
    log=$4
    output=$5
    : >"$log"
    if "$runner" "$fixture" "$log" >"$output" 2>&1; then
        fail "$name executed an injected fetch without failing"
    fi
    if ! grep '^forbidden|command=git|' "$log" >/dev/null; then
        fail "$name injected fetch did not reach the network-command sentinel"
    fi
    if grep '^cargo|' "$log" >/dev/null; then
        fail "$name continued to Cargo after the injected fetch failed"
    fi
}

make_debian_kit "$tmp/debian-invalid"
printf 'different retained bytes\n' >> \
    "$tmp/debian-invalid/packaging/tool-sources/bundles/helm-workspace/source.tar.gz"
rejects_before_cargo Debian run_debian "$tmp/debian-invalid" \
    "$tmp/debian-invalid.log" "$tmp/debian-invalid.out"

make_debian_kit "$tmp/debian-valid"
HELM_EXPECTED_SOURCE="$tmp/debian-valid/debian/helm-workspace/source"
HELM_EXPECTED_CARGO_HOME="$tmp/debian-valid/debian/.cargo-home"
export HELM_EXPECTED_SOURCE HELM_EXPECTED_CARGO_HOME
accepts_offline_cargo Debian run_debian "$tmp/debian-valid" \
    "$tmp/debian-valid.log" "$tmp/debian-valid.out"

make_debian_kit "$tmp/debian-fetch"
sed -i '/^override_dh_auto_build:/a\
\tgit fetch https://example.invalid/helm' "$tmp/debian-fetch/debian/rules"
rejects_injected_fetch Debian run_debian "$tmp/debian-fetch" \
    "$tmp/debian-fetch.log" "$tmp/debian-fetch.out"

make_rpm_tree "$tmp/rpm-invalid"
mkdir -p "$tmp/rpm-invalid/substitution"
tar -C "$tmp/rpm-invalid/source" -xzf "$tmp/rpm-invalid/top/SOURCES/helm-0.1.0.tar.gz" \
    -C "$tmp/rpm-invalid/substitution"
printf 'different retained bytes\n' >> \
    "$tmp/rpm-invalid/substitution/helm-0.1.0/packaging/tool-sources/bundles/helm-workspace/source.tar.gz"
tar -C "$tmp/rpm-invalid/substitution" -czf \
    "$tmp/rpm-invalid/top/SOURCES/helm-0.1.0.tar.gz" helm-0.1.0
rejects_before_cargo RPM run_rpm "$tmp/rpm-invalid" \
    "$tmp/rpm-invalid.log" "$tmp/rpm-invalid.out"

make_rpm_tree "$tmp/rpm-valid"
HELM_EXPECTED_SOURCE="$tmp/rpm-valid/top/BUILD/helm-0.1.0/.helm-workspace/source"
HELM_EXPECTED_CARGO_HOME="$tmp/rpm-valid/top/BUILD/helm-0.1.0/.cargo-home"
export HELM_EXPECTED_SOURCE HELM_EXPECTED_CARGO_HOME
accepts_offline_cargo RPM run_rpm "$tmp/rpm-valid" \
    "$tmp/rpm-valid.log" "$tmp/rpm-valid.out"

make_rpm_tree "$tmp/rpm-fetch"
sed -i '/^%build$/a git fetch https://example.invalid/helm' \
    "$tmp/rpm-fetch/top/SPECS/helm.spec"
rejects_injected_fetch RPM run_rpm "$tmp/rpm-fetch" \
    "$tmp/rpm-fetch.log" "$tmp/rpm-fetch.out"

if [ "$failures" -ne 0 ]; then
    exit 1
fi
