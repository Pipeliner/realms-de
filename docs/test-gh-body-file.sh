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
title_file=$fixture/literal-title.txt
dollar='$'
title="Literal \`title\` ${dollar}(touch /tmp/gh-body-file-escaped) \"quotes\" and * glob"
escape_marker=/tmp/gh-body-file-escaped
rm -f "$escape_marker"

capture=$tmp/issue-comment.argv
expected=$tmp/issue-comment.expected
run_helper "$capture" issue-comment 123 "$body"
write_expected "$expected" issue comment 123 --body-file "$body"
expect_argv "$capture" "$expected"
[ ! -e "$escape_marker" ] || fail 'literal body was evaluated by a shell'

capture=$tmp/issue-comment-edit.argv
expected=$tmp/issue-comment-edit.expected
run_helper "$capture" issue-comment-edit 456 "$body"
write_expected "$expected" api --method PATCH 'repos/{owner}/{repo}/issues/comments/456' --field "body=@$body"
expect_argv "$capture" "$expected"
[ ! -e "$escape_marker" ] || fail 'literal comment-edit body was evaluated by a shell'

capture=$tmp/issue-create.argv
expected=$tmp/issue-create.expected
run_helper "$capture" issue-create 'body safety' "$body"
write_expected "$expected" issue create --title 'body safety' --body-file "$body"
expect_argv "$capture" "$expected"

capture=$tmp/issue-edit.argv
expected=$tmp/issue-edit.expected
run_helper "$capture" issue-edit 123 "$title_file" "$body"
write_expected "$expected" issue edit 123 --title "$title" --body-file "$body"
expect_argv "$capture" "$expected"
[ ! -e "$escape_marker" ] || fail 'literal title was evaluated by a shell'

capture=$tmp/pr-create.argv
expected=$tmp/pr-create.expected
run_helper "$capture" pr-create main codex/body-safety 'body safety' "$body"
write_expected "$expected" pr create --base main --head codex/body-safety --title 'body safety' --body-file "$body"
expect_argv "$capture" "$expected"

capture=$tmp/invalid.argv
expect_no_gh_on_failure "$capture" issue-comment -1 "$body"
expect_no_gh_on_failure "$capture" issue-comment-edit -1 "$body"
expect_no_gh_on_failure "$capture" issue-comment 00 "$body"
expect_no_gh_on_failure "$capture" issue-comment-edit 00 "$body"
expect_no_gh_on_failure "$capture" issue-create 'body safety' "$tmp/missing"
expect_no_gh_on_failure "$capture" pr-create -main codex/body-safety 'body safety' "$body"
expect_no_gh_on_failure "$capture" issue-comment 123 "$tmp"
expect_no_gh_on_failure "$capture" issue-comment-edit 456 "$tmp/missing"
expect_no_gh_on_failure "$capture" issue-edit 123 "$tmp/missing" "$body"
expect_no_gh_on_failure "$capture" issue-edit 00 "$title_file" "$body"
expect_no_gh_on_failure "$capture" issue-edit 123 "$title_file" "$tmp/missing"

empty_title=$tmp/empty-title.txt
: >"$empty_title"
expect_no_gh_on_failure "$capture" issue-edit 123 "$empty_title" "$body"

unterminated_title=$tmp/unterminated-title.txt
printf '%s' 'unterminated title' >"$unterminated_title"
expect_no_gh_on_failure "$capture" issue-edit 123 "$unterminated_title" "$body"

multiline_title=$tmp/multiline-title.txt
printf '%s\n%s\n' 'first title line' 'second title line' >"$multiline_title"
expect_no_gh_on_failure "$capture" issue-edit 123 "$multiline_title" "$body"

crlf_title=$tmp/crlf-title.txt
printf '%s\r\n' 'CRLF title' >"$crlf_title"
expect_no_gh_on_failure "$capture" issue-edit 123 "$crlf_title" "$body"

nul_title=$tmp/nul-title.txt
printf '%s\0%s\n' 'NUL' 'title' >"$nul_title"
expect_no_gh_on_failure "$capture" issue-edit 123 "$nul_title" "$body"

hyphen_title=-crlf-title.txt
printf '%s\r\n' 'hyphen title' >"$tmp/$hyphen_title"
if (cd "$tmp" && run_helper "$capture" issue-edit 123 "$hyphen_title" "$body"); then
    fail 'hyphen-prefixed CRLF title invocation succeeded'
fi
[ ! -e "$capture" ] || fail 'hyphen-prefixed title invocation reached GitHub CLI'

fifo=$tmp/body.fifo
mkfifo "$fifo"
expect_no_gh_on_failure "$capture" issue-comment 123 "$fifo"
expect_no_gh_on_failure "$capture" issue-comment-edit 456 "$fifo"

hyphen_body=-comment-body.md
cp "$body" "$tmp/$hyphen_body"
capture=$tmp/hyphen-comment-edit.argv
expected=$tmp/hyphen-comment-edit.expected
(cd "$tmp" && run_helper "$capture" issue-comment-edit 456 "$hyphen_body")
write_expected "$expected" api --method PATCH 'repos/{owner}/{repo}/issues/comments/456' --field "body=@$hyphen_body"
expect_argv "$capture" "$expected"
[ ! -e "$escape_marker" ] || fail 'hyphen-prefixed body was evaluated by a shell'
capture=$tmp/invalid.argv

title_fifo=$tmp/title.fifo
mkfifo "$title_fifo"
expect_no_gh_on_failure "$capture" issue-edit 123 "$title_fifo" "$body"

if [ "$(id -u)" -ne 0 ]; then
    unreadable=$tmp/unreadable.md
    cp "$body" "$unreadable"
    chmod 000 "$unreadable"
    expect_no_gh_on_failure "$capture" issue-comment 123 "$unreadable"
    expect_no_gh_on_failure "$capture" issue-comment-edit 456 "$unreadable"
    expect_no_gh_on_failure "$capture" issue-edit 123 "$unreadable" "$body"
fi

expect_no_gh_on_failure "$capture" unknown 123 "$body"
expect_no_gh_on_failure "$capture" issue-comment 123
expect_no_gh_on_failure "$capture" issue-comment-edit 456
expect_no_gh_on_failure "$capture" issue-edit 123 "$title_file"

echo 'PASS: GitHub body helper'
