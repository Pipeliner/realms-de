#!/bin/sh
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
checker=$root/packaging/tool-sources/check-intake.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-source-intake.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
cp -R "$root/packaging/tool-sources" "$tmp/sources"

"$checker" "$tmp/sources"

printf x >"$tmp/sources/inputs/unrecorded.tar.gz"
if "$checker" "$tmp/sources" >"$tmp/out" 2>"$tmp/err"; then
    echo "unrecorded retained source unexpectedly passed" >&2
    exit 1
fi
grep -F 'retained input inventory differs from manifest' "$tmp/err"
rm "$tmp/sources/inputs/unrecorded.tar.gz"

mv "$tmp/sources/inputs/yazi-v26.8.15.tar.gz" "$tmp/sources/inputs/yazi-source"
ln -s yazi-source "$tmp/sources/inputs/yazi-v26.8.15.tar.gz"
if "$checker" "$tmp/sources" >"$tmp/out" 2>"$tmp/err"; then
    echo "symlinked retained source unexpectedly passed" >&2
    exit 1
fi
grep -F 'yazi: retained source must be a regular non-symlink file' "$tmp/err"
rm "$tmp/sources/inputs/yazi-v26.8.15.tar.gz"
mv "$tmp/sources/inputs/yazi-source" "$tmp/sources/inputs/yazi-v26.8.15.tar.gz"

printf x >>"$tmp/sources/inputs/yazi-v26.8.15.tar.gz"
if "$checker" "$tmp/sources" >"$tmp/out" 2>"$tmp/err"; then
    echo "corrupt retained source unexpectedly passed" >&2
    exit 1
fi
grep -F 'yazi: SHA-256 mismatch' "$tmp/err"
