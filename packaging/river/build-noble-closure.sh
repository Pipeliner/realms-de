#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
    echo "usage: $0 MANIFEST CACHE OUTPUT-PREFIX PARENT-NETNS" >&2
    exit 2
fi

manifest=$(realpath "$1")
cache=$(realpath "$2")
output=$3
parent_netns=$4
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-closure-manifest.py"

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

if [[ -e "$output" ]]; then
    echo "closure build: output path already exists: $output" >&2
    exit 1
fi
"$checker" --repo-root "$repo_root" --archive-dir "$cache/archives" "$manifest"
mkdir -p "$output" "$cache/sources" "$cache/build"
prefix=$(realpath "$output")

export PATH="$prefix/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig"
export C_INCLUDE_PATH="$prefix/include"
export LIBRARY_PATH="$prefix/lib"
unset LD_LIBRARY_PATH

archive_filename() {
    local selected_name=$1
    "$checker" --repo-root "$repo_root" --emit archives "$manifest" \
        | awk -F '\t' -v selected="$selected_name" '$1 == selected { print $2 }'
}

extract_source() {
    local selected_name=$1
    local filename
    local source_parent="$cache/sources/$selected_name"
    filename=$(archive_filename "$selected_name")
    mkdir "$source_parent"
    tar -xf "$cache/archives/$filename" -C "$source_parent"
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
    "${meson_cmd[@]}" setup "$cache/build/$selected_name" "$source_dir" \
        --wrap-mode=nofallback --prefix "$prefix" --libdir lib "$@"
    "${meson_cmd[@]}" compile -C "$cache/build/$selected_name"
    "${meson_cmd[@]}" install -C "$cache/build/$selected_name"
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
zig="$cache/tools/zig-x86_64-linux-0.16.0/zig"
if [[ ! -x "$zig" ]]; then
    echo "closure build: verified Zig executable is absent" >&2
    exit 1
fi
(
    cd "$river_source"
    "$zig" build --system "$cache/zig-fetch-root/zig-pkg" \
        -Doptimize=ReleaseSafe -Dcpu=baseline -Dxwayland -Dman-pages=false \
        --prefix "$prefix" install
)

if [[ ! -x "$prefix/bin/river" ]]; then
    echo "closure build: River executable was not installed" >&2
    exit 1
fi
origin="\$ORIGIN"
patchelf --set-rpath "$origin/../lib" "$prefix/bin/river"
while IFS= read -r -d '' library; do
    if readelf -h "$library" >/dev/null 2>&1; then
        patchelf --set-rpath "$origin" "$library"
    fi
done < <(find "$prefix/lib" -type f -name '*.so*' -print0)

"$repo_root/packaging/river/probe-noble-closure.sh" "$prefix"
