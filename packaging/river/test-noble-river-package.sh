#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-noble-river-package.py"
control="$repo_root/packaging/debian-river/control"
rules="$repo_root/packaging/debian-river/rules"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

[[ -x "$checker" ]] || {
    echo "Noble River package checker is missing" >&2
    exit 1
}
debian_dollar='$'
for expected in \
    'Source: realm-river' \
    'Package: realm-river' \
    'Architecture: amd64' \
    'Provides: river (= 0.4.8)' \
    "Depends: ${debian_dollar}{shlibs:Depends}, ${debian_dollar}{misc:Depends}, libinput-bin (>= 1.25.0)"; do
    grep -Fx -- "$expected" "$control" >/dev/null || {
        echo "Noble River control omits: $expected" >&2
        exit 1
    }
done
grep -F 'REALM_LOGICAL_PREFIX := /usr/lib/realm' "$rules" >/dev/null || {
    echo 'Noble River rules omit the logical private prefix' >&2
    exit 1
}

make_package() {
    local root=$1 package=$2
    mkdir -p "$root/DEBIAN" "$root/usr/lib/realm/bin" \
        "$root/usr/lib/realm/lib" "$root/usr/lib/realm/share/libinput"
    cat >"$root/DEBIAN/control" <<'EOF'
Package: realm-river
Version: 0.4.8-1
Architecture: amd64
Maintainer: realm contributors <vadim.evard@gmail.com>
Provides: river (= 0.4.8)
Depends: libinput-bin (>= 1.25.0)
Description: fixture private River closure
EOF
    : >"$root/usr/lib/realm/bin/river"
    chmod +x "$root/usr/lib/realm/bin/river"
    for library in \
        libwlroots-0.20.so.20 libwayland-server.so.0 libwayland-client.so.0 \
        libdrm.so.2 libinput.so.10 libpixman-1.so.0 libxkbcommon.so.0 \
        libdisplay-info.so.0; do
        : >"$root/usr/lib/realm/lib/$library"
    done
    : >"$root/usr/lib/realm/share/libinput/10-generic-keyboard.quirks"
    dpkg-deb --build "$root" "$package" >/dev/null
}

make_package "$tmpdir/canonical" "$tmpdir/canonical.deb"
"$checker" "$tmpdir/canonical.deb"

cp -R "$tmpdir/canonical" "$tmpdir/usr-bin"
mkdir -p "$tmpdir/usr-bin/usr/bin"
: >"$tmpdir/usr-bin/usr/bin/river"
dpkg-deb --build "$tmpdir/usr-bin" "$tmpdir/usr-bin.deb" >/dev/null
if "$checker" "$tmpdir/usr-bin.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted /usr/bin/river" >&2
    exit 1
elif ! grep -F 'payload path outside private runtime' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

cp -R "$tmpdir/canonical" "$tmpdir/development"
mkdir -p "$tmpdir/development/usr/lib/realm/include"
: >"$tmpdir/development/usr/lib/realm/include/libinput.h"
dpkg-deb --build "$tmpdir/development" "$tmpdir/development.deb" >/dev/null
if "$checker" "$tmpdir/development.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted development files" >&2
    exit 1
elif ! grep -F 'development or build payload is forbidden' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

cp -R "$tmpdir/canonical" "$tmpdir/udev"
mkdir -p "$tmpdir/udev/usr/lib/udev/rules.d"
: >"$tmpdir/udev/usr/lib/udev/rules.d/80-libinput-device-groups.rules"
dpkg-deb --build "$tmpdir/udev" "$tmpdir/udev.deb" >/dev/null
if "$checker" "$tmpdir/udev.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted a global udev rule" >&2
    exit 1
elif ! grep -F 'payload path outside private runtime' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

cp -R "$tmpdir/canonical" "$tmpdir/missing"
rm "$tmpdir/missing/usr/lib/realm/lib/libinput.so.10"
dpkg-deb --build "$tmpdir/missing" "$tmpdir/missing.deb" >/dev/null
if "$checker" "$tmpdir/missing.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted a missing private library family" >&2
    exit 1
elif ! grep -F 'required private payload absent: libinput' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

echo 'PASS: Noble River package projection fixtures'
