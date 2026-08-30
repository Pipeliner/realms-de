#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
guard="$root/docs/check-contribution-templates.py"

[ -x "$guard" ] || {
    echo "FAIL: missing contribution-template guard" >&2
    exit 1
}

tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-template-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
fixture="$tmp/fixture"
make_fixture() {
    target=$1
    mkdir -p "$target/.github/ISSUE_TEMPLATE"
    cp "$root/.github/ISSUE_TEMPLATE/work-item.yml" "$target/.github/ISSUE_TEMPLATE/"
    cp "$root/.github/ISSUE_TEMPLATE/needs-human.yml" "$target/.github/ISSUE_TEMPLATE/"
    cp "$root/.github/ISSUE_TEMPLATE/bug.yml" "$target/.github/ISSUE_TEMPLATE/"
    cp "$root/.github/PULL_REQUEST_TEMPLATE.md" "$target/.github/"
}
expect_fail() {
    label=$1
    target=$2
    if python3 "$guard" --root "$target" 2>&1; then
        echo "FAIL: $label mutation unexpectedly passed" >&2
        exit 1
    fi
}
make_fixture "$fixture"

python3 "$guard" --root "$fixture"

sed '/label: "Source:"/d' "$fixture/.github/ISSUE_TEMPLATE/work-item.yml" >"$fixture/work.next"
mv "$fixture/work.next" "$fixture/.github/ISSUE_TEMPLATE/work-item.yml"
expect_fail work-item-contract "$fixture"

fixture="$tmp/human"
make_fixture "$fixture"
sed 's/^    id: blocked$/    id: options/' "$fixture/.github/ISSUE_TEMPLATE/needs-human.yml" >"$fixture/human.next"
mv "$fixture/human.next" "$fixture/.github/ISSUE_TEMPLATE/needs-human.yml"
expect_fail needs-human-contract "$fixture"

fixture="$tmp/pr"
make_fixture "$fixture"
sed '/Closes #/d' "$fixture/.github/PULL_REQUEST_TEMPLATE.md" >"$fixture/pr.next"
mv "$fixture/pr.next" "$fixture/.github/PULL_REQUEST_TEMPLATE.md"
expect_fail pull-request-contract "$fixture"

fixture="$tmp/bug"
make_fixture "$fixture"
sed '/id: missed-guard/,+5d' "$fixture/.github/ISSUE_TEMPLATE/bug.yml" >"$fixture/bug.next"
mv "$fixture/bug.next" "$fixture/.github/ISSUE_TEMPLATE/bug.yml"
expect_fail bug-guard-contract "$fixture"

echo 'PASS: contribution template fixtures'
