#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH='' cd "$script_dir/../.." && pwd)
guard="$script_dir/check-root-flake-ci.sh"

# RED is deliberate: add the production guard only after this harness fails.
if [ ! -x "$guard" ]; then
    echo "FAIL: missing executable root-flake CI guard: $guard" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/realm-root-flake-ci-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

tests_run=0

make_fixture() {
    name=$1
    fixture_root="$tmp_dir/$name"
    mkdir -p "$fixture_root/.github/workflows"
    mkdir -p "$fixture_root/packaging/nix"
    cp "$repo_root/flake.nix" "$fixture_root/flake.nix"
    cp "$repo_root/flake.lock" "$fixture_root/flake.lock"
    cp "$repo_root/.github/workflows/distro.yml" "$fixture_root/.github/workflows/distro.yml"
    cp "$repo_root/packaging/nix/checks.nix" "$fixture_root/packaging/nix/checks.nix"
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
expect_pass canonical-root-flake-contract "$fixture_root"

fixture_root=$(make_fixture missing-flake)
rm -f "$fixture_root/flake.nix"
expect_fail missing-flake "$fixture_root" 'root flake.nix is required'

fixture_root=$(make_fixture missing-lock)
rm -f "$fixture_root/flake.lock"
expect_fail missing-lock "$fixture_root" 'root flake.lock is required'

fixture_root=$(make_fixture conditional-nix-job)
printf '%s\n' '        if: steps.flake.outputs.present == '\''true'\''' \
    >>"$fixture_root/.github/workflows/distro.yml"
expect_fail conditional-nix-job "$fixture_root" 'Nix CI must not condition on flake presence'

fixture_root=$(make_fixture missing-runtime-guard)
sed '/\.\/packaging\/nix\/check-root-flake-ci\.sh/d' \
    "$fixture_root/.github/workflows/distro.yml" >"$fixture_root/workflow.yml"
mv "$fixture_root/workflow.yml" "$fixture_root/.github/workflows/distro.yml"
expect_fail missing-runtime-guard "$fixture_root" \
    'normal Nix CI must invoke the root-flake guard'

fixture_root=$(make_fixture missing-fixture-suite)
sed '/\.\/packaging\/nix\/test-root-flake-ci\.sh/d' \
    "$fixture_root/.github/workflows/distro.yml" >"$fixture_root/workflow.yml"
mv "$fixture_root/workflow.yml" "$fixture_root/.github/workflows/distro.yml"
expect_fail missing-fixture-suite "$fixture_root" \
    'normal Nix CI must invoke the root-flake fixture suite'

fixture_root=$(make_fixture missing-portal-helper-imports)
sed '/portal-helper-imports/d' \
    "$fixture_root/.github/workflows/distro.yml" >"$fixture_root/workflow.yml"
mv "$fixture_root/workflow.yml" "$fixture_root/.github/workflows/distro.yml"
expect_fail missing-portal-helper-imports "$fixture_root" \
    'Nix CI must run the portal helper import smoke before the VM'

fixture_root=$(make_fixture missing-portal-evidence-upload)
sed '/realm-session-boots\/portal-roundtrip\.json/d' \
    "$fixture_root/.github/workflows/distro.yml" >"$fixture_root/workflow.yml"
mv "$fixture_root/workflow.yml" "$fixture_root/.github/workflows/distro.yml"
expect_fail missing-portal-evidence-upload "$fixture_root" \
    'live VM evidence upload must retain portal-roundtrip.json'

fixture_root=$(make_fixture portal-window-delete-instead-of-cancel)
sed 's/machine.send_key("alt-c")/machine.send_key("esc")/' \
    "$fixture_root/packaging/nix/checks.nix" >"$fixture_root/checks.nix"
mv "$fixture_root/checks.nix" "$fixture_root/packaging/nix/checks.nix"
expect_fail portal-window-delete-instead-of-cancel "$fixture_root" \
    'portal VM must activate the explicit GTK Cancel response'

fixture_root=$(make_fixture portal-failure-skips-status)
sed 's/f"(if {portal_command}/f"({portal_command}/' \
    "$fixture_root/packaging/nix/checks.nix" >"$fixture_root/checks.nix"
mv "$fixture_root/checks.nix" "$fixture_root/packaging/nix/checks.nix"
expect_fail portal-failure-skips-status "$fixture_root" \
    'portal VM must record helper status after success or failure'

failure_status="$tmp_dir/helper.status"
(
    set -e
    if sh -c 'exit 7'; then
        realm_portal_status=0
    else
        realm_portal_status=$?
    fi
    printf '%s\n' "$realm_portal_status" >"$failure_status"
)
tests_run=$((tests_run + 1))
if [ "$(cat "$failure_status")" != 7 ]; then
    echo 'FAIL: set -e helper wrapper did not retain nonzero status' >&2
    exit 1
fi
printf 'ok %d - portal-failure-status-behaviour\n' "$tests_run"

fixture_root=$(make_fixture missing-doctor-evidence)
sed '/realm-session-boots\/realmctl-doctor\.json/d' \
    "$fixture_root/.github/workflows/distro.yml" >"$fixture_root/workflow.yml"
mv "$fixture_root/workflow.yml" "$fixture_root/.github/workflows/distro.yml"
expect_fail missing-doctor-evidence "$fixture_root" \
    'live VM artifact must retain realmctl doctor JSON'

fixture_root=$(make_fixture missing-prompt-regex-import)
sed '/^      import re$/d' \
    "$fixture_root/packaging/nix/checks.nix" >"$fixture_root/checks.nix"
mv "$fixture_root/checks.nix" "$fixture_root/packaging/nix/checks.nix"
expect_fail missing-prompt-regex-import "$fixture_root" \
    'Nix VM prompt proof must import its regular-expression dependency'

printf 'PASS: %d root-flake CI guard fixtures\n' "$tests_run"
