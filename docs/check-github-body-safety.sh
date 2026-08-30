#!/bin/sh
set -eu

usage() {
    echo "usage: $0 [--root REPOSITORY]" >&2
    exit 64
}

root=.
case "$#" in
    0) ;;
    2)
        [ "$1" = --root ] || usage
        root=$2
        ;;
    *) usage ;;
esac

root=$(CDPATH='' cd "$root" && pwd)
git -C "$root" rev-parse --is-inside-work-tree >/dev/null 2>&1 || {
    echo 'FAIL: root is not a Git worktree' >&2
    exit 2
}

fail() {
    echo "FAIL: GitHub body safety: $1" >&2
    exit 1
}

helper=$root/scripts/gh-body-file
[ -f "$helper" ] || fail 'missing approved helper'

for line in \
    "        exec gh issue create --title \"\$title\" --body-file \"\$body_file\"" \
    "        exec gh issue comment \"\$issue\" --body-file \"\$body_file\"" \
    "        exec gh pr create --base \"\$base\" --head \"\$head\" --title \"\$title\" --body-file \"\$body_file\""; do
    grep -F -x -q "$line" "$helper" || fail 'helper command surface changed'
done

helper_gh_count=$(grep -c -E '^[[:space:]]*exec gh[[:space:]]' "$helper" || true)
[ "$helper_gh_count" -eq 3 ] || fail 'helper has an unapproved GitHub CLI invocation'
if grep -n -E '(^|[[:space:];])eval[[:space:]]|<<' "$helper" >/dev/null ||
    grep -F -q "\$(" "$helper" ||
    grep -F -q '`' "$helper"; then
    fail 'helper dynamically constructs a shell command'
fi

is_command_surface() {
    file=$1
    path=$2
    case "$file" in
        docs/fixtures/* | docs/check-github-body-safety.sh)
            return 1
            ;;
        *.sh | scripts/* | .github/workflows/*.yml | .github/workflows/*.yaml | .github/actions/* | Makefile | makefile | GNUmakefile | justfile | *.mk)
            return 0
            ;;
        *) ;;
    esac

    first_line=
    IFS= read -r first_line <"$path" || true
    case "$first_line" in
        '#!'*) return 0 ;;
        *) return 1 ;;
    esac
}

git -C "$root" ls-files | while IFS= read -r file; do
    path=$root/$file
    [ -f "$path" ] || continue
    is_command_surface "$file" "$path" || continue
    [ "$file" = scripts/gh-body-file ] && continue

    if grep -n -E "(^|[^A-Za-z0-9_-])gh[[:space:]]|['\"]gh['\"][[:space:]]" "$path" >/dev/null; then
        fail "direct GitHub CLI invocation in $file"
    fi
    if grep -F -q -e 'g"h' -e "g'h" -e 'g\h' "$path"; then
        fail "obfuscated GitHub CLI invocation in $file"
    fi
    if grep -n -E -- '--body([[:space:]=]|$)' "$path" >/dev/null; then
        fail "unsafe inline GitHub body option in $file"
    fi
    if grep -n -E "(^|[[:space:];])([A-Za-z_][A-Za-z0-9_]*=)?gh([[:space:];]|$)|[A-Za-z_][A-Za-z0-9_]*=[[:space:]]*['\"]?([^[:space:]'\"]*/)?gh['\"]?([[:space:];]|$)|[\$][(][[:space:]]*(command[[:space:]]+-v|which)[[:space:]]+gh([[:space:])]|$)" "$path" >/dev/null; then
        fail "indirect GitHub CLI invocation in $file"
    fi
    if grep -n -E '(^|[;&|)]|then[[:space:]]|do[[:space:]])[[:space:]]*eval[[:space:]]' "$path" >/dev/null; then
        fail "dynamic shell evaluation in $file"
    fi
    if grep -F -q -e 'e"val' -e "e'val" -e 'e\val' "$path"; then
        fail "obfuscated dynamic shell evaluation in $file"
    fi
    if grep -n -E 'api\.github\.com|github-script|actions/github-script|uses:[[:space:]]+[^@[:space:]]*(comment|issue|pull-request)[^@[:space:]]*@' "$path" >/dev/null; then
        fail "direct GitHub API or action publication surface in $file"
    fi
done

echo 'PASS: GitHub body safety guard'
