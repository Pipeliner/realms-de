#!/bin/sh
# B4/G15 schema guard: Realm's complete terminal profile must target the
# selected tools and must keep every selector generation-local.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
template=$root/configs/templates/yazi-theme.toml
manager=$root/configs/templates/yazi.toml
keymap=$root/configs/templates/yazi-keymap.toml
profile=$root/configs/templates/zshrc
btop=$root/configs/templates/btop.conf
support=$root/packaging/nix/support.nix

require() {
    grep -F -q "$1" "$template" || {
        echo "missing Yazi v25.4 field: $1" >&2
        exit 1
    }
}

forbid() {
    if grep -F -q "$1" "$template"; then
        echo "legacy Yazi field remains: $1" >&2
        exit 1
    fi
}

require '[manager]'
require '[mode]'
require 'normal_main ='
require 'normal_alt ='
require 'select_main ='
require 'select_alt ='
require 'unset_main ='
require 'unset_alt ='
require 'perm_sep ='
require 'perm_type ='
require 'perm_read ='
require 'perm_write ='
require 'perm_exec ='

forbid '[mgr]'
forbid 'mode_normal ='
forbid 'mode_select ='
forbid 'mode_unset ='
forbid 'permissions_t ='
forbid 'permissions_r ='
forbid 'permissions_w ='
forbid 'permissions_x ='
forbid 'permissions_s ='

grep -F -q 'ratio = [1, 4, 3]' "$manager"
grep -F -q 'linemode = "size"' "$manager"
grep -F -q 'on = "<C-p>"' "$keymap"
# The dollar and command-substitution forms are the literal selectors under test.
# shellcheck disable=SC2016
grep -F -q 'btop --config=\"$REALM_GENERATION/btop/btop.conf\"' "$keymap"
# shellcheck disable=SC2016
grep -F -q 'eval "$(starship init zsh)"' "$profile"
# shellcheck disable=SC2016
grep -F -q 'command btop --config="$REALM_GENERATION/btop/btop.conf"' "$profile"
grep -F -q 'color_theme = "realm"' "$btop"
grep -F -q 'shown_boxes = "cpu mem net proc"' "$btop"
grep -F -q 'dontUpdateAutotoolsGnuConfigScripts = true;' "$support" || {
    echo "retained Nix Yazi permits automatic vendor mutation" >&2
    exit 1
}
