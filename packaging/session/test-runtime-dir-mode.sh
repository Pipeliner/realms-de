#!/usr/bin/env bash
# SPEC 0007 A2b: the session entry's early realm directory is server-admissible.
set -euo pipefail

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
session=$root/packaging/session/realm-session
tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-runtime-dir.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

sed '/^main() {$/,$d' "$session" >"$tmp/session-functions.sh"

start_fixture_compositor() {
	local runtime=$1
	XDG_RUNTIME_DIR=$runtime bash -c '
		. "$1"
		REALM_COMPOSITOR=true
		REALM_COMPOSITOR_ARGS=
		snapshot_sockets() { :; }
		start_compositor
		wait "$compositor_pid"
	' bash "$tmp/session-functions.sh"
}

new_runtime=$tmp/new-runtime
mkdir -m 700 "$new_runtime"
(
	umask 0022
	start_fixture_compositor "$new_runtime"
)
new_mode=$(stat -c '%a' "$new_runtime/realm")
[[ $new_mode == 700 ]] || {
	printf 'session entry created realm directory with mode %s, expected 700\n' "$new_mode" >&2
	exit 1
}

existing_runtime=$tmp/existing-runtime
mkdir -m 700 "$existing_runtime"
mkdir -m 755 "$existing_runtime/realm"
existing_identity=$(stat -c '%d:%i' "$existing_runtime/realm")
(
	umask 0022
	start_fixture_compositor "$existing_runtime"
)
preserved_identity=$(stat -c '%d:%i' "$existing_runtime/realm")
preserved_mode=$(stat -c '%a' "$existing_runtime/realm")
[[ $preserved_identity == "$existing_identity" ]] || {
	printf 'session entry replaced an existing realm directory\n' >&2
	exit 1
}
[[ $preserved_mode == 755 ]] || {
	printf 'session entry changed existing realm mode from 755 to %s\n' "$preserved_mode" >&2
	exit 1
}

handoff_runtime=$tmp/handoff-runtime
mkdir -m 700 "$handoff_runtime"
mkdir -m 700 "$handoff_runtime/realm"
XDG_RUNTIME_DIR=$handoff_runtime bash -c '
	. "$1"
	have_systemd_user=0
	degraded_codes=(NO-XWAYLAND NO-GSETTINGS)
	publish_degraded_handoff
	expected="$XDG_RUNTIME_DIR/realm/degraded.$MAIN_PID"
	[[ $REALM_DEGRADED_FILE == "$expected" ]]
	[[ $(stat -c "%a" "$expected") == 600 ]]
	printf "%s\n" "$REALM_DEGRADED_FILE"
' bash "$tmp/session-functions.sh" >"$tmp/handoff-path"
handoff_path=$(<"$tmp/handoff-path")
[[ $(<"$handoff_path") == $'version=1\npid='* ]] || {
	printf 'degraded handoff lacks its versioned incarnation header\n' >&2
	exit 1
}
[[ $(grep -c '^code=' "$handoff_path") == 2 ]] || {
	printf 'degraded handoff did not preserve the finalized code set\n' >&2
	exit 1
}

stale_runtime=$tmp/stale-runtime
mkdir -m 700 "$stale_runtime"
mkdir -m 700 "$stale_runtime/realm"
XDG_RUNTIME_DIR=$stale_runtime CAPTURE=$tmp/manager-env bash -c '
	. "$1"
	have_systemd_user=1
	systemctl() { printf "%s\n" "$*" >"$CAPTURE"; }
	# Force record creation to fail while an older readable manager value could
	# otherwise survive: the incarnation path is occupied by a directory.
	mkdir "$XDG_RUNTIME_DIR/realm/degraded.$MAIN_PID"
	publish_degraded_handoff
	expected="$XDG_RUNTIME_DIR/realm/degraded.$MAIN_PID"
	[[ $REALM_DEGRADED_FILE == "$expected" ]]
	printf "%s\n" "--user set-environment REALM_DEGRADED_FILE=$expected" >"$CAPTURE.expected"
' bash "$tmp/session-functions.sh"
cmp -s "$tmp/manager-env.expected" "$tmp/manager-env" || {
	printf 'failed handoff publication retained a previous manager path\n' >&2
	exit 1
}
