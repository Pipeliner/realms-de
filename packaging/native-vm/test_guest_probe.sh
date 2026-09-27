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

# A failed package-only socket check must prevent all test-package installation.
preflight_seen=false
packages_called=false
require_portal_socket() { preflight_seen=true; return 1; }
apt-get() { packages_called=true; return 99; }
dnf() { packages_called=true; return 99; }
target=ubuntu-24.04-x86_64
if install_portal_test_clients; then
    printf 'inactive baseline socket was accepted\n' >&2
    exit 1
fi
test "$preflight_seen" = true
test "$packages_called" = false

require_portal_socket() { preflight_seen=true; }
apt-get() { test "$preflight_seen" = true; test "$1" = install; }
python3() { test "$preflight_seen" = true; test "$2" = --check-imports; }
preflight_seen=false
install_portal_test_clients
printf 'native VM portal provisioning order fixture passed\n'
