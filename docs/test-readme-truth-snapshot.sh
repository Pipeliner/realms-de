#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH='' cd "$script_dir/.." && pwd)
guard="$script_dir/check-readme-truth-snapshot.sh"

# RED is deliberate: create the production check only after this harness fails.
if [ ! -x "$guard" ]; then
    echo "FAIL: missing executable README truth snapshot check: $guard" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/helm-readme-truth-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

tests_run=0

make_fixture() {
    name=$1
    fixture_root="$tmp_dir/$name"
    mkdir -p "$fixture_root/.github/workflows" \
        "$fixture_root/docs" \
        "$fixture_root/crates" \
        "$fixture_root/packaging/session" \
        "$fixture_root/packaging/systemd" \
        "$fixture_root/packaging/nix" \
        "$fixture_root/packaging/debian" \
        "$fixture_root/packaging/fedora" \
        "$fixture_root/configs/portal" \
        "$fixture_root/configs/templates" \
        "$fixture_root/crates/helm-theme/src"
    cp "$repo_root/README.md" "$fixture_root/README.md"
    cp "$repo_root/docs/ROADMAP.md" "$fixture_root/docs/ROADMAP.md"
    cp "$repo_root/.github/workflows/ci.yml" "$fixture_root/.github/workflows/ci.yml"
    : >"$fixture_root/flake.nix"
    : >"$fixture_root/packaging/session/helm.desktop"
    : >"$fixture_root/packaging/session/helm-session"
    : >"$fixture_root/packaging/systemd/helm-session.target"
    : >"$fixture_root/configs/portal/helm-portals.conf"
    : >"$fixture_root/crates/helm-theme/Cargo.toml"
    : >"$fixture_root/crates/helm-theme/src/lib.rs"
    printf '%s\n' '#[test]' >"$fixture_root/crates/helm-theme/src/theme.rs"
    : >"$fixture_root/packaging/nix/nixos-module.nix"
    : >"$fixture_root/packaging/debian/control"
    : >"$fixture_root/packaging/fedora/helm.spec"
    : >"$fixture_root/palette.toml"
    mkdir -p "$fixture_root/design" "$fixture_root/.claude"
    printf '%s\n' "$fixture_root"
}

run_guard() {
    "$guard" --root "$1" 2>&1
}

expect_pass() {
    name=$1
    fixture_root=$2
    tests_run=$((tests_run + 1))

    if ! output=$(run_guard "$fixture_root"); then
        printf 'FAIL: %s unexpectedly failed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

expect_fail() {
    name=$1
    fixture_root=$2
    expected=$3
    tests_run=$((tests_run + 1))

    if output=$(run_guard "$fixture_root"); then
        printf 'FAIL: %s unexpectedly passed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    case "$output" in
        *"$expected"*) ;;
        *)
            printf 'FAIL: %s failed for the wrong reason\nexpected: %s\nactual: %s\n' \
                "$name" "$expected" "$output" >&2
            exit 1
            ;;
    esac
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

fixture_root=$(make_fixture canonical)
expect_pass canonical-snapshot "$fixture_root"

fixture_root=$(make_fixture missing-blocker)
sed '/issues\/168/d' "$fixture_root/README.md" >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail missing-blocker "$fixture_root" 'missing needs-human snapshot issue #168'

fixture_root=$(make_fixture resolved-font-question)
sed '/## Repo map/i\\| [#34 — resolved font policy](https://github.com/Pipeliner/realms-de/issues/34) | must not return |' \
    "$fixture_root/README.md" >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail resolved-font-question "$fixture_root" 'closed issue #34 must not appear in the needs-human snapshot'

fixture_root=$(make_fixture changed-issue-title)
sed 's/Does the 𓂃 prompt sigil survive/Does the prompt sigil survive/' \
    "$fixture_root/README.md" >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail changed-issue-title "$fixture_root" 'needs-human snapshot title differs from GitHub'

fixture_root=$(make_fixture empty-blocker)
sed "s/The public \`theme lint\` and \`theme diff\` JSON contracts\./ /" \
    "$fixture_root/README.md" >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail empty-blocker "$fixture_root" 'needs-human snapshot blocker is empty'

fixture_root=$(make_fixture stale-session-claim)
sed 's/Tracked pre-alpha contract/Planned. Not started/' "$fixture_root/README.md" \
    >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail stale-session-claim "$fixture_root" 'README must not call tracked session assets planned/not started'

fixture_root=$(make_fixture missing-ci-invocation)
sed '/\.\/docs\/test-readme-truth-snapshot\.sh/d' \
    "$fixture_root/.github/workflows/ci.yml" >"$fixture_root/ci.next.yml"
mv "$fixture_root/ci.next.yml" "$fixture_root/.github/workflows/ci.yml"
expect_fail missing-ci-invocation "$fixture_root" 'documentation CI must run the README truth snapshot fixtures'

fixture_root=$(make_fixture commented-ci-invocations)
sed 's@^[[:space:]]*\./docs/\(test-readme-truth-snapshot\|check-readme-truth-snapshot\)\.sh@          # \&@' \
    "$fixture_root/.github/workflows/ci.yml" >"$fixture_root/ci.next.yml"
mv "$fixture_root/ci.next.yml" "$fixture_root/.github/workflows/ci.yml"
expect_fail commented-ci-invocations "$fixture_root" \
    'documentation CI must run the README truth snapshot fixtures'

fixture_root=$(make_fixture relocated-first-screen-identity)
sed 's/keyboard-first, gapless-tiling, Rust-first Wayland desktop environment/moved identity/' \
    "$fixture_root/README.md" >"$fixture_root/README.next"
printf '%s\n' 'keyboard-first, gapless-tiling, Rust-first Wayland desktop environment' \
    >>"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail relocated-first-screen-identity "$fixture_root" \
    'README first screen must state the full project identity'

fixture_root=$(make_fixture nonexistent-map-path)
sed 's@│  └─ portal/@│  └─ absent-portal/@' "$fixture_root/README.md" \
    >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail nonexistent-map-path "$fixture_root" \
    'README repository map must name configs/portal/'

fixture_root=$(make_fixture stale-roadmap)
sed 's/NativeBackend/NiriBackend/' "$fixture_root/docs/ROADMAP.md" \
    >"$fixture_root/ROADMAP.next"
mv "$fixture_root/ROADMAP.next" "$fixture_root/docs/ROADMAP.md"
expect_fail stale-roadmap "$fixture_root" 'roadmap must not refer to NiriBackend'

fixture_root=$(make_fixture changed-roadmap-m0)
sed 's/| \*\*in progress\*\* |/| planned |/' "$fixture_root/docs/ROADMAP.md" \
    >"$fixture_root/ROADMAP.next"
mv "$fixture_root/ROADMAP.next" "$fixture_root/docs/ROADMAP.md"
expect_fail changed-roadmap-m0 "$fixture_root" 'roadmap must mark M0 in progress'

fixture_root=$(make_fixture changed-roadmap-m3)
sed 's/\*\*\[M3\](#m3--daily-drivable) is the MVP\.\*\*/M3 is planned./' \
    "$fixture_root/docs/ROADMAP.md" >"$fixture_root/ROADMAP.next"
mv "$fixture_root/ROADMAP.next" "$fixture_root/docs/ROADMAP.md"
expect_fail changed-roadmap-m3 "$fixture_root" 'roadmap must identify M3 as the MVP'

fixture_root=$(make_fixture relocated-readme-m3)
sed 's/\*\*M3 is the MVP\.\*\*/M3 is planned./' "$fixture_root/README.md" \
    >"$fixture_root/README.next"
printf '%s\n' '**M3 is the MVP.**' >>"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail relocated-readme-m3 "$fixture_root" 'README status must identify M3 as the MVP'

fixture_root=$(make_fixture changed-snapshot-timestamp)
sed 's/2026-08-30T06:18:36Z/2026-08-30T06:18:37Z/' "$fixture_root/README.md" \
    >"$fixture_root/README.next"
mv "$fixture_root/README.next" "$fixture_root/README.md"
expect_fail changed-snapshot-timestamp "$fixture_root" 'README needs-human snapshot timestamp differs from the accepted snapshot'

fixture_root=$(make_fixture implemented-wm-crate)
mkdir -p "$fixture_root/crates/helm-session"
: >"$fixture_root/crates/helm-session/Cargo.toml"
expect_fail implemented-wm-crate "$fixture_root" 'README must not say helm-wm is absent after its implementation crate lands'

fixture_root=$(make_fixture missing-nix-module)
rm "$fixture_root/packaging/nix/nixos-module.nix"
expect_fail missing-nix-module "$fixture_root" 'README truth snapshot artifact is missing: packaging/nix/nixos-module.nix'

fixture_root=$(make_fixture missing-debian-definition)
rm "$fixture_root/packaging/debian/control"
expect_fail missing-debian-definition "$fixture_root" 'README truth snapshot artifact is missing: packaging/debian/control'

fixture_root=$(make_fixture missing-fedora-definition)
rm "$fixture_root/packaging/fedora/helm.spec"
expect_fail missing-fedora-definition "$fixture_root" 'README truth snapshot artifact is missing: packaging/fedora/helm.spec'

fixture_root=$(make_fixture missing-theme-tests)
rm "$fixture_root/crates/helm-theme/src/theme.rs"
expect_fail missing-theme-tests "$fixture_root" 'README truth snapshot artifact is missing: crates/helm-theme/src/theme.rs'

printf 'PASS: %d README truth snapshot fixtures\n' "$tests_run"
