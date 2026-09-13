#!/bin/sh
# Transparent Cargo recorder for the native package-path fixture.
set -eu

printf 'cargo|cwd=%s|home=%s|args=%s\n' "$PWD" "${CARGO_HOME:-}" "$*" \
    >>"${REALM_SENTINEL_LOG:?}"
if ! cmp -s "${CARGO_HOME:?}/config.toml" "${REALM_EXPECTED_CARGO_CONFIG:?}" \
    || find "$CARGO_HOME" -mindepth 1 ! -path "$CARGO_HOME/config.toml" \
        -print -quit | grep . >/dev/null; then
    printf 'cargo-home-not-retained-config|home=%s\n' "$CARGO_HOME" \
        >>"$REALM_SENTINEL_LOG"
    exit 96
fi

set +e
"${REALM_REAL_CARGO:?}" "$@"
status=$?
set -e
printf 'cargo-result|status=%s\n' "$status" >>"$REALM_SENTINEL_LOG"

if [ "$status" -eq 0 ] && [ "${1:-}" = build ]; then
    output_status=0
    for bin in realmctl realm-wm realm-bar; do
        if [ -x "${CARGO_TARGET_DIR:?}/release/$bin" ]; then
            printf 'cargo-output|binary=%s|executable=yes\n' "$bin" \
                >>"$REALM_SENTINEL_LOG"
        else
            printf 'cargo-output|binary=%s|executable=no\n' "$bin" \
                >>"$REALM_SENTINEL_LOG"
            output_status=95
        fi
    done
    [ "$output_status" -eq 0 ] || exit "$output_status"
fi

exit "$status"
