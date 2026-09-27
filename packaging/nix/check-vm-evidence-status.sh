#!/bin/sh

set -eu

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

if [ "$#" -ne 1 ]; then
    fail "usage: $0 STATUS_FILE"
fi

status_file=$1
[ -f "$status_file" ] || fail "VM evidence status is missing: $status_file"

exec 3<"$status_file"
IFS= read -r schema <&3 || fail 'VM evidence status has no complete schema line'
IFS= read -r exit_line <&3 || fail 'VM evidence status has no complete exit-code line'
extra=
if IFS= read -r extra <&3 || [ -n "$extra" ]; then
    fail 'VM evidence status has unexpected trailing data'
fi

[ "$schema" = 'realm-session-boots-status/v1' ] \
    || fail "VM evidence status has an unknown schema: $schema"

case "$exit_line" in
    exit_code=*) exit_code=${exit_line#exit_code=} ;;
    *) fail "VM evidence status has a malformed exit-code line: $exit_line" ;;
esac
case "$exit_code" in
    ''|*[!0-9]*) fail "VM evidence status has a malformed exit code: $exit_code" ;;
esac
[ "$exit_code" = 0 ] || fail "NixOS VM driver exited with status $exit_code"

