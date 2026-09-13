#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
builder="$repo_root/packaging/river/run-noble-river-package-build.sh"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

[[ -x "$builder" ]] || {
    echo 'Noble River package-build entrypoint is missing' >&2
    exit 1
}

mkdir -p "$tmpdir/bin" "$tmpdir/source/debian"
: >"$tmpdir/source/debian/control"
: >"$tmpdir/log"
cat >"$tmpdir/bin/unshare" <<'EOF'
#!/bin/sh
set -eu
printf 'unshare=%s\n' "$*" >>"${REALM_TEST_LOG:?}"
[ "$1" = --user ]
[ "$2" = --map-root-user ]
[ "$3" = --net ]
[ "$4" = -- ]
shift 4
exec "$@"
EOF
cat >"$tmpdir/bin/dpkg-buildpackage" <<'EOF'
#!/bin/sh
set -eu
printf 'cwd=%s\n' "$PWD" >>"${REALM_TEST_LOG:?}"
printf 'parent=%s\n' "${REALM_PARENT_NETNS:?}" >>"${REALM_TEST_LOG:?}"
printf 'dpkg=%s\n' "$*" >>"${REALM_TEST_LOG:?}"
EOF
chmod +x "$tmpdir/bin/unshare" "$tmpdir/bin/dpkg-buildpackage"

PATH="$tmpdir/bin:/usr/bin:/bin" REALM_TEST_LOG="$tmpdir/log" \
    "$builder" "$tmpdir/source"

grep -Fx 'unshare=--user --map-root-user --net -- env' "$tmpdir/log" >/dev/null && {
    echo 'package-build entrypoint omitted the command after env' >&2
    exit 1
}
grep -F 'unshare=--user --map-root-user --net -- env REALM_PARENT_NETNS=' \
    "$tmpdir/log" >/dev/null
grep -Fx "cwd=$tmpdir/source" "$tmpdir/log" >/dev/null
grep -E '^parent=net:\[[0-9]+\]$' "$tmpdir/log" >/dev/null
grep -Fx 'dpkg=-us -uc -b' "$tmpdir/log" >/dev/null

echo 'PASS: Noble River package-build namespace entrypoint fixture'
