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

checks="$root/packaging/nix/checks.nix"
[ -f "$checks" ] || fail 'installed Nix VM check is required'

qt6ct_references=$(grep -F -c -e 'pkgs.qt6Packages.qt6ct' "$checks" || true)
if [ "$qt6ct_references" -ne 3 ] || grep -F -q -e 'pkgs.qt6ct' "$checks"; then
    fail 'installed VM must use the pinned Qt 6 qt6ct attribute'
fi

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

for artifact in \
    realm-gtk3-toolkit.png \
    realm-gtk4-toolkit.png \
    realm-qt6-toolkit.png \
    control-gtk3-toolkit-state.json \
    control-gtk4-toolkit-state.json \
    control-qt6-toolkit-state.json \
    gtk3-toolkit-openat.log \
    gtk3-toolkit-stderr.log \
    gtk4-toolkit-openat.log \
    gtk4-toolkit-stderr.log \
    qt6-toolkit-openat.log \
    qt6-toolkit-stderr.log \
    qt6-user-override-openat.log \
    qt6-user-override-stderr.log
do
    if ! grep -F -q -e "\${{ runner.temp }}/realm-session-boots/$artifact" "$workflow"; then
        fail "live VM evidence must retain $artifact"
    fi
done

if ! grep -F -q -e "\".#checks.\$system.portal-helper-imports\"" "$workflow"; then
    fail 'Nix CI must run the portal helper import smoke before the VM'
fi

if ! grep -F -q -e "\${{ runner.temp }}/realm-session-boots/portal-roundtrip.json" "$workflow"; then
    fail 'live VM evidence upload must retain portal-roundtrip.json'
fi

if ! grep -F -q -e 'machine.send_key("alt-c")' "$checks"; then
    fail 'portal VM must activate the explicit GTK Cancel response'
fi

if ! grep -F -q -e 'f"(if {portal_command} > {portal_output_path} "' "$checks" \
    || ! grep -F -q -e 'f"realm_portal_status=0; else realm_portal_status=$?; fi; "' "$checks"; then
    fail 'portal VM must record helper status after success or failure'
fi

if ! grep -F -q -e 'realm-session-boots/realmctl-doctor.json' "$workflow"; then
    fail 'live VM artifact must retain realmctl doctor JSON'
fi

echo 'root-flake CI contract: pass'
