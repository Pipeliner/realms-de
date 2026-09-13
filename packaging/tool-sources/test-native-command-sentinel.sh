#!/bin/sh
# The Starship build may encounter only pinned, denied VCS metadata attempts.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
sentinel=$root/packaging/tool-sources/native-command-sentinel.sh

if [ ! -x "$sentinel" ]; then
    echo "native command sentinel is missing or not executable" >&2
    exit 1
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-native-command-sentinel.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir -p "$tmp/bin" "$tmp/starship"
ln -s "$sentinel" "$tmp/bin/git"
log=$tmp/sentinel.log

for args in \
    'status --porcelain' \
    'status --porcelain --untracked-files=all' \
    'rev-parse HEAD' \
    'rev-parse --short HEAD' \
    'log -1 --pretty=format:%an' \
    'log -1 --pretty=format:%ae' \
    'show --pretty=format:%ct --date=raw -s' \
    'tag -l --contains HEAD' \
    'describe --tags --abbrev=0 HEAD' \
    'describe --tags HEAD' \
    'symbolic-ref --short HEAD'; do
    (
        cd "$tmp/starship"
        # Intentional splitting: these are fixed pinned fixture arguments.
        # shellcheck disable=SC2086
        REALM_SENTINEL_LOG="$log" \
            REALM_EXPECTED_STARSHIP_SOURCE="$tmp/starship" \
            "$tmp/bin/git" $args
    ) && {
        echo "metadata sentinel unexpectedly executed Git: $args" >&2
        exit 1
    }
done

if [ "$(grep -c '^git-metadata-denied|' "$log")" -ne 11 ] \
    || grep '^forbidden|' "$log" >/dev/null; then
    echo "metadata sentinel did not classify the exact read-only commands" >&2
    exit 1
fi

(
    cd "$tmp/starship"
    REALM_SENTINEL_LOG="$log" \
        REALM_EXPECTED_STARSHIP_SOURCE="$tmp/starship" \
        "$tmp/bin/git" 'status --porcelain'
) && {
    echo "metadata sentinel unexpectedly collapsed one Git argument into two" >&2
    exit 1
}
grep -F -x \
    "forbidden|command=git|cwd=$tmp/starship|argc=1|args=status --porcelain" \
    "$log" >/dev/null

(
    cd "$tmp/starship"
    REALM_SENTINEL_LOG="$log" \
        REALM_EXPECTED_STARSHIP_SOURCE="$tmp/starship" \
        "$tmp/bin/git" fetch https://example.invalid/realm
) && {
    echo "metadata sentinel unexpectedly executed Git fetch" >&2
    exit 1
}
grep -F -x \
    "forbidden|command=git|cwd=$tmp/starship|argc=2|args=fetch https://example.invalid/realm" \
    "$log" >/dev/null

echo "PASS: native command sentinel boundaries"
