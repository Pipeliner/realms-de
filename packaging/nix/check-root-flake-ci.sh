#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
default_root=$(CDPATH='' cd "$script_dir/../.." && pwd)
root=$default_root

if [ "$#" -gt 0 ]; then
    if [ "$#" -ne 2 ] || [ "$1" != "--root" ]; then
        echo "usage: $0 [--root PATH]" >&2
        exit 2
    fi
    root=$2
fi

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

[ -f "$root/flake.nix" ] || fail 'root flake.nix is required'
[ -f "$root/flake.lock" ] || fail 'root flake.lock is required'

workflow="$root/.github/workflows/distro.yml"
[ -f "$workflow" ] || fail 'distro workflow is required'

if grep -F -q -e 'steps.flake.outputs.present' "$workflow"; then
    fail 'Nix CI must not condition on flake presence'
fi

if ! grep -F -q -e './packaging/nix/check-root-flake-ci.sh' "$workflow"; then
    fail 'normal Nix CI must invoke the root-flake guard'
fi

if ! grep -F -q -e './packaging/nix/test-root-flake-ci.sh' "$workflow"; then
    fail 'normal Nix CI must invoke the root-flake fixture suite'
fi

echo 'root-flake CI contract: pass'
