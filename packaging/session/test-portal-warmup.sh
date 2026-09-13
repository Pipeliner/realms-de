#!/bin/sh
# SPEC 0005 A2: portal activation starts after import without blocking startup.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
session=$root/packaging/session/realm-session
tmp=$(mktemp -d "${TMPDIR:-/tmp}/realm-portal-warmup.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

sed '/^main() {$/,$d' "$session" >"$tmp/session-functions.sh"

PORTAL_CALLS="$tmp/calls" bash -c '
    . "$1"
    have_systemd_user=1
    systemctl() { printf "%s\n" "$*" >>"$PORTAL_CALLS"; }
    warm_portal
' bash "$tmp/session-functions.sh"

[ "$(cat "$tmp/calls")" = '--user start --no-block xdg-desktop-portal.service' ] || {
    echo 'portal warm-up was not the exact non-blocking user-service request' >&2
    exit 1
}

rm "$tmp/calls"
PORTAL_CALLS="$tmp/calls" bash -c '
    . "$1"
    have_systemd_user=0
    systemctl() { printf "%s\n" "$*" >>"$PORTAL_CALLS"; }
    warm_portal
' bash "$tmp/session-functions.sh"

[ ! -e "$tmp/calls" ] || {
    echo 'no-systemd session attempted a portal service start' >&2
    exit 1
}

# Exercise the real main function with controlled process boundaries. The
# ordered trace proves warm-up remains after both imports and before gsettings,
# degradation publication, and the session target.
sed '/^main "$@"$/d' "$session" >"$tmp/session-main.sh"
PORTAL_ORDER="$tmp/order" bash -c '
    . "$1"
    record() { printf "%s\n" "$1" >>"$PORTAL_ORDER"; }
    setup_log() { :; }
    log() { :; }
    set_identity() { record identity; }
    ensure_session_bus() { record bus; }
    detect_session_services() { record detect; }
    start_compositor() { record compositor; compositor_pid=999999; }
    wait_for_display() { record display; }
    discover_x_display() { record xdisplay; }
    import_session_environment() { record import; }
    warm_portal() { record portal; }
    apply_cursor_settings() { record cursor; }
    publish_degraded_handoff() { record handoff; }
    start_session_units() { record target; }
    cleanup() { :; }
    wait() { return 0; }
    main
' bash "$tmp/session-main.sh"

expected='identity
bus
detect
compositor
display
xdisplay
import
portal
cursor
handoff
target'
[ "$(cat "$tmp/order")" = "$expected" ] || {
    echo 'portal warm-up moved outside the accepted main startup order' >&2
    cat "$tmp/order" >&2
    exit 1
}

# A missing or refused portal service is diagnosed later by doctor; enqueue
# failure itself must not abort the desktop startup path.
PORTAL_LOG="$tmp/failure-log" bash -c '
    . "$1"
    have_systemd_user=1
    systemctl() { return 1; }
    log() { printf "%s\n" "$*" >>"$PORTAL_LOG"; }
    warm_portal
    printf survived >"$PORTAL_LOG.result"
' bash "$tmp/session-functions.sh"

[ "$(cat "$tmp/failure-log.result")" = survived ] || {
    echo 'portal enqueue failure aborted session startup' >&2
    exit 1
}
grep -F -q 'portal checks may fail' "$tmp/failure-log" || {
    echo 'portal enqueue failure was not diagnosed' >&2
    exit 1
}
