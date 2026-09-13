#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 SOURCE-KIT" >&2
    exit 2
fi

source_kit=$(realpath "$1")
if [[ ! -f "$source_kit/debian/control" ]]; then
    echo "package build: source kit omits debian/control: $source_kit" >&2
    exit 1
fi
parent_netns=$(readlink /proc/self/ns/net)

if ! sudo --non-interactive true; then
    echo "package build: non-interactive sudo is required for a fresh network namespace" >&2
    exit 1
fi

cd "$source_kit"
exec sudo --non-interactive unshare --net -- \
    env REALM_PARENT_NETNS="$parent_netns" dpkg-buildpackage -us -uc -b
