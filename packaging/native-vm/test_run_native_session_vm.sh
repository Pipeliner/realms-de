#!/usr/bin/env bash

set -euo pipefail

fixture_script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
# shellcheck source=packaging/native-vm/run-native-session-vm.sh
source "$fixture_script_dir/run-native-session-vm.sh"

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

case_root=$(mktemp -d)
cleanup() {
    jobs -pr | xargs -r kill 2>/dev/null || true
    rm -rf -- "$case_root"
}
trap cleanup EXIT

if require_kvm "$case_root/missing-kvm" 2>"$case_root/missing.err"; then
    fail 'missing KVM was accepted'
fi
grep -Fq 'KVM is required' "$case_root/missing.err" || fail 'missing KVM diagnostic was lost'

if grep -Fq 'local-hostname:' "$fixture_script_dir/run-native-session-vm.sh"; then
    fail 'NoCloud seed still requests a cosmetic hostname'
fi

(
    exit 0
) &
dead_pid=$!
wait "$dead_pid"
start=$SECONDS
if wait_for_ssh "$dead_pid" 5 false 2>"$case_root/dead.err"; then
    fail 'dead QEMU was accepted'
fi
(( SECONDS - start < 2 )) || fail 'dead QEMU was not detected promptly'
grep -Fq 'QEMU exited before SSH became ready' "$case_root/dead.err" \
    || fail 'early QEMU death diagnostic was lost'

attempts="$case_root/attempts"
printf '0\n' >"$attempts"
fake_ssh="$case_root/fake-ssh"
cat >"$fake_ssh" <<'EOF'
#!/usr/bin/env bash
count=$(cat "$REALM_TEST_ATTEMPTS")
count=$((count + 1))
printf '%s\n' "$count" >"$REALM_TEST_ATTEMPTS"
test "$count" -ge 3
EOF
chmod +x "$fake_ssh"
sleep 10 &
qemu_pid=$!
REALM_TEST_ATTEMPTS="$attempts" wait_for_ssh "$qemu_pid" 3 "$fake_ssh"
test "$(cat "$attempts")" = 3 || fail 'SSH readiness did not retry to success'
kill "$qemu_pid"
wait "$qemu_pid" 2>/dev/null || true

printf '0\n' >"$attempts"
fake_disconnect="$case_root/fake-disconnect"
cat >"$fake_disconnect" <<'EOF'
#!/usr/bin/env bash
count=$(cat "$REALM_TEST_ATTEMPTS")
count=$((count + 1))
printf '%s\n' "$count" >"$REALM_TEST_ATTEMPTS"
test "$count" -lt 3
EOF
chmod +x "$fake_disconnect"
sleep 10 &
qemu_pid=$!
REALM_TEST_ATTEMPTS="$attempts" wait_for_ssh_down "$qemu_pid" 3 "$fake_disconnect"
test "$(cat "$attempts")" = 3 || fail 'SSH reboot transition did not retry'
stop_qemu "$qemu_pid"
if kill -0 "$qemu_pid" 2>/dev/null; then
    fail 'bounded QEMU cleanup left the process running'
fi

fake_bin="$case_root/fake-bin"
mkdir "$fake_bin"
fake_command="$fake_bin/fake-command"
cat >"$fake_command" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case ${0##*/} in
    curl)
        while (($#)); do
            if [[ $1 == --output ]]; then
                printf 'fixture-image\n' >"$2"
                exit 0
            fi
            shift
        done
        exit 2
        ;;
    ssh-keygen)
        while (($#)); do
            if [[ $1 == -f ]]; then
                : >"$2"
                printf 'ssh-ed25519 fixture realm-native-vm\n' >"$2.pub"
                exit 0
            fi
            shift
        done
        exit 2
        ;;
    qemu-system-x86_64)
        for argument in "$@"; do
            if [[ $argument == unix:*,server=on,wait=off ]]; then
                monitor=${argument#unix:}
                monitor=${monitor%,server=on,wait=off}
                : >"$monitor"
            fi
        done
        sleep 30
        ;;
    socat)
        read -r command output
        [[ $command == screendump ]]
        printf 'P6\n1 1\n255\n000' >"$output"
        ;;
    ssh)
        count=$(cat "$REALM_TEST_SSH_COUNT")
        count=$((count + 1))
        printf '%s\n' "$count" >"$REALM_TEST_SSH_COUNT"
        if [[ $count -ge 3 ]]; then
            sleep 30
        fi
        ;;
    *)
        exit 0
        ;;
esac
EOF
chmod +x "$fake_command"
for command in cloud-localds curl qemu-img qemu-system-x86_64 scp socat ssh ssh-keygen; do
    ln -s fake-command "$fake_bin/$command"
done

fixture_packages="$case_root/packages"
fixture_evidence="$case_root/failure-evidence"
mkdir "$fixture_packages"
touch "$fixture_packages/realm.deb" "$fixture_packages/realm-river.deb"
python3() {
    case $2 in
        packages)
            printf '%s\n' \
                "$fixture_packages/realm.deb" \
                "$fixture_packages/realm-river.deb"
            ;;
        url)
            printf 'https://images.example.invalid/base.qcow2\n'
            ;;
        image)
            return 0
            ;;
        *)
            command python3 "$@"
            ;;
    esac
}
require_kvm() { return 0; }
ssh_count="$case_root/ssh-count"
printf '0\n' >"$ssh_count"
set +e
failure_start=$SECONDS
(
    set -e
    PATH="$fake_bin:$PATH" RUNNER_TEMP="$case_root" \
        REALM_TEST_SSH_COUNT="$ssh_count" \
        REALM_NATIVE_VM_INSTALL_TIMEOUT_SECONDS=1 \
        REALM_NATIVE_VM_CLEANUP_TIMEOUT_SECONDS=1 \
        run_native_session_vm ubuntu-24.04-x86_64 \
        "$fixture_packages" "$fixture_evidence"
)
fixture_status=$?
set -e
test "$fixture_status" = 124 || fail "failure-path fixture returned $fixture_status"
((SECONDS - failure_start < 4)) \
    || fail 'remote command or cleanup exceeded its outer deadline'
test -s "$fixture_evidence/framebuffer.ppm" \
    || fail 'failure cleanup did not retain a framebuffer'
if find "$case_root" -maxdepth 1 -type d -name 'realm-native-vm.*' | grep -q .; then
    fail 'failure cleanup left its temporary VM directory behind'
fi

printf 'native VM bounded-wait fixtures passed\n'
