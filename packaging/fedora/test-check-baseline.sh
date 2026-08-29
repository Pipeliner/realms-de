#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
guard="$script_dir/check-baseline.sh"

if [ ! -x "$guard" ]; then
    echo "FAIL: missing executable Fedora baseline guard: $guard" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/helm-fedora-baseline-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

canonical_image='registry.fedoraproject.org/fedora:44@sha256:df52038ff64ee61affa188d78beb85cf6eecfe4e9f6042238269ccdc8e944392'
canonical_schema='helm-fedora-baseline/v1'
tests_run=0

write_manifest() {
    case_dir=$1
    schema=$2
    status=$3
    release=$4
    eol=$5
    image=$6
    extra=${7-}
    omitted=${8-}

    mkdir -p "$case_dir"
    cp "$guard" "$case_dir/check-baseline.sh"
    {
        if [ "$omitted" != schema ]; then
            printf 'schema = "%s"\n' "$schema"
        fi
        if [ "$omitted" != status ]; then
            printf 'status = "%s"\n' "$status"
        fi
        if [ "$omitted" != release ]; then
            printf 'release = %s\n' "$release"
        fi
        if [ "$omitted" != eol ]; then
            printf 'eol = "%s"\n' "$eol"
        fi
        if [ "$omitted" != image ]; then
            printf 'image = "%s"\n' "$image"
        fi
        if [ -n "$extra" ]; then
            printf '%s\n' "$extra"
        fi
    } >"$case_dir/baseline.toml"
}

expect_pass() {
    name=$1
    date=$2
    shift 2
    case_dir="$tmp_dir/$name"
    write_manifest "$case_dir" "$@"
    tests_run=$((tests_run + 1))

    if ! output=$("$case_dir/check-baseline.sh" --date "$date" 2>&1); then
        printf 'FAIL: %s unexpectedly failed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

expect_fail() {
    name=$1
    date=$2
    shift 2
    case_dir="$tmp_dir/$name"
    write_manifest "$case_dir" "$@"
    tests_run=$((tests_run + 1))

    if output=$("$case_dir/check-baseline.sh" --date "$date" 2>&1); then
        printf 'FAIL: %s unexpectedly passed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

expect_fail_message() {
    name=$1
    date=$2
    expected=$3
    shift 3
    case_dir="$tmp_dir/$name"
    write_manifest "$case_dir" "$@"
    tests_run=$((tests_run + 1))

    if output=$("$case_dir/check-baseline.sh" --date "$date" 2>&1); then
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

# A wrong lifecycle comparison would fail the first three boundary fixtures.
expect_pass before-eol 2027-06-01 "$canonical_schema" pre-alpha 44 2027-06-02 "$canonical_image"
expect_fail on-eol 2027-06-02 "$canonical_schema" pre-alpha 44 2027-06-02 "$canonical_image"
expect_fail after-eol 2027-06-03 "$canonical_schema" pre-alpha 44 2027-06-02 "$canonical_image"

# Unsupported retains the canonical last-admitted identity but has no active
# lifecycle. The separately scoped consistency guard must retire live claims.
expect_pass unsupported 2030-01-01 "$canonical_schema" unsupported 44 2027-06-02 "$canonical_image"

# Each closed-record obligation has a fixture that changes only that property.
expect_fail_message fedora-41 2027-06-01 'admits only Fedora release 44' \
    "$canonical_schema" pre-alpha 41 2027-06-02 "$canonical_image"
expect_fail_message wrong-schema 2027-06-01 'unsupported schema' \
    helm-fedora-baseline/v2 pre-alpha 44 2027-06-02 "$canonical_image"
expect_fail_message wrong-image 2027-06-01 'image does not match SPEC 0009' \
    "$canonical_schema" pre-alpha 44 2027-06-02 \
    'registry.fedoraproject.org/fedora:44@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
expect_fail_message invalid-status 2027-06-01 'unsupported status' \
    "$canonical_schema" current 44 2027-06-02 "$canonical_image"
expect_fail_message missing-image 2027-06-01 'missing image field' \
    "$canonical_schema" pre-alpha 44 2027-06-02 "$canonical_image" '' image
expect_fail unknown-field 2027-06-01 "$canonical_schema" pre-alpha 44 2027-06-02 \
    "$canonical_image" 'track = "latest"'
expect_fail duplicate-field 2027-06-01 "$canonical_schema" pre-alpha 44 2027-06-02 \
    "$canonical_image" 'release = 44'

# Dates are calendar values, not merely ten-character strings.
expect_fail_message invalid-evaluation-date 2027-02-29 'evaluation date is not a real' \
    "$canonical_schema" pre-alpha 44 2027-06-02 "$canonical_image"
expect_fail_message invalid-eol-date 2027-06-01 'EOL is not a real' \
    "$canonical_schema" pre-alpha 44 2027-02-29 "$canonical_image"
expect_fail_message noncanonical-eol 2027-06-01 'EOL does not match SPEC 0009' \
    "$canonical_schema" pre-alpha 44 2027-06-03 "$canonical_image"

printf 'PASS: %d Fedora baseline guard fixtures\n' "$tests_run"
