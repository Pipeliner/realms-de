#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
    echo "usage: $0 MANIFEST NEW-CACHE-DIRECTORY" >&2
    exit 2
fi

manifest=$(realpath "$1")
cache=$2
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-closure-manifest.py"

if [[ -e "$cache" ]]; then
    echo "closure acquisition: cache path already exists: $cache" >&2
    exit 1
fi

"$checker" --repo-root "$repo_root" "$manifest"
mkdir -p "$cache/archives" "$cache/tools" "$cache/zig-global" \
    "$cache/zig-fetch-root"

while IFS=$'\t' read -r name filename url digest; do
    destination="$cache/archives/$filename"
    echo "acquire: $name $url"
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output "$destination.part" "$url"
    printf '%s  %s\n' "$digest" "$destination.part" | sha256sum --check --status -
    mv "$destination.part" "$destination"
done < <("$checker" --repo-root "$repo_root" --emit archives "$manifest")

"$checker" --repo-root "$repo_root" --archive-dir "$cache/archives" "$manifest"
tar -xf "$cache/archives/zig-x86_64-linux-0.16.0.tar.xz" -C "$cache/tools"
zig="$cache/tools/zig-x86_64-linux-0.16.0/zig"
if [[ ! -x "$zig" ]]; then
    echo "closure acquisition: selected Zig executable is absent" >&2
    exit 1
fi
printf '%s\n' \
    'const std = @import("std");' \
    'pub fn build(_: *std.Build) void {}' \
    >"$cache/zig-fetch-root/build.zig"

while IFS=$'\t' read -r name url content_hash; do
    echo "zig fetch: $name $url"
    fetched_hash=$(
        cd "$cache/zig-fetch-root"
        "$zig" fetch --global-cache-dir "$cache/zig-global" "$url"
    )
    if [[ "$fetched_hash" != "$content_hash" ]]; then
        echo "closure acquisition: Zig hash mismatch for $name" >&2
        exit 1
    fi
    if [[ ! -d "$cache/zig-fetch-root/zig-pkg/$content_hash" ]]; then
        echo "closure acquisition: Zig package cache is absent for $name" >&2
        exit 1
    fi
done < <("$checker" --repo-root "$repo_root" --emit zig-dependencies "$manifest")

echo "closure acquisition: verified"
