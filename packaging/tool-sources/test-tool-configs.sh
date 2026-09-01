#!/bin/sh
# B4 schema guard: Helm's Yazi template must target the selected v25.4.8 schema.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
template=$root/configs/templates/yazi-theme.toml

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
