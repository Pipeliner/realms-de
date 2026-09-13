#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 MANIFEST CLOSURE-INPUT STAGING-ROOT PARENT-NETNS" >&2
    exit 2
fi

manifest=$(realpath "$1")
closure_input=$(realpath "$2")
staging=$3
parent_netns=$4
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-closure-manifest.py"
pc_relocator="$repo_root/packaging/river/relocate-noble-package-pkgconfig.py"

current_netns=$(readlink /proc/self/ns/net)
if [[ "$current_netns" == "$parent_netns" ]]; then
    echo "closure build: network namespace was not isolated" >&2
    exit 1
fi
mapfile -t interfaces < <(
    ip -o link show | awk -F ': ' '{ name = $2; sub(/@.*/, "", name); print name }'
)
if [[ ${#interfaces[@]} -ne 1 || ${interfaces[0]} != lo ]]; then
    echo "closure build: isolated namespace interface inventory differs: ${interfaces[*]}" >&2
    exit 1
fi
lo_line=$(ip -o link show dev lo)
lo_flags=${lo_line#*<}
lo_flags=${lo_flags%%>*}
if [[ ",$lo_flags," == *,UP,* ]]; then
    echo "closure build: loopback interface is usable" >&2
    exit 1
fi
echo "closure build: isolated netns $parent_netns -> $current_netns; loopback down"

if [[ -e "$staging" ]]; then
    echo "closure build: staging path already exists: $staging" >&2
    exit 1
fi
"$checker" --repo-root "$repo_root" --archive-dir "$closure_input/archives" "$manifest"
mkdir -p "$staging/.build/sources" "$staging/.build/objects"
staging=$(realpath "$staging")
build_root="$staging/.build"
prefix=/usr/lib/realm
staged_prefix="$staging$prefix"

export PATH="$staged_prefix/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PKG_CONFIG_PATH="$staged_prefix/lib/pkgconfig:$staged_prefix/share/pkgconfig"
export C_INCLUDE_PATH="$staged_prefix/include"
export LIBRARY_PATH="$staged_prefix/lib"
unset LD_LIBRARY_PATH

archive_filename() {
    local selected_name=$1
    "$checker" --repo-root "$repo_root" --emit archives "$manifest" \
        | awk -F '\t' -v selected="$selected_name" '$1 == selected { print $2 }'
}

extract_source() {
    local selected_name=$1
    local filename
    local source_parent="$build_root/sources/$selected_name"
    filename=$(archive_filename "$selected_name")
    mkdir "$source_parent"
    tar -xf "$closure_input/archives/$filename" -C "$source_parent"
    mapfile -t roots < <(find "$source_parent" -mindepth 1 -maxdepth 1 -type d)
    if [[ ${#roots[@]} -ne 1 ]]; then
        echo "closure build: $selected_name archive does not have one source root" >&2
        exit 1
    fi
    printf '%s\n' "${roots[0]}"
}

meson_install() {
    local selected_name=$1
    shift
    local source_dir
    source_dir=$(extract_source "$selected_name")
    "${meson_cmd[@]}" setup "$build_root/objects/$selected_name" "$source_dir" \
        --wrap-mode=nofallback --prefix "$prefix" --libdir lib "$@"
    "${meson_cmd[@]}" compile -C "$build_root/objects/$selected_name"
    DESTDIR="$staging" "${meson_cmd[@]}" install -C "$build_root/objects/$selected_name"
    "$pc_relocator" "$prefix" "$staged_prefix"
}

meson_source=$(extract_source meson)
meson_cmd=(python3 "$meson_source/meson.py")

meson_install wayland \
    -Ddocumentation=false -Ddocbook_validation=false -Ddtd_validation=false \
    -Dtests=false -Dscanner=true
meson_install wayland-protocols -Dtests=false
meson_install libdrm \
    -Dtests=false -Dcairo-tests=disabled -Dman-pages=disabled \
    -Dvalgrind=disabled -Dinstall-test-programs=false
meson_install libinput \
    -Dmtdev=true -Dlibwacom=true \
    -Dtests=false -Dinstall-tests=false -Ddocumentation=false \
    -Ddebug-gui=false -Dlua-plugins=disabled
meson_install pixman -Dtests=disabled -Ddemos=disabled -Dgtk=disabled \
    -Dlibpng=disabled -Dopenmp=disabled
meson_install libxkbcommon \
    -Denable-tools=false -Denable-x11=false -Denable-wayland=false \
    -Denable-docs=false -Denable-xkbregistry=false \
    -Denable-bash-completion=false
meson_install libdisplay-info
meson_install wlroots \
    -Dauto_features=disabled \
    -Dbackends=drm,libinput \
    -Drenderers=gles2 \
    -Dallocators=gbm \
    -Dsession=enabled \
    -Dxwayland=enabled \
    -Dexamples=false \
    -Dlibliftoff=disabled \
    -Dcolor-management=disabled \
    -Dxcb-errors=disabled

river_source=$(extract_source river)
zig="$closure_input/tools/zig-x86_64-linux-0.16.0/zig"
if [[ ! -x "$zig" ]]; then
    echo "closure build: verified Zig executable is absent" >&2
    exit 1
fi
(
    cd "$river_source"
    "$zig" build --system "$closure_input/zig-pkg" \
        -Doptimize=ReleaseSafe -Dcpu=baseline -Dxwayland -Dman-pages=false \
        --prefix "$staged_prefix" install
)

if [[ ! -x "$staged_prefix/bin/river" ]]; then
    echo "closure build: River executable was not installed" >&2
    exit 1
fi
origin="\$ORIGIN"
patchelf --set-rpath "$origin/../lib" "$staged_prefix/bin/river"
while IFS= read -r -d '' library; do
    if readelf -h "$library" >/dev/null 2>&1; then
        patchelf --set-rpath "$origin" "$library"
    fi
done < <(find "$staged_prefix/lib" -type f -name '*.so*' -print0)

"$repo_root/packaging/river/probe-noble-package-closure.sh" "$staging" "$prefix"

# Project only runtime ownership into the binary package. The selected
# libinput quirks stay private; Noble's global udev rules and callouts remain
# owned by libinput-bin.
find "$staged_prefix/bin" -mindepth 1 -maxdepth 1 ! -name river -delete
rm -rf "${staged_prefix:?}/include" "$staged_prefix/lib/pkgconfig" \
    "$staged_prefix/share/pkgconfig" "$staged_prefix/libexec" \
    "$staged_prefix/lib/udev" "${staged_prefix:?}/etc"
find "$staged_prefix/lib" -type f \( -name '*.a' -o -name '*.la' \) -delete
find "$staged_prefix/share" -mindepth 1 -maxdepth 1 ! -name libinput -exec rm -rf {} +
rm -rf "$build_root"
