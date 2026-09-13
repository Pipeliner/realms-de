#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
check="$script_dir/check-vm-evidence-status.sh"
tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/realm-vm-evidence-status.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

tests_run=0

expect_pass() {
    name=$1
    status_file=$2
    tests_run=$((tests_run + 1))
    if ! output=$(sh "$check" "$status_file" 2>&1); then
        printf 'FAIL: %s unexpectedly failed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

expect_fail() {
    name=$1
    status_file=$2
    expected=$3
    tests_run=$((tests_run + 1))
    if output=$(sh "$check" "$status_file" 2>&1); then
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

printf 'realm-session-boots-status/v1\nexit_code=0\n' >"$tmp_dir/pass"
expect_pass zero-status "$tmp_dir/pass"

expect_fail missing-status "$tmp_dir/missing" 'VM evidence status is missing'

: >"$tmp_dir/empty"
expect_fail empty-status "$tmp_dir/empty" 'no complete schema line'

printf 'realm-session-boots-status/v2\nexit_code=0\n' >"$tmp_dir/schema"
expect_fail unknown-schema "$tmp_dir/schema" 'unknown schema'

printf 'realm-session-boots-status/v1\nexit_code=7\n' >"$tmp_dir/nonzero"
expect_fail nonzero-status "$tmp_dir/nonzero" 'exited with status 7'

printf 'realm-session-boots-status/v1\nexit_code=wat\n' >"$tmp_dir/malformed"
expect_fail malformed-status "$tmp_dir/malformed" 'malformed exit code'

printf 'realm-session-boots-status/v1\nexit_code=0\nextra\n' >"$tmp_dir/extra"
expect_fail trailing-data "$tmp_dir/extra" 'unexpected trailing data'

printf 'realm-session-boots-status/v1\nexit_code=0' >"$tmp_dir/unterminated"
expect_fail unterminated-status "$tmp_dir/unterminated" 'no complete exit-code line'

printf 'PASS: %d VM evidence status fixtures\n' "$tests_run"
