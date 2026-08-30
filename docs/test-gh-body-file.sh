#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
helper=$root/scripts/gh-body-file
fixture=$root/docs/fixtures/gh-body-file
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

expect_argv() {
    capture=$1
    expected=$2
    cmp -s "$capture" "$expected" || fail 'unexpected GitHub CLI argv'
}

write_expected() {
    expected=$1
    shift
    : >"$expected"
    for argument in "$@"; do
        printf '%s\0' "$argument" >>"$expected"
    done
}

run_helper() {
    capture=$1
    shift
    GH_CAPTURE=$capture PATH=$fixture:$PATH "$helper" "$@"
}

expect_no_gh_on_failure() {
    capture=$1
    shift
    if run_helper "$capture" "$@"; then
        fail 'invalid helper invocation succeeded'
    fi
    [ ! -e "$capture" ] || fail 'invalid helper invocation reached GitHub CLI'
}

body=$fixture/literal-body.md
escape_marker=/tmp/gh-body-file-escaped
rm -f "$escape_marker"

capture=$tmp/issue-comment.argv
expected=$tmp/issue-comment.expected
run_helper "$capture" issue-comment 123 "$body"
write_expected "$expected" issue comment 123 --body-file "$body"
expect_argv "$capture" "$expected"
[ ! -e "$escape_marker" ] || fail 'literal body was evaluated by a shell'

capture=$tmp/issue-create.argv
expected=$tmp/issue-create.expected
run_helper "$capture" issue-create 'body safety' "$body"
write_expected "$expected" issue create --title 'body safety' --body-file "$body"
expect_argv "$capture" "$expected"

capture=$tmp/pr-create.argv
expected=$tmp/pr-create.expected
run_helper "$capture" pr-create main codex/body-safety 'body safety' "$body"
write_expected "$expected" pr create --base main --head codex/body-safety --title 'body safety' --body-file "$body"
expect_argv "$capture" "$expected"

capture=$tmp/invalid.argv
expect_no_gh_on_failure "$capture" issue-comment -1 "$body"
expect_no_gh_on_failure "$capture" issue-create 'body safety' "$tmp/missing"
expect_no_gh_on_failure "$capture" pr-create -main codex/body-safety 'body safety' "$body"
expect_no_gh_on_failure "$capture" issue-comment 123 "$tmp"

fifo=$tmp/body.fifo
mkfifo "$fifo"
expect_no_gh_on_failure "$capture" issue-comment 123 "$fifo"

if [ "$(id -u)" -ne 0 ]; then
    unreadable=$tmp/unreadable.md
    cp "$body" "$unreadable"
    chmod 000 "$unreadable"
    expect_no_gh_on_failure "$capture" issue-comment 123 "$unreadable"
fi

expect_no_gh_on_failure "$capture" unknown 123 "$body"
expect_no_gh_on_failure "$capture" issue-comment 123

echo 'PASS: GitHub body helper'
