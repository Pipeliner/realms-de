#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
guard=$root/docs/check-github-body-safety.sh
fixtures=$root/docs/fixtures/github-body-safety
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

make_fixture() {
    fixture=$1
    mkdir -p "$fixture/scripts" "$fixture/.github/workflows" "$fixture/docs"
    cp "$root/scripts/gh-body-file" "$fixture/scripts/gh-body-file"
    printf '%s%s%s%s\n' '# allowed documentation prose: g' 'h issue comment --bo' 'dy "$' 'text"' >"$fixture/docs/policy.md"
    printf '%s\n' 'name: fixture' >"$fixture/.github/workflows/ci.yml"
    git -C "$fixture" init -q
    git -C "$fixture" add .
}

expect_pass() {
    fixture=$1
    "$guard" --root "$fixture" || fail 'safe tracked command surface was rejected'
}

expect_fail() {
    fixture=$1
    if "$guard" --root "$fixture" >/dev/null 2>&1; then
        fail 'unsafe tracked command surface was accepted'
    fi
}

fixture=$tmp/repository
make_fixture "$fixture"
expect_pass "$fixture"

directive_prefix='shellcheck dis'
directive_suffix='able'
if grep -F -q "$directive_prefix$directive_suffix" "$guard" ||
    grep -F -q "$directive_prefix$directive_suffix" "$root/docs/test-github-body-safety.sh"; then
    fail 'body-safety guard and fixture test must encode literal syntax without ShellCheck suppressions'
fi

for command in \
    './docs/test-gh-body-file.sh' \
    './docs/test-github-body-safety.sh' \
    './docs/check-github-body-safety.sh'; do
    grep -F -q "$command" "$root/.github/workflows/ci.yml" ||
        fail "CI does not run $command"
done

cp "$fixtures/direct-gh.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/direct-gh-equals.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/quoted-direct-gh.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/concatenated-gh.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/concatenated-gh-short.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/gh-api.sh" "$fixture/scripts/unsafe"
chmod +x "$fixture/scripts/unsafe"
git -C "$fixture" add scripts/unsafe
expect_fail "$fixture"
git -C "$fixture" rm -q -f scripts/unsafe

cp "$fixtures/variable-gh.sh" "$fixture/docs/publish.sh"
chmod +x "$fixture/docs/publish.sh"
git -C "$fixture" add docs/publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f docs/publish.sh

cp "$fixtures/quoted-gh.sh" "$fixture/docs/publish.sh"
chmod +x "$fixture/docs/publish.sh"
git -C "$fixture" add docs/publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f docs/publish.sh

cp "$fixtures/path-gh.sh" "$fixture/docs/publish.sh"
chmod +x "$fixture/docs/publish.sh"
git -C "$fixture" add docs/publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f docs/publish.sh

cp "$fixtures/dynamic-eval.sh" "$fixture/packaging-publish.sh"
chmod +x "$fixture/packaging-publish.sh"
git -C "$fixture" add packaging-publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f packaging-publish.sh

cp "$fixtures/then-eval.sh" "$fixture/packaging-publish.sh"
chmod +x "$fixture/packaging-publish.sh"
git -C "$fixture" add packaging-publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f packaging-publish.sh

cp "$fixtures/case-eval.sh" "$fixture/packaging-publish.sh"
chmod +x "$fixture/packaging-publish.sh"
git -C "$fixture" add packaging-publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f packaging-publish.sh

cp "$fixtures/concatenated-eval.sh" "$fixture/packaging-publish.sh"
chmod +x "$fixture/packaging-publish.sh"
git -C "$fixture" add packaging-publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f packaging-publish.sh

cp "$fixtures/command-substitution-gh.sh" "$fixture/docs/publish.sh"
chmod +x "$fixture/docs/publish.sh"
git -C "$fixture" add docs/publish.sh
expect_fail "$fixture"
git -C "$fixture" rm -q -f docs/publish.sh

cp "$fixtures/authoring-action.yml" "$fixture/.github/workflows/ci.yml"
git -C "$fixture" add .github/workflows/ci.yml
expect_fail "$fixture"
printf '%s\n' 'name: fixture' >"$fixture/.github/workflows/ci.yml"
git -C "$fixture" add .github/workflows/ci.yml

mkdir -p "$fixture/.github/actions/publish"
cp "$fixtures/composite-action.yml" "$fixture/.github/actions/publish/action.yml"
git -C "$fixture" add .github/actions/publish/action.yml
expect_fail "$fixture"
git -C "$fixture" rm -q -f .github/actions/publish/action.yml

mkdir -p "$fixture/tools"
cp "$fixtures/extensionless-publish" "$fixture/tools/publish"
chmod +x "$fixture/tools/publish"
git -C "$fixture" add tools/publish
expect_fail "$fixture"

echo 'PASS: GitHub body safety guard'
