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

if ! grep -F -q -e "\".#checks.\$system.session-boots-evidence\"" "$workflow"; then
    fail 'Nix CI must build the VM evidence producer before the public status gate'
fi

if ! grep -F -q -e "if: always() && steps.reference-build.outputs.vm_artifacts == 'true'" "$workflow"; then
    fail 'live VM evidence upload must run after a failed public status gate'
fi

if ! grep -F -q -e 'realm-session-boots/driver.log' "$workflow" \
    || ! grep -F -q -e 'realm-session-boots/test-status' "$workflow"; then
    fail 'live VM evidence upload must retain the driver log and exact status'
fi

if ! grep -F -q -e 'session-boots-evidence =' "$checks" \
    || [ "$(grep -F -c -e 'overrideTestDerivation' "$checks")" -lt 2 ] \
    || ! grep -F -q -e "\${session-boots-evidence}/test-status" "$checks" \
    || ! grep -F -q -e 'check-vm-evidence-status.sh' "$checks"; then
    fail 'public session-boots must gate the retained evidence driver status'
fi

if grep -F -q -e 're.sub(' "$checks" \
    && ! grep -E -q -e '^[[:space:]]+import re$' "$checks"; then
    fail 'Nix VM prompt proof must import its regular-expression dependency'
fi

if ! grep -F -q -e 'direct_activation_control = direct_activation_doctor(import_environment=True)' "$checks" \
    || ! grep -F -q -e 'direct_activation_omitted = direct_activation_doctor(import_environment=False)' "$checks" \
    || ! grep -F -q -e 'assert "SystemdActivation" not in observation["features"], observation' "$checks" \
    || ! grep -F -q -e 'assert control_bus_id != omitted_bus_id' "$checks" \
    || ! grep -F -q -e 'assert control_by_id["env/wayland-display/dbus"]["status"] == "ok"' "$checks" \
    || ! grep -F -q -e 'assert omitted_dbus["status"] == "fail"' "$checks"; then
    fail 'doctor VM must compare imported and omitted fresh direct D-Bus activation'
fi

if ! grep -F -q -e '"--property=KillMode=control-group",' "$checks" \
    || ! grep -F -q -e '"--property=RuntimeMaxSec=5s",' "$checks" \
    || ! grep -F -q -e '"--property=TimeoutStopSec=1s",' "$checks" \
    || ! grep -F -q -e 'elapsed_ms = round((time.monotonic() - started) * 1000)' "$checks"; then
    fail 'direct D-Bus doctor probes must retain their bounded control-group cleanup'
fi

if ! grep -F -q -e 'realm-session-boots/realmctl-doctor-direct-dbus-control.json' "$workflow" \
    || ! grep -F -q -e 'realm-session-boots/realmctl-doctor-direct-dbus-omitted.json' "$workflow" \
    || ! grep -F -q -e 'realm-session-boots/realmctl-doctor-direct-dbus-control.stderr' "$workflow" \
    || ! grep -F -q -e 'realm-session-boots/realmctl-doctor-direct-dbus-omitted.stderr' "$workflow" \
    || ! grep -F -q -e 'realm-session-boots/realmctl-doctor-direct-dbus-metadata.json' "$workflow"; then
    fail 'live VM artifact must retain both direct D-Bus doctor reports and diagnostics'
fi

echo 'root-flake CI contract: pass'
