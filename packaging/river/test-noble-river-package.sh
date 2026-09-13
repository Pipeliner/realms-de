#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-noble-river-package.py"
closure_builder="$repo_root/packaging/river/build-noble-package-closure.sh"
pc_relocator="$repo_root/packaging/river/relocate-noble-package-pkgconfig.py"
control="$repo_root/packaging/debian-river/control"
rules="$repo_root/packaging/debian-river/rules"
realm_control="$repo_root/packaging/debian/control"
workflow="$repo_root/.github/workflows/distro.yml"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

[[ -x "$checker" ]] || {
    echo "Noble River package checker is missing" >&2
    exit 1
}
[[ -x "$pc_relocator" ]] || {
    echo "Noble River staged pkg-config relocator is missing" >&2
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
grep -F 'realm-river (>= 0.4.8) | river (>= 0.4.0),' "$realm_control" >/dev/null || {
    echo 'Realm control does not require the selected private River version' >&2
    exit 1
}
river_job=$(sed -n '/^  ubuntu-river-debian:/,/^  ubuntu-debian-package:/p' "$workflow")
printf '%s\n' "$river_job" | grep -Fx '            zstd' >/dev/null || {
    echo 'Noble River package job omits the Realm zstd build dependency' >&2
    exit 1
}
if printf '%s\n' "$river_job" | \
    grep -F 'dpkg-buildpackage -us -uc -b -d' >/dev/null; then
    echo 'Noble River package job bypasses declared build dependency validation' >&2
    exit 1
fi
grep -F 'REALM_PARENT_NETNS' "$rules" >/dev/null || {
    echo 'Noble River rules do not require the outer namespace identity' >&2
    exit 1
}
if grep -F 'export PKG_CONFIG_SYSROOT_DIR=' "$closure_builder" >/dev/null; then
    echo 'Noble River closure rewrites host pkg-config paths beneath staging' >&2
    exit 1
fi
grep -F "\"\$pc_relocator\" \"\$prefix\" \"\$staged_prefix\"" \
    "$closure_builder" >/dev/null || {
    echo 'Noble River closure omits staged private pkg-config relocation' >&2
    exit 1
}
for staged_search in "export C_INCLUDE_PATH=\"\$staged_prefix/include\"" \
    "export LIBRARY_PATH=\"\$staged_prefix/lib\""; do
    grep -F "$staged_search" "$closure_builder" >/dev/null || {
        echo "Noble River closure omits staged search path: $staged_search" >&2
        exit 1
    }
done
if grep -F 'unshare --user --map-root-user --net' "$rules" >/dev/null; then
    echo 'Noble River rules create the user namespace too late under dpkg' >&2
    exit 1
fi
if ! printf '%s\n' "$river_job" | grep -F \
    '/packaging/river/run-noble-river-package-build.sh"' >/dev/null \
    || ! printf '%s\n' "$river_job" | \
        grep -Fx "            \"${debian_dollar}RUNNER_TEMP/realm-river-0.4.8\"" \
        >/dev/null; then
    echo 'Noble River job bypasses the retained package-build entrypoint' >&2
    exit 1
fi

pc_stage="$tmpdir/pkgconfig-stage/usr/lib/realm"
host_pc="$tmpdir/host/lib/x86_64-linux-gnu/pkgconfig/libevdev.pc"
mkdir -p "$pc_stage/lib/pkgconfig" "$pc_stage/share/pkgconfig" \
    "$(dirname "$host_pc")"
cat >"$pc_stage/lib/pkgconfig/libdrm.pc" <<'EOF'
prefix=/usr/lib/realm
includedir=${prefix}/include/libdrm
libdir=${prefix}/lib
EOF
cat >"$pc_stage/share/pkgconfig/wayland-scanner.pc" <<'EOF'
prefix=/usr/lib/realm
wayland_scanner=${prefix}/bin/wayland-scanner
pkgdatadir=${prefix}/share/wayland
EOF
cat >"$host_pc" <<'EOF'
prefix=/usr
includedir=${prefix}/include/libevdev-1.0
libdir=${prefix}/lib/x86_64-linux-gnu
EOF
cp "$host_pc" "$tmpdir/host.pc.before"
"$pc_relocator" /usr/lib/realm "$pc_stage"
grep -Fx "prefix=$pc_stage" "$pc_stage/lib/pkgconfig/libdrm.pc" >/dev/null
grep -Fx "includedir=\${prefix}/include/libdrm" \
    "$pc_stage/lib/pkgconfig/libdrm.pc" >/dev/null
grep -Fx "wayland_scanner=\${prefix}/bin/wayland-scanner" \
    "$pc_stage/share/pkgconfig/wayland-scanner.pc" >/dev/null
if grep -R -Fx 'prefix=/usr/lib/realm' "$pc_stage/lib/pkgconfig" \
    "$pc_stage/share/pkgconfig" >/dev/null; then
    echo 'private staged pkg-config metadata retained its logical path' >&2
    exit 1
fi
cmp "$tmpdir/host.pc.before" "$host_pc"

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

cp -R "$tmpdir/canonical" "$tmpdir/private-dependency"
sed -i \
    's/^Depends:.*/Depends: libinput-bin (>= 1.25.0), libwlroots-0.20-0 (= 0.20.0)/' \
    "$tmpdir/private-dependency/DEBIAN/control"
dpkg-deb --build "$tmpdir/private-dependency" "$tmpdir/private-dependency.deb" >/dev/null
if "$checker" "$tmpdir/private-dependency.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted a dependency on a bundled private library" >&2
    exit 1
elif ! grep -F 'dependency on bundled private library is forbidden: libwlroots-0.20-0' \
    "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

for companion in \
    libdrm-amdgpu1 \
    libdrm-intel1 \
    libdrm-nouveau2 \
    libdrm-radeon1 \
    libwayland-cursor0 \
    libwayland-egl1; do
    cp -R "$tmpdir/canonical" "$tmpdir/$companion"
    sed -i \
        "s/^Depends:.*/Depends: libinput-bin (>= 1.25.0), $companion/" \
        "$tmpdir/$companion/DEBIAN/control"
    dpkg-deb --build "$tmpdir/$companion" "$tmpdir/$companion.deb" >/dev/null
    if "$checker" "$tmpdir/$companion.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
        echo "package checker accepted a dependency on bundled $companion" >&2
        exit 1
    elif ! grep -F \
        "dependency on bundled private library is forbidden: $companion" \
        "$tmpdir/err" >/dev/null; then
        cat "$tmpdir/err" >&2
        exit 1
    fi
done

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

cp -R "$tmpdir/canonical" "$tmpdir/build-tree"
mkdir -p "$tmpdir/build-tree/usr/lib/realm/build-cache"
: >"$tmpdir/build-tree/usr/lib/realm/build-cache/object.o"
dpkg-deb --build "$tmpdir/build-tree" "$tmpdir/build-tree.deb" >/dev/null
if "$checker" "$tmpdir/build-tree.deb" >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "package checker accepted an arbitrary private-prefix build tree" >&2
    exit 1
elif ! grep -F 'payload path is not an allowed runtime file' "$tmpdir/err" >/dev/null; then
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
