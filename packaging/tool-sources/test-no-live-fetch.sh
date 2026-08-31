#!/bin/sh
# A deliberately narrow regression guard. It does not establish that a native
# package consumes retained inputs or a locked Cargo closure; SPEC 0023 A2 does.
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
if grep -n -E -i '(curl|wget|git[[:space:]]+clone)' \
    "$root/packaging/debian/rules" "$root/packaging/fedora/helm.spec"; then
    echo 'native package construction contains a live upstream acquisition path' >&2
    exit 1
fi
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-offline-policy.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
cp "$root/packaging/debian/rules" "$tmp/rules"
printf '\ncurl https://upstream.invalid/source.tar.gz\n' >>"$tmp/rules"
if ! grep -q -E -i '(curl|wget|git[[:space:]]+clone)' "$tmp/rules"; then
    echo 'hostile live-fetch fixture was not detected' >&2
    exit 1
fi
