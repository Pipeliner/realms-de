#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 STAGING-ROOT LOGICAL-PREFIX" >&2
    exit 2
fi

staging=$(realpath "$1")
logical_prefix=$2
prefix="$staging$logical_prefix"
river="$prefix/bin/river"
origin="\$ORIGIN"
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$prefix/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$staging"

if [[ -n ${LD_LIBRARY_PATH:-} ]]; then
    echo "closure probe: LD_LIBRARY_PATH must be unset" >&2
    exit 1
fi
unset LD_LIBRARY_PATH

for feature in \
    have_drm_backend \
    have_libinput_backend \
    have_gles2_renderer \
    have_gbm_allocator \
    have_session \
    have_xwayland
do
    value=$(pkg-config --variable="$feature" wlroots-0.20)
    if [[ "$value" != true ]]; then
        echo "closure probe: required wlroots feature $feature=$value" >&2
        exit 1
    fi
done

if [[ $(pkg-config --modversion libinput) != 1.31.3 ]]; then
    echo "closure probe: private libinput version differs" >&2
    exit 1
fi
libinput=$(find "$prefix/lib" -type f -name 'libinput.so.*' -print -quit)
if [[ -z "$libinput" ]] || ! strings "$libinput" \
    | grep -Fx -- "$logical_prefix/share/libinput" >/dev/null; then
    echo "closure probe: libinput logical quirks path differs" >&2
    exit 1
fi
for feature in \
    have_x11_backend \
    have_vulkan_renderer \
    have_udmabuf_allocator \
    have_color_management
do
    value=$(pkg-config --variable="$feature" wlroots-0.20)
    if [[ "$value" != false ]]; then
        echo "closure probe: omitted wlroots feature $feature=$value" >&2
        exit 1
    fi
done

if [[ $(patchelf --print-rpath "$river") != "$origin/../lib" ]]; then
    echo "closure probe: River has an unexpected runpath" >&2
    exit 1
fi

elf_objects=("$river")
while IFS= read -r -d '' candidate; do
    if readelf -h "$candidate" >/dev/null 2>&1; then
        elf_objects+=("$candidate")
    fi
done < <(find "$prefix/lib" -type f -name '*.so*' -print0)
if [[ ${#elf_objects[@]} -lt 2 ]]; then
    echo "closure probe: no private shared objects were installed" >&2
    exit 1
fi

resolution_log=$(mktemp)
trap 'rm -f "$resolution_log"' EXIT
for object in "${elf_objects[@]}"; do
    readelf -d "$object" >/dev/null
    if [[ "$object" != "$river" ]] && \
        [[ $(patchelf --print-rpath "$object") != "$origin" ]]; then
        echo "closure probe: private object has an unexpected runpath: $object" >&2
        exit 1
    fi
    env -u LD_LIBRARY_PATH ldd "$object" >>"$resolution_log"
done
if grep -F 'not found' "$resolution_log"; then
    echo "closure probe: unresolved transitive ELF dependency" >&2
    exit 1
fi
"$repo_root/packaging/river/check-private-resolutions.py" \
    "$prefix" "$resolution_log"
if ! find "$prefix/lib" -type f -name 'libwayland-client.so.*' -print -quit \
    | grep -q .; then
    echo "closure probe: private libwayland-client shared object is absent" >&2
    exit 1
fi

version=$(env -u LD_LIBRARY_PATH "$river" -version)
if [[ "$version" != '0.4.8 +xwayland' ]]; then
    echo "closure probe: unexpected River version output: $version" >&2
    exit 1
fi

echo "closure probe: wlroots features verified"
echo "closure probe: ${#elf_objects[@]} ELF objects resolved transitively with private runpaths"
echo "closure probe: river -version: $version"
