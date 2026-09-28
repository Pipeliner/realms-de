#!/usr/bin/env bash

set -euo pipefail

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
# shellcheck source=packaging/native-vm/guest-probe.sh
source "$script_dir/guest-probe.sh"

# The diagnostic wrapper changes only the compositor environment, retaining
# the normal session-command argv and the installed entry itself.
declare -F write_sync_diagnostic >/dev/null || fail 'sync diagnostic writer missing'
sync_fixture=$(mktemp -d)
trap 'rm -rf -- "$sync_fixture"' EXIT
# Expand these variables in the executed fixture, not its generator.
# shellcheck disable=SC2016
printf '%s\n' '#!/usr/bin/env bash' 'printf "%s|%s|%s\\n" "${LP_NUM_THREADS-unset}" "$#" "$1"' > "$sync_fixture/river"
# shellcheck disable=SC2016
printf '%s\n' '#!/usr/bin/env bash' 'test -z "${LP_NUM_THREADS+x}"' 'exec "$REALM_COMPOSITOR" "$@"' > "$sync_fixture/session"
chmod +x "$sync_fixture/river" "$sync_fixture/session"
write_sync_diagnostic "$sync_fixture/output" "$sync_fixture/session" "$sync_fixture/river"
test "$("$sync_fixture/output/session" 'argument with spaces')" = '0|1|argument with spaces'
test -z "${LP_NUM_THREADS+x}"
grep -Fxq "SessionCommand=$sync_fixture/output/session" "$sync_fixture/output/sddm.conf"
rm -rf -- "$sync_fixture"
trap - EXIT

# Fedora must declare the exact owner of the activation helper needed by the
# production login; a fixture-side install must not hide an omitted dependency.
grep -Eq '^Requires:[[:space:]]+dbus-tools([[:space:]]|$)' \
    "$script_dir/../fedora/realm.spec"

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

# The two target repositories package the same three real toolkit executables
# under different GTK development/example names; no alternate app may silently
# replace the installed consumer in the VM.
declare -F toolkit_fixture_packages >/dev/null || fail 'toolkit package map missing'
test "$(toolkit_fixture_packages ubuntu-24.04-x86_64 | tr '\n' ' ')" = \
    'gtk-3-examples gtk-4-examples strace '
test "$(toolkit_fixture_packages fedora-44-x86_64 | tr '\n' ' ')" = \
    'gtk3-devel gtk4-devel-tools strace '
if (toolkit_fixture_packages unknown-target) >/dev/null 2>&1; then
    fail 'unknown toolkit fixture target accepted'
fi
declare -F require_packaged_qt6ct >/dev/null || fail 'qt6ct package preflight missing'
preflight_seen=false
packages_called=false
require_packaged_qt6ct() { preflight_seen=true; return 1; }
apt-get() { packages_called=true; return 99; }
target=ubuntu-24.04-x86_64
if install_toolkit_test_clients; then
    fail 'toolkit fixture accepted missing shipped qt6ct'
fi
test "$preflight_seen" = true
test "$packages_called" = false

# Run the browser input-order regression through this existing CI entry point.
unset -f python3
python3 "$script_dir/test_browser_roundtrip.py"
python3 "$script_dir/test_relogin_roundtrip.py"
python3 "$script_dir/test_window_roundtrip.py"
