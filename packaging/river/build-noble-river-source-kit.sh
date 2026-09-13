#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 MANIFEST ACQUIRED-CACHE OUTPUT" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
manifest=$(realpath "$1")
cache=$(realpath "$2")
output=$3
manifest_checker="$repo_root/packaging/river/check-closure-manifest.py"
kit_checker="$repo_root/packaging/river/check-noble-river-source-kit.py"

if [[ -e "$output" || -L "$output" ]]; then
    echo "source-kit: output path already exists: $output" >&2
    exit 1
fi
mapfile -t cache_entries < <(find "$cache" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
if [[ "${cache_entries[*]}" != "archives tools zig-fetch-root zig-global" ]]; then
    echo "source-kit: acquired cache inventory differs: ${cache_entries[*]}" >&2
    exit 1
fi
mapfile -t fetch_entries < <(find "$cache/zig-fetch-root" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
if [[ "${fetch_entries[*]}" != "build.zig zig-pkg" ]]; then
    echo "source-kit: Zig fetch-root inventory differs: ${fetch_entries[*]}" >&2
    exit 1
fi
mapfile -t tool_entries < <(find "$cache/tools" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
if [[ "${tool_entries[*]}" != "zig-x86_64-linux-0.16.0" ]]; then
    echo "source-kit: selected Zig tool inventory differs: ${tool_entries[*]}" >&2
    exit 1
fi
"$manifest_checker" --repo-root "$repo_root" \
    --archive-dir "$cache/archives" "$manifest"
mapfile -t expected_hashes < <(
    "$manifest_checker" --repo-root "$repo_root" --emit zig-dependencies "$manifest" \
        | cut -f3 | sort
)
mapfile -t actual_hashes < <(
    find "$cache/zig-fetch-root/zig-pkg" -mindepth 1 -maxdepth 1 -type d \
        -printf '%f\n' | sort
)
if [[ "${actual_hashes[*]}" != "${expected_hashes[*]}" ]]; then
    echo "source-kit: Zig package inventory differs" >&2
    exit 1
fi
while IFS= read -r entry; do
    if [[ -L "$entry" || (! -f "$entry" && ! -d "$entry") ]]; then
        echo "source-kit: acquired cache contains unsafe entry: $entry" >&2
        exit 1
    fi
done < <(find "$cache" -mindepth 1)

output_parent=$(dirname "$output")
mkdir -p "$output_parent"
output_parent=$(realpath "$output_parent")
temporary=$(mktemp -d "$output_parent/.realm-river-source.XXXXXX")
published=0
cleanup() {
    ((published)) || rm -rf "$temporary"
}
trap cleanup EXIT
kit="$temporary/realm-river-0.4.8"
mkdir -p "$kit/packaging/river" "$kit/closure-input"
cp -R "$repo_root/packaging/debian-river" "$kit/debian"
cp "$repo_root/flake.lock" "$kit/flake.lock"
for helper in \
    build-noble-package-closure.sh \
    check-closure-manifest.py \
    check-private-resolutions.py \
    probe-noble-package-closure.sh \
    run-noble-river-package-build.sh; do
    cp "$repo_root/packaging/river/$helper" "$kit/packaging/river/$helper"
done
cp "$manifest" "$kit/packaging/river/sources.toml"
cp -R "$cache/archives" "$kit/closure-input/archives"
cp -R "$cache/tools" "$kit/closure-input/tools"
cp -R "$cache/zig-fetch-root/zig-pkg" "$kit/closure-input/zig-pkg"
"$kit_checker" "$kit"
mv -Tn "$kit" "$output"
if [[ -e "$kit" ]]; then
    echo "source-kit: output path appeared during publication: $output" >&2
    exit 1
fi
published=1
rmdir "$temporary"
echo "source-kit: verified and published $output"
