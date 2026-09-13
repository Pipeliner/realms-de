#!/usr/bin/env bash

set -euo pipefail

target=${2:-}
package_dir=${3:-}

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

install_guest() {
    case "$target" in
        ubuntu-24.04-x86_64)
            export DEBIAN_FRONTEND=noninteractive
            apt-get update
            mapfile -t packages < <(
                find "$package_dir" -maxdepth 1 -type f -name '*.deb' -print | sort
            )
            ((${#packages[@]} == 2)) || fail 'expected two Ubuntu package inputs'
            apt-get install --yes "${packages[@]}" sddm
            dpkg-query -W -f='${Package} ${Version} ${Architecture}\n' \
                realm realm-river > /var/tmp/realm-native-packages.txt
            grep -Fxq 'realm 0.1.0 amd64' /var/tmp/realm-native-packages.txt
            grep -Fxq 'realm-river 0.4.8-1 amd64' \
                /var/tmp/realm-native-packages.txt
            dpkg-query -S /usr/share/wayland-sessions/realm.desktop \
                > /var/tmp/realm-native-session-owner.txt
            ;;
        fedora-44-x86_64)
            mapfile -t packages < <(
                find "$package_dir" -maxdepth 1 -type f -name '*.rpm' -print | sort
            )
            ((${#packages[@]} == 1)) || fail 'expected one Fedora package input'
            dnf -y install "${packages[@]}" sddm
            rpm -q --queryformat '%{NAME} %{VERSION}-%{RELEASE} %{ARCH}\n' realm \
                > /var/tmp/realm-native-packages.txt
            grep -Eq '^realm 0\.1\.0-1\.fc44 x86_64$' \
                /var/tmp/realm-native-packages.txt
            rpm -qf /usr/share/wayland-sessions/realm.desktop \
                > /var/tmp/realm-native-session-owner.txt
            ;;
        *)
            fail "unknown native VM target: $target"
            ;;
    esac

    test "$(stat -c '%U:%G:%a' /usr/share/wayland-sessions/realm.desktop)" = 'root:root:644'
    grep -Fxq 'Name=realm' /usr/share/wayland-sessions/realm.desktop
    grep -Fxq 'Exec=/usr/bin/realm-session' /usr/share/wayland-sessions/realm.desktop
    grep -Fxq 'TryExec=/usr/bin/realm-session' /usr/share/wayland-sessions/realm.desktop

    install -d -m 0755 /etc/sddm.conf.d
    printf '%s\n' \
        '[Autologin]' \
        'User=alice' \
        'Session=realm' \
        'Relogin=true' \
        > /etc/sddm.conf.d/realm-native-vm.conf
    systemctl enable sddm.service
    systemctl set-default graphical.target
}

session_property() {
    loginctl show-session "$1" -p "$2" --value
}

find_realm_session() {
    local deadline=$((SECONDS + 90)) session name type remote
    while ((SECONDS < deadline)); do
        while read -r session _; do
            [[ -n "$session" ]] || continue
            name=$(session_property "$session" Name)
            type=$(session_property "$session" Type)
            remote=$(session_property "$session" Remote)
            if [[ "$name" == alice && "$type" == wayland && "$remote" == no ]]; then
                printf '%s\n' "$session"
                return 0
            fi
        done < <(loginctl list-sessions --no-legend)
        sleep 1
    done
    return 1
}

one_user_pid() {
    local process=$1
    mapfile -t pids < <(pgrep -u alice -x "$process" || true)
    ((${#pids[@]} == 1)) || fail "expected one alice $process process, found ${#pids[@]}"
    printf '%s\n' "${pids[0]}"
}

user_command() {
    runuser -u alice -- env \
        XDG_RUNTIME_DIR="$runtime_dir" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime_dir/bus" \
        "$@"
}

wait_user_unit() {
    local unit=$1 timeout_seconds=$2 deadline
    deadline=$((SECONDS + timeout_seconds))
    while ((SECONDS < deadline)); do
        if user_command systemctl --user is-active --quiet "$unit"; then
            return 0
        fi
        sleep 1
    done
    fail "user unit did not become active: $unit"
}

probe_guest() {
    local evidence=$package_dir
    local session uid runtime_dir river_pid wm_pid bar_pid river_exe wm_exe
    install -d -m 0755 "$evidence"
    chown alice:alice "$evidence"

    session=$(find_realm_session) || fail 'alice has no non-remote Wayland login'
    uid=$(id -u alice)
    runtime_dir="/run/user/$uid"
    test -S "$runtime_dir/bus" || fail 'alice user bus is absent'

    loginctl show-session "$session" --all > "$evidence/logind-session.txt"
    test "$(session_property "$session" Type)" = wayland
    test "$(session_property "$session" Remote)" = no

    for unit in realm-session.target realm-wm.service realm-bar.service; do
        wait_user_unit "$unit" 30
        user_command systemctl --user is-active "$unit" >> "$evidence/units.txt"
    done

    river_pid=$(one_user_pid river)
    wm_pid=$(one_user_pid realm-wm)
    bar_pid=$(one_user_pid realm-bar)
    river_exe=$(readlink -f "/proc/$river_pid/exe")
    wm_exe=$(readlink -f "/proc/$wm_pid/exe")
    case "$target" in
        ubuntu-24.04-x86_64)
            test "$river_exe" = /usr/lib/realm/bin/river
            ;;
        fedora-44-x86_64)
            test "$river_exe" = /usr/bin/river
            ;;
    esac
    test "$wm_exe" = /usr/bin/realm-wm
    test "$(readlink -f "/proc/$bar_pid/exe")" = /usr/bin/realm-bar
    printf '%s %s\n%s %s\n%s %s\n' \
        river "$river_exe" realm-wm "$wm_exe" realm-bar \
        "$(readlink -f "/proc/$bar_pid/exe")" > "$evidence/processes.txt"

    user_command timeout 10 python3 /tmp/realm-native-vm/control_get_state.py \
        "$runtime_dir/realm/ctl.sock" "$evidence/control-get-state.ndjson"
    python3 /tmp/realm-native-vm/check_inputs.py control \
        "$evidence/control-get-state.ndjson"

    user_command timeout 15 systemd-run --user --wait --pipe --quiet --collect \
        --unit=realm-native-doctor /usr/bin/realmctl --json doctor \
        > "$evidence/realmctl-doctor.json"
    python3 /tmp/realm-native-vm/check_inputs.py doctor \
        "$evidence/realmctl-doctor.json"

    cp /var/tmp/realm-native-packages.txt "$evidence/packages.txt"
    cp /var/tmp/realm-native-session-owner.txt "$evidence/session-entry-owner.txt"
    cp /usr/share/wayland-sessions/realm.desktop "$evidence/realm.desktop"
    cp /etc/sddm.conf.d/realm-native-vm.conf "$evidence/sddm-autologin.conf"
    systemctl status display-manager.service --no-pager \
        > "$evidence/display-manager.txt" 2>&1 || true
    user_command systemctl --user status realm-session.target realm-wm.service \
        realm-bar.service --no-pager > "$evidence/user-units.txt" 2>&1 || true
    journalctl -b -n 2000 --no-pager > "$evidence/system-journal.txt"
    journalctl _UID="$uid" -b -n 2000 --no-pager > "$evidence/alice-journal.txt"
    if command -v getenforce >/dev/null 2>&1; then
        getenforce > "$evidence/selinux-mode.txt"
    fi
}

main() {
    case ${1:-} in
        install)
            install_guest
            ;;
        probe)
            probe_guest
            ;;
        *)
            fail 'usage: guest-probe.sh {install TARGET PACKAGE_DIR|probe TARGET EVIDENCE_DIR}'
            ;;
    esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
