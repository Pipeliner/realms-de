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
