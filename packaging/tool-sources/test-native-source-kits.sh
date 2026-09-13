#!/bin/sh
# The distributable native source kits retain packaging inputs, not a checkout.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
builder=$root/packaging/tool-sources/build-native-source-kits.sh
checker=$root/packaging/tool-sources/check-native-source-kit.py
bundle=$root/packaging/tool-sources/bundles/realm-workspace
tool_stager=$root/packaging/tool-sources/stage-tool-bundle.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-native-source-kits.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

if [ ! -x "$builder" ] || [ ! -x "$checker" ] || [ ! -x "$tool_stager" ]; then
    echo "native source-kit producer or checker is missing" >&2
    exit 1
fi

python3 "$root/packaging/tool-sources/test-tool-runtime.py" --self-test
"$root/packaging/session/test-private-tool-path.sh"

# SPEC 0024 requires the retained source authority to move whenever workspace
# source or Cargo build/test input moves. Keep this guard in the source checkout
# before the Git-free kit producer is placed behind sentinels below.
check_realm_workspace_freshness() {
    repository=$1
    bundle_root=$2
    bundle_commit=$(sed -n 's/^commit = "\([0-9a-f][0-9a-f]*\)"$/\1/p' \
        "$bundle_root/bundle.toml")
    if [ -z "$bundle_commit" ] \
        || ! git -C "$repository" cat-file -e "$bundle_commit^{commit}" 2>/dev/null; then
        echo "Realm workspace bundle does not name a reachable source commit" >&2
        return 1
    fi
    if ! git -C "$repository" diff --quiet "$bundle_commit" HEAD -- \
        . ':(exclude)packaging/tool-sources/bundles/**'; then
        echo "Realm workspace bundle is stale relative to retained source inputs" >&2
        git -C "$repository" diff --name-only "$bundle_commit" HEAD -- \
            . ':(exclude)packaging/tool-sources/bundles/**' >&2
        return 1
    fi
}

check_realm_workspace_freshness "$root" "$bundle"

if [ "$(grep -c -F 'yazi = "retained:yazi@25.4.8"' \
    "$root/packaging/tool-sources/targets.toml")" -ne 2 ] \
    || [ "$(grep -c -F 'starship = "retained:starship@1.23.0"' \
        "$root/packaging/tool-sources/targets.toml")" -ne 2 ] \
    || grep -E 'retained:(yazi@26\.8\.15|starship@1\.26\.0)' \
        "$root/packaging/tool-sources/targets.toml" >/dev/null; then
    echo "native target map does not select the accepted retained tool versions" >&2
    exit 1
fi

freshness_fixture=$tmp/freshness-fixture
fixture_bundle=$freshness_fixture/packaging/tool-sources/bundles/realm-workspace
mkdir -p "$freshness_fixture/packaging/session"
printf '#!/bin/sh\nexec river\n' > \
    "$freshness_fixture/packaging/session/realm-session"
git -C "$freshness_fixture" init -q
git -C "$freshness_fixture" add packaging/session/realm-session
git -C "$freshness_fixture" -c user.name='Realm test' \
    -c user.email='realm-test@example.invalid' commit -qm 'seed packaged wrapper'
fixture_source_commit=$(git -C "$freshness_fixture" rev-parse HEAD)
mkdir -p "$fixture_bundle"
printf '[bundle]\ncommit = "%s"\n' "$fixture_source_commit" > \
    "$fixture_bundle/bundle.toml"
git -C "$freshness_fixture" add packaging/tool-sources/bundles/realm-workspace/bundle.toml
git -C "$freshness_fixture" -c user.name='Realm test' \
    -c user.email='realm-test@example.invalid' commit -qm 'bind source authority'
printf '# wrapper behaviour changed\n' >> \
    "$freshness_fixture/packaging/session/realm-session"
git -C "$freshness_fixture" add packaging/session/realm-session
git -C "$freshness_fixture" -c user.name='Realm test' \
    -c user.email='realm-test@example.invalid' commit -qm 'change packaged wrapper'
if output=$(check_realm_workspace_freshness \
    "$freshness_fixture" "$fixture_bundle" 2>&1); then
    echo "Realm workspace freshness guard accepted a changed packaged wrapper" >&2
    exit 1
elif ! printf '%s\n' "$output" | \
    grep -F 'packaging/session/realm-session' >/dev/null; then
    echo "Realm workspace freshness guard omitted the changed wrapper diagnostic" >&2
    printf '%s\n' "$output" >&2
    exit 1
fi

archive_members=$(tar -tzf "$bundle/source.tar.gz")
for member in \
    realm-workspace/crates/realm-ctl/src/main.rs \
    realm-workspace/crates/realm-session/src/bin/realm-wm.rs \
    realm-workspace/crates/realm-bar/src/main.rs; do
    printf '%s\n' "$archive_members" | grep -Fx "$member" >/dev/null \
        || {
            echo "Realm workspace source authority omits mandatory runtime source: $member" >&2
            exit 1
        }
done

if grep -F "skipping \$\$bin: not built in this revision" \
    "$root/packaging/debian/rules" >/dev/null \
    || grep -F "if [ -x \"%{realm_target_dir}/release/\${bin}\" ]" \
        "$root/packaging/fedora/realm.spec" >/dev/null; then
    echo "native package recipes still permit a missing Realm runtime binary" >&2
    exit 1
fi

if grep -E '^(Suggests:|Recommends: +)(yazi|starship)' \
    "$root/packaging/debian/control" "$root/packaging/fedora/realm.spec" \
    >/dev/null \
    || grep -F 'suggests = "yazi, starship"' \
        "$root/packaging/debian/cargo-deb.toml.fragment" >/dev/null; then
    echo "native package metadata delegates Realm-owned tools to distribution packages" >&2
    exit 1
fi

mkdir -p "$tmp/sentinels"
for command in git curl wget ssh scp; do
    sed "s/@COMMAND@/$command/g" >"$tmp/sentinels/$command" <<'EOF'
#!/bin/sh
printf 'forbidden source-kit command: @COMMAND@ %s\n' "$*" >>"${REALM_KIT_SENTINEL_LOG:?}"
exit 97
EOF
    chmod +x "$tmp/sentinels/$command"
done
: >"$tmp/sentinel.log"
PATH="$tmp/sentinels:/usr/bin:/bin" \
    REALM_KIT_SENTINEL_LOG="$tmp/sentinel.log" \
    "$builder" "$tmp/output"

if [ -s "$tmp/sentinel.log" ]; then
    echo "native source-kit production invoked Git or a network command" >&2
    cat "$tmp/sentinel.log" >&2
    exit 1
fi

debian=$tmp/output/realm-debian-0.1.0
rpm_archive=$tmp/output/realm-0.1.0.tar.gz
rpm_spec=$tmp/output/realm.spec
if [ ! -d "$debian" ] || [ ! -f "$rpm_archive" ] || [ ! -f "$rpm_spec" ]; then
    echo "native source-kit producer omitted an output" >&2
    exit 1
fi

"$checker" debian "$debian"
mkdir -p "$tmp/rpm"
tar -C "$tmp/rpm" -xzf "$rpm_archive"
rpm=$tmp/rpm/realm-0.1.0
"$checker" rpm "$rpm"

for kit in "$debian" "$rpm"; do
    for selected in yazi-25.4.8 starship-1.23.0; do
        selected_bundle=$kit/packaging/tool-sources/bundles/$selected
        if [ ! -d "$selected_bundle" ]; then
            echo "native source kit omitted selected bundle: $selected" >&2
            exit 1
        fi
        staged=$tmp/staged-$(basename "$kit")-$selected
        source=$(python3 "$kit/packaging/tool-sources/stage-tool-bundle.py" \
            "$selected_bundle" "$staged")
        if [ "$source" != "$staged/source" ] \
            || [ ! -f "$source/Cargo.lock" ] \
            || [ ! -f "$staged/.cargo/config.toml" ] \
            || [ ! -d "$staged/vendor" ]; then
            echo "selected bundle did not materialize a complete Cargo stage: $selected" >&2
            exit 1
        fi
        if find "$source" -path '*/packaging/tool-sources/bundles' -print -quit \
            | grep . >/dev/null; then
            echo "selected bundle stage contains a recursive retained authority" >&2
            exit 1
        fi
    done
done

cp -R "$debian" "$tmp/debian-missing-selected"
rm -rf "$tmp/debian-missing-selected/packaging/tool-sources/bundles/yazi-25.4.8"
if "$checker" debian "$tmp/debian-missing-selected" >"$tmp/out" 2>"$tmp/err"; then
    echo "Debian source kit accepted a missing selected tool bundle" >&2
    exit 1
elif ! grep -F 'DEBIAN source kit bundle inventory differs from policy' \
    "$tmp/err" >/dev/null; then
    echo "missing selected tool bundle rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    exit 1
fi

guide_failures=0
for guide in \
    "$debian/packaging/package-docs/INSTALL.md" \
    "$rpm/packaging/package-docs/INSTALL.md"; do
    if [ ! -f "$guide" ]; then
        echo "native source kit omitted the current package build guide: $guide" >&2
        guide_failures=$((guide_failures + 1))
        continue
    fi
    if ! cmp "$root/docs/INSTALL.md" "$guide"; then
        echo "native source-kit package guide differs from tracked guidance" >&2
        guide_failures=$((guide_failures + 1))
    fi
    if grep -F 'ln -s packaging/debian' "$guide" >/dev/null \
        || grep -F 'git archive' "$guide" >/dev/null \
        || grep -F 'packaging/tool-sources/build-native-source-kits.sh' \
            "$guide" >/dev/null; then
        echo "native source-kit package guide retained a forbidden build workflow" >&2
        guide_failures=$((guide_failures + 1))
    fi
done
if [ "$guide_failures" -ne 0 ]; then
    exit 1
fi

if "$checker" debian "$root" >"$tmp/out" 2>"$tmp/err"; then
    echo "full checkout was accepted as a Debian source kit" >&2
    exit 1
elif ! grep -F 'Debian source kit top-level inventory differs from policy' "$tmp/err" >/dev/null; then
    echo "full checkout Debian rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    exit 1
fi
if "$checker" rpm "$root" >"$tmp/out" 2>"$tmp/err"; then
    echo "full checkout was accepted as an RPM source kit" >&2
    exit 1
elif ! grep -F 'RPM source kit top-level inventory differs from policy' "$tmp/err" >/dev/null; then
    echo "full checkout RPM rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    exit 1
fi

shadow_failures=0
cp -R "$debian" "$tmp/debian-shadow"
mkdir -p "$tmp/debian-shadow/debian/source/shadow-workspace/crates/shadow/src"
printf '[workspace]\nmembers = ["crates/shadow"]\n' > \
    "$tmp/debian-shadow/debian/source/shadow-workspace/Cargo.toml"
printf '[package]\nname = "shadow"\nversion = "0.0.0"\n' > \
    "$tmp/debian-shadow/debian/source/shadow-workspace/crates/shadow/Cargo.toml"
printf 'pub fn shadow() {}\n' > \
    "$tmp/debian-shadow/debian/source/shadow-workspace/crates/shadow/src/lib.rs"
if "$checker" debian "$tmp/debian-shadow" >"$tmp/out" 2>"$tmp/err"; then
    echo "nested Debian shadow workspace was accepted" >&2
    shadow_failures=$((shadow_failures + 1))
elif ! grep -F 'DEBIAN source kit contains forbidden workspace marker' \
    "$tmp/err" >/dev/null; then
    echo "nested Debian shadow workspace rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    shadow_failures=$((shadow_failures + 1))
fi

cp -R "$rpm" "$tmp/rpm-shadow"
mkdir -p "$tmp/rpm-shadow/packaging/fedora/shadow-workspace/crates/shadow/src"
printf '[workspace]\nmembers = ["crates/shadow"]\n' > \
    "$tmp/rpm-shadow/packaging/fedora/shadow-workspace/Cargo.toml"
printf '[package]\nname = "shadow"\nversion = "0.0.0"\n' > \
    "$tmp/rpm-shadow/packaging/fedora/shadow-workspace/crates/shadow/Cargo.toml"
printf 'pub fn shadow() {}\n' > \
    "$tmp/rpm-shadow/packaging/fedora/shadow-workspace/crates/shadow/src/lib.rs"
if "$checker" rpm "$tmp/rpm-shadow" >"$tmp/out" 2>"$tmp/err"; then
    echo "nested RPM shadow workspace was accepted" >&2
    shadow_failures=$((shadow_failures + 1))
elif ! grep -F 'RPM source kit contains forbidden workspace marker' \
    "$tmp/err" >/dev/null; then
    echo "nested RPM shadow workspace rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    shadow_failures=$((shadow_failures + 1))
fi
if [ "$shadow_failures" -ne 0 ]; then
    exit 1
fi

diff -qr "$root/packaging/debian" "$debian/debian"
diff -qr "$root/packaging/fedora" "$rpm/packaging/fedora"
cmp "$root/packaging/fedora/realm.spec" "$rpm_spec"
for helper in \
    check-bundle-linkage.py \
    check-native-source-kit.py \
    stage-realm-workspace.py \
    stage-tool-bundle.py \
    test-tool-runtime.py; do
    cmp "$root/packaging/tool-sources/$helper" \
        "$debian/packaging/tool-sources/$helper"
    cmp "$root/packaging/tool-sources/$helper" \
        "$rpm/packaging/tool-sources/$helper"
done
for selected in realm-workspace yazi-25.4.8 starship-1.23.0; do
    diff -qr "$root/packaging/tool-sources/bundles/$selected" \
        "$debian/packaging/tool-sources/bundles/$selected"
    diff -qr "$root/packaging/tool-sources/bundles/$selected" \
        "$rpm/packaging/tool-sources/bundles/$selected"
done
