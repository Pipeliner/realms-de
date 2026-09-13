#!/usr/bin/env bash

set -euo pipefail

require_kvm() {
    local device=${1:-/dev/kvm}
    if [[ ! -c "$device" || ! -r "$device" || ! -w "$device" ]]; then
        printf 'KVM is required but unavailable: %s\n' "$device" >&2
        return 1
    fi
}

wait_for_ssh() {
    local qemu_pid=$1
    local timeout_seconds=$2
    shift 2

    local deadline=$((SECONDS + timeout_seconds))
    while (( SECONDS < deadline )); do
        if ! kill -0 "$qemu_pid" 2>/dev/null; then
            printf 'QEMU exited before SSH became ready\n' >&2
            return 1
        fi
        if "$@"; then
            return 0
        fi
        sleep 0.25
    done
    printf 'SSH did not become ready within %s seconds\n' "$timeout_seconds" >&2
    return 1
}

wait_for_ssh_down() {
    local qemu_pid=$1 timeout_seconds=$2
    shift 2

    local deadline=$((SECONDS + timeout_seconds))
    while ((SECONDS < deadline)); do
        if ! kill -0 "$qemu_pid" 2>/dev/null; then
            printf 'QEMU exited while waiting for guest reboot\n' >&2
            return 1
        fi
        if ! "$@"; then
            return 0
        fi
        sleep 0.25
    done
    printf 'SSH stayed reachable instead of beginning the guest reboot\n' >&2
    return 1
}

stop_qemu() {
    local pid=${1:-}
    [[ -n "$pid" ]] || return 0
    kill -0 "$pid" 2>/dev/null || {
        wait "$pid" 2>/dev/null || true
        return 0
    }
    kill -TERM "$pid" 2>/dev/null || true
    local deadline=$((SECONDS + 10))
    while kill -0 "$pid" 2>/dev/null && ((SECONDS < deadline)); do
        sleep 0.25
    done
    if kill -0 "$pid" 2>/dev/null; then
        kill -KILL "$pid" 2>/dev/null || true
    fi
    wait "$pid" 2>/dev/null || true
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || {
        printf 'required host command is unavailable: %s\n' "$1" >&2
        return 1
    }
}

run_native_session_vm() (
    set -euo pipefail
    if (($# != 3)); then
        printf 'usage: run-native-session-vm.sh TARGET PACKAGE_DIR EVIDENCE_DIR\n' >&2
        return 2
    fi
    local target=$1 package_dir=$2 evidence_dir=$3
    local script_dir check_inputs guest_probe control_probe image_url image
    local run_root overlay seed user_data meta_data public_key monitor serial_log
    local qemu_pid='' ssh_port=2222 serial_log='' probe_status=0
    local install_timeout=${REALM_NATIVE_VM_INSTALL_TIMEOUT_SECONDS:-900}
    local cleanup_timeout=${REALM_NATIVE_VM_CLEANUP_TIMEOUT_SECONDS:-10}
    local -a packages ssh_options scp_options qemu_options

    script_dir=$(CDPATH='' cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    check_inputs="$script_dir/check_inputs.py"
    guest_probe="$script_dir/guest-probe.sh"
    control_probe="$script_dir/control_get_state.py"
    for command in cloud-localds curl qemu-img qemu-system-x86_64 \
        scp socat ssh ssh-keygen python3 timeout; do
        require_command "$command"
    done
    require_kvm /dev/kvm

    mkdir -p "$evidence_dir"
    mapfile -t packages < <(python3 "$check_inputs" packages "$target" "$package_dir")
    case "$target" in
        ubuntu-24.04-x86_64)
            ((${#packages[@]} == 2)) || return 1
            ;;
        fedora-44-x86_64)
            ((${#packages[@]} == 1)) || return 1
            ;;
        *)
            printf 'unknown native VM target: %s\n' "$target" >&2
            return 1
            ;;
    esac

    run_root=$(mktemp -d "${RUNNER_TEMP:-/tmp}/realm-native-vm.XXXXXX")
    cleanup_native_vm() {
        if [[ -n "$qemu_pid" ]] && kill -0 "$qemu_pid" 2>/dev/null; then
            capture_framebuffer "$evidence_dir/framebuffer.ppm" || true
            timeout "$cleanup_timeout" ssh "${ssh_options[@]}" alice@127.0.0.1 \
                'sudo journalctl -b --no-pager' \
                > "$evidence_dir/cleanup-journal.txt" 2>&1 || true
        fi
        [[ -f "$serial_log" ]] && cp "$serial_log" "$evidence_dir/qemu-serial.log"
        stop_qemu "$qemu_pid"
        rm -rf -- "$run_root"
    }
    capture_framebuffer() {
        local output=$1 deadline=$((SECONDS + 2))
        [[ -n "$qemu_pid" ]] || return 1
        while [[ ! -e "$monitor" ]] && kill -0 "$qemu_pid" 2>/dev/null \
            && ((SECONDS < deadline)); do
            sleep 0.05
        done
        [[ -e "$monitor" ]] || return 1
        printf 'screendump %s\n' "$output" \
            | timeout 10 socat - "UNIX-CONNECT:$monitor"
    }
    trap cleanup_native_vm EXIT

    image_url=$(python3 "$check_inputs" url "$target")
    image="$run_root/base.qcow2"
    curl --fail --location --retry 3 --connect-timeout 15 --max-time 900 \
        --output "$image" "$image_url"
    python3 "$check_inputs" image "$target" "$image"
    cp "$script_dir/images.json" "$evidence_dir/image-authority.json"

    overlay="$run_root/overlay.qcow2"
    qemu-img create -f qcow2 -F qcow2 -b "$image" "$overlay"
    qemu-img resize "$overlay" 20G

    ssh-keygen -q -t ed25519 -N '' -f "$run_root/ssh-key"
    public_key=$(<"$run_root/ssh-key.pub")
    user_data="$run_root/user-data"
    meta_data="$run_root/meta-data"
    case "$target" in
        ubuntu-24.04-x86_64) local admin_group=sudo ;;
        fedora-44-x86_64) local admin_group=wheel ;;
    esac
    printf '%s\n' \
        '#cloud-config' \
        'users:' \
        '  - default' \
        '  - name: alice' \
        "    groups: [$admin_group]" \
        '    sudo: ALL=(ALL) NOPASSWD:ALL' \
        '    shell: /bin/bash' \
        '    lock_passwd: true' \
        '    ssh_authorized_keys:' \
        "      - $public_key" \
        'ssh_pwauth: false' \
        'growpart:' \
        '  mode: auto' \
        'resize_rootfs: true' \
        > "$user_data"
    printf 'instance-id: realm-%s\nlocal-hostname: realm-native-vm\n' \
        "$target" > "$meta_data"
    seed="$run_root/seed.img"
    cloud-localds "$seed" "$user_data" "$meta_data"

    monitor="$run_root/qemu-monitor.sock"
    serial_log="$run_root/qemu-serial.log"
    ssh_options=(
        -F /dev/null
        -i "$run_root/ssh-key"
        -o BatchMode=yes
        -o ConnectTimeout=2
        -o StrictHostKeyChecking=no
        -o UserKnownHostsFile=/dev/null
        -p "$ssh_port"
    )
    scp_options=(
        -F /dev/null
        -i "$run_root/ssh-key"
        -o BatchMode=yes
        -o ConnectTimeout=2
        -o StrictHostKeyChecking=no
        -o UserKnownHostsFile=/dev/null
        -P "$ssh_port"
    )
    qemu_options=(
        -enable-kvm
        -machine accel=kvm
        -cpu host
        -m 4096
        -smp 2
        -drive "file=$overlay,if=virtio,format=qcow2"
        -drive "file=$seed,if=virtio,format=raw,readonly=on"
        -device virtio-vga
        -device qemu-xhci
        -device usb-tablet
        -device usb-kbd
        -netdev "user,id=net0,hostfwd=tcp:127.0.0.1:$ssh_port-:22"
        -device "virtio-net-pci,netdev=net0"
        -display none
        -vnc 127.0.0.1:1
        -monitor "unix:$monitor,server=on,wait=off"
        -serial "file:$serial_log"
    )
    qemu-system-x86_64 "${qemu_options[@]}" &
    qemu_pid=$!

    wait_for_ssh "$qemu_pid" 180 timeout 5 \
        ssh "${ssh_options[@]}" alice@127.0.0.1 true
    timeout 180 ssh "${ssh_options[@]}" alice@127.0.0.1 \
        'cloud-init status --wait && mkdir -p /tmp/realm-native-packages /tmp/realm-native-vm'
    timeout 120 scp "${scp_options[@]}" "${packages[@]}" \
        alice@127.0.0.1:/tmp/realm-native-packages/
    timeout 60 scp "${scp_options[@]}" \
        "$guest_probe" "$control_probe" "$check_inputs" \
        alice@127.0.0.1:/tmp/realm-native-vm/
    timeout "$install_timeout" ssh "${ssh_options[@]}" alice@127.0.0.1 sudo bash \
        /tmp/realm-native-vm/guest-probe.sh install "$target" /tmp/realm-native-packages

    timeout 30 ssh "${ssh_options[@]}" alice@127.0.0.1 \
        'sudo systemctl reboot' || true
    wait_for_ssh_down "$qemu_pid" 30 timeout 5 \
        ssh "${ssh_options[@]}" alice@127.0.0.1 true
    wait_for_ssh "$qemu_pid" 180 timeout 5 \
        ssh "${ssh_options[@]}" alice@127.0.0.1 true
    timeout 180 ssh "${ssh_options[@]}" alice@127.0.0.1 sudo bash \
        /tmp/realm-native-vm/guest-probe.sh probe "$target" \
        /tmp/realm-native-evidence || probe_status=$?
    timeout 60 scp "${scp_options[@]}" -r \
        alice@127.0.0.1:/tmp/realm-native-evidence/. "$evidence_dir/" || true
    if ((probe_status != 0)); then
        printf 'installed graphical-session probe failed with status %s\n' \
            "$probe_status" >&2
        return "$probe_status"
    fi

    capture_framebuffer "$evidence_dir/framebuffer.ppm"
    test -s "$evidence_dir/framebuffer.ppm"
    printf '%s\n' "$target" > "$evidence_dir/target.txt"

    stop_qemu "$qemu_pid"
    qemu_pid=''
    trap - EXIT
    cleanup_native_vm
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    run_native_session_vm "$@"
fi
