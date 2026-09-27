#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "usage: $0 MANIFEST CACHE OUTPUT-PREFIX" >&2
    exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
parent_netns=$(readlink /proc/self/ns/net)

if ! sudo --non-interactive true; then
    echo "closure build: non-interactive sudo is required for a fresh network namespace" >&2
    exit 1
fi

exec sudo --non-interactive unshare --net -- \
    "$repo_root/packaging/river/build-noble-closure.sh" \
    "$1" "$2" "$3" "$parent_netns"
