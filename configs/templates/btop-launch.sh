#!/bin/sh
set -eu
: "${XDG_RUNTIME_DIR:?Realm btop requires XDG_RUNTIME_DIR}"
: "${REALM_GENERATION:?Realm btop requires a selected generation}"
test -r "$REALM_GENERATION/btop/btop.conf"
test -d "$REALM_GENERATION/btop/themes"
realm_btop_view=$(mktemp -d "$XDG_RUNTIME_DIR/realm-btop.XXXXXX")
trap 'rm -rf -- "$realm_btop_view"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir "$realm_btop_view/btop"
ln -s "$REALM_GENERATION/btop/btop.conf" "$realm_btop_view/btop/btop.conf"
ln -s "$REALM_GENERATION/btop/themes" "$realm_btop_view/btop/themes"
XDG_CONFIG_HOME="$realm_btop_view" btop "$@"
