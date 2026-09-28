#!/usr/bin/env bash

set -euo pipefail

target=${2:-}
package_dir=${3:-}
probe_input_dir=$(CDPATH='' cd -- "$(dirname "${BASH_SOURCE[0]}")" && pwd)
control_probe="$probe_input_dir/control_get_state.py"
input_checker="$probe_input_dir/check_inputs.py"

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

write_sync_diagnostic() {
    local directory=$1 session_command=$2 river_command=$3
    install -d -m 0755 "$directory"
    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf 'exec env LP_NUM_THREADS=0 %q "$@"\n' "$river_command"
    } > "$directory/river"
    {
        printf '%s\n' '#!/usr/bin/env bash'
        printf 'export REALM_COMPOSITOR=%q\n' "$directory/river"
        printf 'exec %q "$@"\n' "$session_command"
    } > "$directory/session"
    chmod 0755 "$directory/river" "$directory/session"
    printf '[Wayland]\nSessionCommand=%s/session\n' "$directory" > "$directory/sddm.conf"
}

configure_sync_diagnostic() {
    test "$target" = ubuntu-24.04-x86_64 || fail 'sync diagnostic is Ubuntu-only'
    local session_command
    session_command=$(sddm --example-config | awk '
        /^\[/ { wayland = ($0 == "[Wayland]") }
        wayland && /^SessionCommand=/ { sub(/^SessionCommand=/, ""); print; exit }')
    test -x "$session_command" || fail 'SDDM default Wayland session command unavailable'
    write_sync_diagnostic /var/tmp/realm-native-sync-diagnostic "$session_command" /usr/bin/river
    install -m 0644 /var/tmp/realm-native-sync-diagnostic/sddm.conf \
        /etc/sddm.conf.d/realm-native-sync-diagnostic.conf
}

probe_sync_diagnostic() {
    test "$target" = ubuntu-24.04-x86_64 || fail 'sync diagnostic is Ubuntu-only'
    python3 -c 'import json, pathlib, pwd
uid = pwd.getpwnam("alice").pw_uid
pid = pathlib.Path(f"/run/user/{uid}/realm/session.pid").read_text().strip()
assert pid.isdigit(), pid
parent = pathlib.Path("/proc") / pid
children = parent.joinpath("task", pid, "children").read_text().split()
compositors = [pathlib.Path("/proc") / child for child in children
              if (pathlib.Path("/proc") / child / "exe").resolve().name == "river"]
assert len(compositors) == 1, compositors
p = compositors[0]
env = p.joinpath("environ").read_bytes().split(b"\0")
assert b"LP_NUM_THREADS=0" in env, "compositor sync override missing"
print(json.dumps({"diagnostic_only": True, "LP_NUM_THREADS": "0", "pid": int(p.name),
 "executable": str(p.joinpath("exe").resolve()),
 "threads": [t.joinpath("comm").read_text().strip() for t in p.joinpath("task").iterdir()]}))'
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
            test -x /usr/bin/dbus-update-activation-environment || \
                fail 'Fedora package omitted the D-Bus activation helper'
            rpm -qf --queryformat '%{NAME}\n' \
                /usr/bin/dbus-update-activation-environment \
                > /var/tmp/realm-native-activation-owner.txt
            grep -Fxq dbus-tools /var/tmp/realm-native-activation-owner.txt || \
                fail 'unexpected Fedora D-Bus activation helper owner'
            ;;
        *)
            fail "unknown native VM target: $target"
            ;;
    esac

    # Capture package-only runtime state before test libraries can pull a server.
    # Never silently repair the production dependency through fixture packages.
    {
        command -v pipewire
        pipewire --version
        test -f /usr/lib/systemd/user/pipewire.service
        test -f /usr/lib/systemd/user/pipewire.socket
        command -v slurp
        cat /usr/lib/systemd/user/pipewire.service /usr/lib/systemd/user/pipewire.socket
        case "$target" in
            ubuntu-24.04-x86_64)
                dpkg-query -W pipewire slurp
                dpkg-query -S /usr/bin/pipewire /usr/lib/systemd/user/pipewire.service
                ;;
            fedora-44-x86_64)
                rpm -q pipewire slurp
                rpm -qf /usr/bin/pipewire /usr/lib/systemd/user/pipewire.service
                ;;
        esac
    } > /var/tmp/realm-native-portal-runtime.txt
    test "$(stat -c '%U:%G:%a' /usr/share/wayland-sessions/realm.desktop)" = 'root:root:644'
    grep -Fxq 'Name=realm' /usr/share/wayland-sessions/realm.desktop
    grep -Fxq 'Exec=/usr/bin/realm-session' /usr/share/wayland-sessions/realm.desktop
    grep -Fxq 'TryExec=/usr/bin/realm-session' /usr/share/wayland-sessions/realm.desktop

    # SPEC 0032 requires packaged fresh-login idle activation.
    for helper in realm-idle realm-backlight swayidle swaylock brightnessctl; do
        test -x "/usr/bin/$helper" || fail "missing idle/lock helper: $helper"
    done
    test -f /etc/pam.d/swaylock
    test -f /usr/lib/systemd/user/realm-idle.service
    grep -Fxq 'ExecStart=/usr/bin/swaylock -f -C /dev/null' /usr/lib/systemd/user/realm-lock.service
    test "$(readlink /usr/lib/systemd/user/realm-session.target.wants/realm-idle.service)" = ../realm-idle.service
    swaylock --version

    # Disposable CI guest only. SSH remains key-only; use the distro PAM stack.
    printf '%s\n' 'alice:realmtest' | chpasswd
    {
        swaylock --version
        case "$target" in
            ubuntu-24.04-x86_64)
                dpkg-query -W swaylock swayidle brightnessctl libpam0g libpam-modules
                dpkg-query -S /usr/bin/swaylock /etc/pam.d/swaylock
                ;;
            fedora-44-x86_64)
                rpm -q swaylock swayidle brightnessctl pam
                rpm -qf /usr/bin/swaylock /etc/pam.d/swaylock
                ;;
        esac
        # Preserve the small distro PAM configuration, including include targets.
        find -L /etc/pam.d -maxdepth 1 -type f -print -exec head -c 16384 {} \;
    } > /var/tmp/realm-native-lock-packages-pam.txt

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

require_portal_socket() {
    local runtime_dir uid socket_status
    uid=$(id -u alice)
    runtime_dir="/run/user/$uid"
    # The first package-only graphical login has already happened. Capture
    # status without repairing presets or starting the socket manually.
    {
        user_command timeout 10 systemctl --user status pipewire.socket --no-pager || true
        if user_command timeout 10 systemctl --user is-active --quiet pipewire.socket; then
            socket_status=0
        else
            socket_status=$?
        fi
        printf 'package-only socket status: %s\n' "$socket_status"
    } >> /var/tmp/realm-native-portal-runtime.txt 2>&1
    return "$socket_status"
}

install_portal_test_clients() {
    require_portal_socket || return $?
    case "$target" in
        ubuntu-24.04-x86_64)
            export DEBIAN_FRONTEND=noninteractive
            apt-get install --yes python3-gi gir1.2-gst-plugins-base-1.0 \
                gstreamer1.0-plugins-base gstreamer1.0-pipewire
            ;;
        fedora-44-x86_64)
            dnf -y install python3-gobject-base gstreamer1-plugins-base pipewire-gstreamer
            ;;
        *) fail "unknown portal fixture target: $target" ;;
    esac
    python3 "$probe_input_dir/portal_vm_helper.py" --check-imports
}

install_browser_test_client() {
    # CI-only; this action is after package-only and direct portal acceptance.
    case "$target" in
        ubuntu-24.04-x86_64)
            export DEBIAN_FRONTEND=noninteractive
            apt-get install --yes ca-certificates curl gnupg
            install -d -m 0755 /etc/apt/keyrings
            curl --fail --location --connect-timeout 15 --max-time 60 \
                https://packages.mozilla.org/apt/repo-signing-key.gpg \
                -o /etc/apt/keyrings/packages.mozilla.org.asc
            local fingerprint
            fingerprint=$(gpg --batch --show-keys --with-colons \
                /etc/apt/keyrings/packages.mozilla.org.asc | awk -F: '$1 == "fpr" { print $10; exit }')
            test "$fingerprint" = 35BAA0B33E9EB396F59CA838C0BA5CE6DC6315A3
            printf '%s\n' \
                'deb [signed-by=/etc/apt/keyrings/packages.mozilla.org.asc] https://packages.mozilla.org/apt mozilla main' \
                > /etc/apt/sources.list.d/mozilla.list
            printf '%s\n' 'Package: firefox*' 'Pin: origin packages.mozilla.org' \
                'Pin-Priority: 1000' '' 'Package: firefox' 'Pin: release o=Ubuntu' \
                'Pin-Priority: -1' > /etc/apt/preferences.d/realm-browser-fixture
            apt-get update
            apt-get install --yes firefox tesseract-ocr tesseract-ocr-eng
            {
                printf 'Mozilla signing-key fingerprint: %s\n' "$fingerprint"
                cat /etc/apt/sources.list.d/mozilla.list /etc/apt/preferences.d/realm-browser-fixture
                apt-cache policy firefox
                dpkg-query -W firefox tesseract-ocr tesseract-ocr-eng
                dpkg-query -S /usr/bin/firefox
            } > /var/tmp/realm-native-browser-packages.txt
            if dpkg-query -W -f='${Version}' firefox | grep -q snap; then
                fail 'browser fixture installed the Ubuntu Snap transition instead of Mozilla DEB'
            fi
            ;;
        fedora-44-x86_64)
            dnf -y install firefox tesseract tesseract-langpack-eng
            {
                rpm -q firefox tesseract tesseract-langpack-eng
                rpm -qf /usr/bin/firefox
                dnf info --installed firefox
            } > /var/tmp/realm-native-browser-packages.txt
            ;;
        *) fail "unknown browser fixture target: $target" ;;
    esac
    firefox --version >> /var/tmp/realm-native-browser-packages.txt
    tesseract --list-langs 2>&1 | tee -a /var/tmp/realm-native-browser-packages.txt | grep -Fxq eng
    install -d -m 0755 /usr/share/applications /etc/xdg
    printf '%s\n' '[Desktop Entry]' 'Type=Application' 'Name=Realm Browser Test' \
        'Exec=/usr/bin/firefox --no-remote http://127.0.0.1:8765/' \
        'MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;' \
        > /usr/share/applications/realm-browser-test.desktop
    printf '%s\n' '[Default Applications]' \
        'text/html=realm-browser-test.desktop' \
        'x-scheme-handler/http=realm-browser-test.desktop' \
        'x-scheme-handler/https=realm-browser-test.desktop' \
        > /etc/xdg/mimeapps.list
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

    for unit in realm-session.target realm-wm.service realm-bar.service realm-idle.service; do
        wait_user_unit "$unit" 30
        user_command systemctl --user is-active "$unit" >> "$evidence/units.txt"
    done

    # Observe automatic login activation before controlling the long fixture.
    # Timing acceptance is separate; this does not certify the 300/600 timers.
    user_command systemctl --user show realm-idle.service \
        -p ActiveState -p MainPID -p ActiveEnterTimestampMonotonic \
        > "$evidence/idle-login.txt"
    user_command systemctl --user stop realm-idle.service
    test "$(user_command systemctl --user show realm-idle.service -p ActiveState --value)" = inactive

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

    user_command timeout 10 python3 "$control_probe" \
        "$runtime_dir/realm/ctl.sock" "$evidence/control-get-state.ndjson"
    python3 "$input_checker" control \
        "$evidence/control-get-state.ndjson"

    user_command timeout 15 systemd-run --user --wait --pipe --quiet --collect \
        --unit=realm-native-doctor /usr/bin/realmctl --json doctor \
        > "$evidence/realmctl-doctor.json"
    python3 "$input_checker" doctor \
        "$evidence/realmctl-doctor.json"

    cp /var/tmp/realm-native-packages.txt "$evidence/packages.txt"
    cp /var/tmp/realm-native-portal-runtime.txt "$evidence/portal-package-runtime.txt"
    cp /var/tmp/realm-native-lock-packages-pam.txt "$evidence/lock-packages-pam.txt"
    cp /var/tmp/realm-native-session-owner.txt "$evidence/session-entry-owner.txt"
    if [[ "$target" == fedora-44-x86_64 ]]; then
        cp /var/tmp/realm-native-activation-owner.txt \
            "$evidence/dbus-activation-owner.txt"
    fi
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
        configure-sync-diagnostic)
            configure_sync_diagnostic
            ;;
        probe-sync-diagnostic)
            probe_sync_diagnostic
            ;;
        install)
            install_guest
            ;;
        probe)
            probe_guest
            ;;
        portal-clients)
            install_portal_test_clients
            ;;
        browser-client)
            install_browser_test_client
            ;;
        *)
            fail 'usage: guest-probe.sh {install TARGET PACKAGE_DIR|probe TARGET EVIDENCE_DIR|portal-clients TARGET|browser-client TARGET}'
            ;;
    esac
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
