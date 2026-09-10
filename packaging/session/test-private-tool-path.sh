#!/bin/sh
# B3: private Realm tools are available to Realm, never leaked to global activation.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
session=$root/packaging/session/realm-session
unit_path='PATH=/usr/lib/realm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin'
tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-private-path.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

require() {
    file=$1
    text=$2
    grep -F -q "$text" "$file" || {
        echo "missing private-tool PATH contract in $file: $text" >&2
        exit 1
    }
}

forbid() {
    file=$1
    text=$2
    if grep -F -q "$text" "$file"; then
        echo "private-tool PATH leaked in $file: $text" >&2
        exit 1
    fi
}

require "$session" 'readonly REALM_PRIVATE_BIN='/usr/lib/realm/bin''
require "$session" 'strip_private_path()'
# shellcheck disable=SC2016 # This is a literal source-contract probe.
require "$session" 'REALM_CALLER_PATH="$(strip_private_path "${PATH:-}")"'
# shellcheck disable=SC2016 # This is a literal source-contract probe.
forbid "$session" 'REALM_CALLER_PATH="${PATH:-}"'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'export PATH="$REALM_PRIVATE_BIN:$REALM_CALLER_PATH"'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'PATH="$REALM_CALLER_PATH" systemctl --user import-environment'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'PATH="$REALM_CALLER_PATH" dbus-update-activation-environment'
forbid "$session" 'SESSION_ENV_VARS+=(PATH)'

require "$root/packaging/systemd/realm-wm.service" "Environment=$unit_path"
require "$root/packaging/systemd/realm-bar.service" "Environment=$unit_path"
forbid "$root/packaging/nix/home-manager-module.nix" 'PATH=/usr/lib/realm/bin'

sed '/^main() {$/,$d' "$session" >"$tmp/session-functions.sh"
captured=$(PATH='/caller/bin:/usr/lib/realm/bin:/other/bin' \
    /bin/bash -c '. "$1"; printf "%s" "$REALM_CALLER_PATH"' bash "$tmp/session-functions.sh")
[ "$captured" = '/caller/bin:/other/bin' ] || {
    echo "contaminated caller PATH was not filtered: $captured" >&2
    exit 1
}

IMPORT_OUTPUT_DIR="$tmp" PATH='/bin:/usr/lib/realm/bin:/caller/bin' /bin/bash -c '
    . "$1"
    REALM_IMPORT_PATH=1
    have_systemd_user=1
    have_dbus_update=1
    systemctl() { printf "%s\n" "$PATH" >"$IMPORT_OUTPUT_DIR/systemd-path"; }
    dbus-update-activation-environment() { printf "%s\n" "$PATH" >"$IMPORT_OUTPUT_DIR/dbus-path"; }
    import_session_environment
' bash "$tmp/session-functions.sh" "$tmp"

for imported in "$tmp/systemd-path" "$tmp/dbus-path"; do
    [ "$(cat "$imported")" = '/bin:/caller/bin' ] || {
        echo "activation import leaked private PATH through $imported" >&2
        exit 1
    }
done
