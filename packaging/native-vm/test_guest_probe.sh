#!/usr/bin/env bash

set -euo pipefail

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
# shellcheck source=packaging/native-vm/guest-probe.sh
source "$script_dir/guest-probe.sh"

attempts=0
user_command() {
    attempts=$((attempts + 1))
    ((attempts >= 3))
}

wait_user_unit realm-wm.service 3
test "$attempts" = 3

printf 'native VM guest retry fixture passed\n'
