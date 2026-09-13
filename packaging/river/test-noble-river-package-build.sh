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
cat >"$tmpdir/bin/sudo" <<'EOF'
#!/bin/sh
set -eu
printf 'sudo=%s\n' "$*" >>"${REALM_TEST_LOG:?}"
[ "$1" = --non-interactive ]
shift
if [ "$1" = true ]; then
    exit 0
fi
exec "$@"
EOF
cat >"$tmpdir/bin/unshare" <<'EOF'
#!/bin/sh
set -eu
printf 'unshare=%s\n' "$*" >>"${REALM_TEST_LOG:?}"
[ "$1" = --net ]
[ "$2" = -- ]
shift 2
exec "$@"
EOF
cat >"$tmpdir/bin/dpkg-buildpackage" <<'EOF'
#!/bin/sh
set -eu
printf 'cwd=%s\n' "$PWD" >>"${REALM_TEST_LOG:?}"
printf 'parent=%s\n' "${REALM_PARENT_NETNS:?}" >>"${REALM_TEST_LOG:?}"
printf 'dpkg=%s\n' "$*" >>"${REALM_TEST_LOG:?}"
EOF
chmod +x "$tmpdir/bin/sudo" "$tmpdir/bin/unshare" \
    "$tmpdir/bin/dpkg-buildpackage"

PATH="$tmpdir/bin:/usr/bin:/bin" REALM_TEST_LOG="$tmpdir/log" \
    "$builder" "$tmpdir/source"

grep -Fx 'sudo=--non-interactive true' "$tmpdir/log" >/dev/null
grep -Fx 'sudo=--non-interactive unshare --net -- env' "$tmpdir/log" >/dev/null && {
    echo 'package-build entrypoint omitted the command after env' >&2
    exit 1
}
grep -F 'sudo=--non-interactive unshare --net -- env REALM_PARENT_NETNS=' \
    "$tmpdir/log" >/dev/null
grep -F 'unshare=--net -- env REALM_PARENT_NETNS=' \
    "$tmpdir/log" >/dev/null
grep -Fx "cwd=$tmpdir/source" "$tmpdir/log" >/dev/null
grep -E '^parent=net:\[[0-9]+\]$' "$tmpdir/log" >/dev/null
grep -Fx 'dpkg=-us -uc -b' "$tmpdir/log" >/dev/null

echo 'PASS: Noble River package-build namespace entrypoint fixture'
