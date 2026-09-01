#!/bin/sh
# Helm-workspace B2: real Debian/RPM drivers consume only retained authority.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
kit_builder=$root/packaging/tool-sources/build-native-source-kits.sh
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-native-builds.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

for command in dh dpkg-deb gzip rpm2archive rpmbuild python3 tar zstd; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "required native-build test command is missing: $command" >&2
        exit 1
    fi
done
real_cargo=${HELM_REAL_CARGO:-$(command -v cargo || true)}
real_rustc=${HELM_REAL_RUSTC:-$(command -v rustc || true)}
if [ ! -x "$real_cargo" ] || [ ! -x "$real_rustc" ]; then
    echo "real Cargo or rustc is not executable" >&2
    exit 1
fi
real_rust_bin=$(dirname "$real_rustc")

"$kit_builder" "$tmp/production"

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

make_debian_kit() {
    kit=$1
    cp -R "$tmp/production/helm-debian-0.1.0" "$kit"
}

make_rpm_tree() {
    tree=$1
    mkdir -p "$tree/top/SOURCES" "$tree/top/SPECS" \
        "$tree/top/BUILD" "$tree/top/BUILDROOT" "$tree/top/RPMS" "$tree/top/SRPMS"
    cp "$tmp/production/helm-0.1.0.tar.gz" "$tree/top/SOURCES/helm-0.1.0.tar.gz"
    cp "$tmp/production/helm.spec" "$tree/top/SPECS/helm.spec"
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
    cat >"$directory/cargo" <<'EOF'
#!/bin/sh
printf 'cargo|cwd=%s|home=%s|args=%s\n' "$PWD" "${CARGO_HOME:-}" "$*" >>"${HELM_SENTINEL_LOG:?}"
if [ ! -e "${HELM_CARGO_START_MARKER:?}" ]; then
    if find "${CARGO_HOME:?}" -mindepth 1 -print -quit | grep . >/dev/null; then
        printf 'cargo-home-not-empty|home=%s\n' "$CARGO_HOME" >>"$HELM_SENTINEL_LOG"
        exit 96
    fi
    : >"$HELM_CARGO_START_MARKER"
fi
set +e
"${HELM_REAL_CARGO:?}" "$@"
status=$?
set -e
printf 'cargo-result|status=%s\n' "$status" >>"$HELM_SENTINEL_LOG"
exit "$status"
EOF
    chmod +x "$directory/cargo"
}

run_debian() {
    kit=$1
    log=$2
    source=$kit/debian/helm-workspace/source
    cargo_home=$kit/debian/.cargo-home
    target_dir=$kit/debian/cargo-target
    fixture_state=$kit.fixture-state
    sentinels=$fixture_state/sentinels
    make_sentinels "$sentinels"
    mkdir -p "$fixture_state/outer-cargo-home"
    run_isolated env \
        PATH="$sentinels:$real_rust_bin:/usr/bin:/bin" \
        CARGO_HOME="$fixture_state/outer-cargo-home" \
        HELM_EXPECTED_SOURCE="$source" \
        HELM_EXPECTED_CARGO_HOME="$cargo_home" \
        HELM_EXPECTED_TARGET_DIR="$target_dir" \
        HELM_SENTINEL_LOG="$log" \
        HELM_CARGO_START_MARKER="$fixture_state/cargo-started" \
        HELM_REAL_CARGO="$real_cargo" \
        HELM_REAL_RUSTC="$real_rustc" \
        CARGO_INCREMENTAL=0 \
        CARGO_PROFILE_RELEASE_DEBUG=0 \
        make -C "$kit" -f debian/rules binary RUST_VERSIONED_BIN="$sentinels"
}

assert_current_package_guide() {
    guide_name=$1
    guide_root=$2
    guide_output=$3
    guide_path=$(find "$guide_root/usr/share/doc" -type f \
        \( -name 'INSTALL.md' -o -name 'INSTALL.md.gz' \) -print -quit)
    if [ -z "$guide_path" ]; then
        fail "$guide_name package omitted its installed build guide"
        return
    fi
    case $guide_path in
        *.gz) gzip -cd "$guide_path" >"$guide_output" ;;
        *) cp "$guide_path" "$guide_output" ;;
    esac
    if grep -F 'ln -s packaging/debian' "$guide_output" >/dev/null \
        || grep -F 'git archive' "$guide_output" >/dev/null; then
        fail "$guide_name package installed a guide with a forbidden build workflow"
    fi
    if ! grep -F 'packaging/tool-sources/build-native-source-kits.sh' \
        "$guide_output" >/dev/null; then
        fail "$guide_name package installed a guide without the retained-kit producer"
    fi
}

run_rpm() {
    tree=$1
    log=$2
    top=$tree/top
    source=$top/BUILD/helm-0.1.0/.helm-workspace/source
    cargo_home=$top/BUILD/helm-0.1.0/.cargo-home
    target_dir=$top/BUILD/helm-0.1.0/.cargo-target
    sentinels=$tree/sentinels
    make_sentinels "$sentinels"
    mkdir -p "$tree/home" "$tree/outer-cargo-home"
    run_isolated env \
        HOME="$tree/home" \
        PATH="$sentinels:$real_rust_bin:/usr/bin:/bin" \
        CARGO_HOME="$tree/outer-cargo-home" \
        HELM_EXPECTED_SOURCE="$source" \
        HELM_EXPECTED_CARGO_HOME="$cargo_home" \
        HELM_EXPECTED_TARGET_DIR="$target_dir" \
        HELM_SENTINEL_LOG="$log" \
        HELM_CARGO_START_MARKER="$tree/cargo-started" \
        HELM_REAL_CARGO="$real_cargo" \
        HELM_REAL_RUSTC="$real_rustc" \
        CARGO_INCREMENTAL=0 \
        CARGO_PROFILE_RELEASE_DEBUG=0 \
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
    fi
    if grep '^forbidden|' "$log" >/dev/null; then
        fail "$name invoked Git or a network command"
    fi
    if grep '^cargo-home-not-empty|' "$log" >/dev/null; then
        fail "$name did not begin with an empty package-local Cargo home"
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
    build_invocation=$(grep '|args=build ' "$log" | head -n 1 || true)
    case $build_invocation in
        *"|args=build --release --frozen --offline --locked --workspace") ;;
        *) fail "$name did not run the exact complete staged workspace build" ;;
    esac
    test_invocation=$(grep '|args=test ' "$log" | head -n 1 || true)
    case $test_invocation in
        *"|args=test --release --frozen --offline --locked --workspace --exclude helm-agent-sdd") ;;
        *) fail "$name did not run the exact package-relevant workspace test selection" ;;
    esac
    case $name in
        Debian)
            if [ ! -x "$HELM_EXPECTED_TARGET_DIR/release/helmctl" ]; then
                fail "$name Cargo build did not produce the staged workspace helmctl"
            fi
            deb_artifact=$(find "$HELM_EXPECTED_PACKAGE_ROOT" -maxdepth 1 \
                -type f -name 'helm_*_*.deb' -print -quit)
            if [ -z "$deb_artifact" ]; then
                fail "$name native driver did not emit a package artifact"
            else
                mkdir -p "$output.deb-root"
                dpkg-deb -x "$deb_artifact" "$output.deb-root"
                assert_current_package_guide "$name" "$output.deb-root" \
                    "$output.package-guide"
            fi
            ;;
        RPM)
            rpm_artifact=$(find "$HELM_EXPECTED_PACKAGE_ROOT" -type f \
                -name 'helm-*.rpm' -print -quit)
            if [ -z "$rpm_artifact" ]; then
                fail "$name native driver did not emit a package artifact"
            elif ! rpm -qpl "$rpm_artifact" | grep -Fx '/usr/bin/helmctl' >/dev/null; then
                fail "$name package did not contain the staged workspace helmctl"
            else
                mkdir -p "$output.rpm-archive" "$output.rpm-root"
                rpm_tar=$output.rpm-archive/package.tgz
                if ! rpm2archive "$rpm_artifact" >"$rpm_tar"; then
                    fail "$name package could not be converted for guide inspection"
                else
                    tar -C "$output.rpm-root" -xzf "$rpm_tar"
                    assert_current_package_guide "$name" "$output.rpm-root" \
                        "$output.package-guide"
                fi
            fi
            ;;
    esac
    if ! grep -F 'test result: ok.' "$output" >/dev/null; then
        fail "$name Cargo test did not execute the staged workspace tests"
    fi
    result_count=$(grep -c '^cargo-result|status=0$' "$log" || true)
    if [ "$result_count" -ne 2 ]; then
        fail "$name did not complete both real Cargo invocations successfully"
    fi
    grep '^cargo|' "$log" >"$output.cargo"
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
    done <"$output.cargo"
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
HELM_EXPECTED_TARGET_DIR="$tmp/debian-valid/debian/cargo-target"
HELM_EXPECTED_PACKAGE_ROOT="$tmp"
export HELM_EXPECTED_SOURCE HELM_EXPECTED_CARGO_HOME HELM_EXPECTED_TARGET_DIR \
    HELM_EXPECTED_PACKAGE_ROOT
accepts_offline_cargo Debian run_debian "$tmp/debian-valid" \
    "$tmp/debian-valid.log" "$tmp/debian-valid.out"

make_debian_kit "$tmp/debian-fetch"
sed -i '/^override_dh_auto_build:/a\
\tgit fetch https://example.invalid/helm' "$tmp/debian-fetch/debian/rules"
rejects_injected_fetch Debian run_debian "$tmp/debian-fetch" \
    "$tmp/debian-fetch.log" "$tmp/debian-fetch.out"

make_rpm_tree "$tmp/rpm-invalid"
mkdir -p "$tmp/rpm-invalid/substitution"
tar -C "$tmp/rpm-invalid/substitution" -xzf \
    "$tmp/rpm-invalid/top/SOURCES/helm-0.1.0.tar.gz"
printf 'different retained bytes\n' >> \
    "$tmp/rpm-invalid/substitution/helm-0.1.0/packaging/tool-sources/bundles/helm-workspace/source.tar.gz"
tar -C "$tmp/rpm-invalid/substitution" -czf \
    "$tmp/rpm-invalid/top/SOURCES/helm-0.1.0.tar.gz" helm-0.1.0
rejects_before_cargo RPM run_rpm "$tmp/rpm-invalid" \
    "$tmp/rpm-invalid.log" "$tmp/rpm-invalid.out"

make_rpm_tree "$tmp/rpm-valid"
HELM_EXPECTED_SOURCE="$tmp/rpm-valid/top/BUILD/helm-0.1.0/.helm-workspace/source"
HELM_EXPECTED_CARGO_HOME="$tmp/rpm-valid/top/BUILD/helm-0.1.0/.cargo-home"
HELM_EXPECTED_TARGET_DIR="$tmp/rpm-valid/top/BUILD/helm-0.1.0/.cargo-target"
HELM_EXPECTED_PACKAGE_ROOT="$tmp/rpm-valid/top/RPMS"
export HELM_EXPECTED_SOURCE HELM_EXPECTED_CARGO_HOME HELM_EXPECTED_TARGET_DIR \
    HELM_EXPECTED_PACKAGE_ROOT
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
