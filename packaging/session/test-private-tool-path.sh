#!/bin/sh
# B3: private Helm tools are available to Helm, never leaked to global activation.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
session=$root/packaging/session/helm-session
unit_path='PATH=/usr/lib/helm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin'
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-private-path.XXXXXX")
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

require "$session" 'readonly HELM_PRIVATE_BIN='/usr/lib/helm/bin''
require "$session" 'strip_private_path()'
# shellcheck disable=SC2016 # This is a literal source-contract probe.
require "$session" 'HELM_CALLER_PATH="$(strip_private_path "${PATH:-}")"'
# shellcheck disable=SC2016 # This is a literal source-contract probe.
forbid "$session" 'HELM_CALLER_PATH="${PATH:-}"'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'export PATH="$HELM_PRIVATE_BIN:$HELM_CALLER_PATH"'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'PATH="$HELM_CALLER_PATH" systemctl --user import-environment'
# shellcheck disable=SC2016 # These are literal source-contract probes.
require "$session" 'PATH="$HELM_CALLER_PATH" dbus-update-activation-environment'
forbid "$session" 'SESSION_ENV_VARS+=(PATH)'

require "$root/packaging/systemd/helm-wm.service" "Environment=$unit_path"
require "$root/packaging/systemd/helm-bar.service" "Environment=$unit_path"
forbid "$root/packaging/nix/home-manager-module.nix" 'PATH=/usr/lib/helm/bin'

sed '/^main() {$/,$d' "$session" >"$tmp/session-functions.sh"
captured=$(PATH='/caller/bin:/usr/lib/helm/bin:/other/bin' \
    /bin/bash -c '. "$1"; printf "%s" "$HELM_CALLER_PATH"' bash "$tmp/session-functions.sh")
[ "$captured" = '/caller/bin:/other/bin' ] || {
    echo "contaminated caller PATH was not filtered: $captured" >&2
    exit 1
}
