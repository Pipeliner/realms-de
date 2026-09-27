# SPEC 0032 — Upstream idle and lock integration

- **Status:** Accepted technical refinement of owner-approved #79 defaults
  (2026-09-27); implementation and installed-session evidence pending
- **Depends on:** SPEC 0005 host-policy/suspend boundary, SPEC 0031 launch journey
- **Supersedes:** SPEC 0005 OQ-1's obsolete gtklock recommendation and proposed
  alternative defaults. No custom locker is required.

## Behavior and implementation boundary

Use distro-provided PAM-backed swaylock supporting ext-session-lock-v1
(minimum 1.7) and systemd-enabled swayidle. NixOS explicitly enables swaylock's
PAM service. Verify actual packaged versions/protocols in CI before enabling.
No independent lid listener, suspend policy or authentication implementation.

`realm-idle` execs swayidle with `-w -C /dev/null`, so unrelated user swayidle
configuration does not silently add duplicate timers. At 300 seconds it invokes
`realm-backlight dim`; activity restores the saved backlight. At 600 seconds,
logind lock, and before-sleep, it synchronously starts `realm-lock.service` via
systemctl --user. Never use --no-block on this readiness boundary. The host
decides whether to suspend; the hook waits for locking within logind's bounded
inhibit delay. A failed lock is reported, never treated as proof of readiness.
The delay is not an unlimited guarantee against a host forcing sleep.

The locker service is Type=forking and runs swaylock -f: the parent exits only
after the compositor accepts locking. Systemd coalesces repeated start requests
while the service is starting or active; it tracks the locker lifetime through
its cgroup (`ExitType=cgroup`, `GuessMainPID=no`). PAM-backed swaylock forks an
authentication child as well as the graphical daemon, so no uniquely guessed
MainPID is required. Verify the actual packaged process group, not mocks.
It has no automatic restart that could relock after successful authentication.
The normal swaylock opaque background provides blanking; display power-off is
not claimed by this slice. Use swaylock's default appearance initially, not a
new theme renderer. It reads no unrelated swaylock user config.

Both units stop with Realm and graphical-session targets; neither can pull the
session down on its own failure. The idle unit follows realm-wm.service and
requires WAYLAND_DISPLAY. Locker and idle are not enabled until River binding
suppression (#247), PAM, protocol and readiness checks pass.

Backlight integration uses brightnessctl's supported backlight class, save and
restore commands. Dimming subtracts 90% of the maximum, with the tool's minimum
brightness floor: it must not increase an already dimmed backlight. It does not
touch keyboard LEDs. Missing/unsupported backlight devices are a logged no-op,
not failure of locking; VM absence must not disable the independent lock timer.
The current value is saved even when already at minimum, so a later restore
does not select an older idle cycle's brightness.
On service stop swayidle runs pending resume commands, restoring saved brightness.
Real device restoration and absence handling require CI/hardware evidence.

## Verification and delivery

Pure command-boundary tests exercise the launcher with stand-in executables:
exact timer values, synchronous lock triggers, no host-lid action, backlight
class and save/restore arguments, invalid helper invocation, and propagation of
swayidle failure. These prove glue only, not locking or authentication.

CI-only package integration must install helpers/units, dependencies and wants;
Nix wrappers resolve their own executable paths. Actual VM/hardware evidence
must prove lock readiness, password unlock, duplicate starts, binding suppression,
idle timing, restore and host-policy suspend. #78 retains the evidence. Source
helpers may land dormant before package enablement; do not count that as #79 done.

### Package staging before enablement

All targets install realm-idle, realm-backlight and the two service units.
Native packages depend on swayidle, swaylock >= 1.7 and brightnessctl; Nix binds
their store paths and rejects an older swaylock. NixOS configures the swaylock
PAM service whenever Realm is enabled. Native targets use the distro swaylock
PAM file, verified after installation. No Realm PAM implementation is introduced.

During dormant staging realm-idle.service has no Install/WantedBy section or
shipped wants link, preventing Debian helper auto-enablement. Existing WM/bar
enablement stays unchanged. Nix and native VM checks verify the installed
helpers, resolved locker executable, units and PAM file, and absence of an idle
wants link. These package checks do not claim password or suspend correctness.

### Installed NixOS lock round-trip

The graphical VM uses an explicit test-only account password and manually starts
the shipped lock service, without enabling idle timers. Two lock/unlock cycles
must prove synchronous start, live swaylock processes in the service cgroup,
unchanged PID/start-time identities after
a duplicate start, PAM password unlock through the virtual keyboard, and a clean
inactive service afterward with all captured process identities gone. Retain
service properties and process identities before assertions. While locked, the launcher shortcut must not launch
the launcher; after unlock the same shortcut must work. Retain lock screenshots
and structured results. This does not replace idle timing, wrong-password,
suspend, native-distro or hardware acceptance.

This corrects the failed MainPID assertion in CI run 36317090486: the compositor
had accepted locking and systemd reported the service active, but its PID
guessing returned zero for the multiprocess locker. Systemd documents cgroup
lifetime tracking for services without a reliable main process and warns that
PID guessing is unreliable for multiprocess daemons:
[systemd.service](https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html).

### Installed dormant idle timing (accepted refinement, 2026-09-27)

After the two existing lock round-trips, the NixOS VM manually starts the
unmodified installed idle service. No wants link or shortened test timers are
introduced. With no backlight device, a real idle cycle must log the helper's
unavailable-device diagnostic at 300 seconds and still reach compositor-backed
lock readiness at 600 seconds. Record the virtual input reset's boot-monotonic
time, journal monotonic timestamps, and the lock service's active-enter
monotonic timestamp. Allow one second of input/measurement uncertainty and up
to thirty seconds of CI scheduling delay; fail an earlier or missing event.

While idle-locked, a deliberately wrong password must leave the same live locker
process identities and service active after a five-second observation interval;
the launcher binding remains suppressed. Correct authentication then unlocks.
Activity must produce a second unavailable-backlight diagnostic through the
resume hook. Stopping idle must leave the service inactive and no live swayidle
process. On failure, emit structured results and the last 200 journal entries
into the build log before attempting artifact writes, where the driver can
still collect them. Successful derivations retain journal JSON, structured
results and lock screenshots for artifact upload. Failed-derivation screenshot
retention is not claimed.

This proves the installed default timers and the no-backlight path only. Real
brightness restoration, host-policy suspend/readiness/resume and native-distro
acceptance remain outstanding. Automatic startup stays disabled, and this
slice does not complete #79.

### Real logind suspend/resume (accepted refinement, 2026-09-27)

The CI VM manually starts the unchanged installed idle service with its locker
inactive, and requires logind to list a sleep/delay inhibitor owned by that
service's PID and UID. Record the configured inhibitor deadline. Require actual
guest suspend support and select its supported deep sleep mode for the fixture;
an unsupported VM fails this probe rather than counting as suspend evidence.

Request `org.freedesktop.login1.Manager.Suspend(false)` from a detached system
service. Do not inject a signal or directly start systemd-suspend.service.
Within sixty seconds QEMU must report the guest suspended; wake it through the
monitor's `system_wakeup` command and require the guest to respond again within
sixty seconds. These inner polling budgets
assume responsive test-driver monitor and guest I/O; they do not interrupt a
blocked driver socket read. The canonical Nix CI job SHALL explicitly set
`timeout-minutes: 60` as the outer execution cap, covering build and VM work.
A root-flake policy regression SHALL reject its absence or another value even
when other jobs have sixty-minute limits. A new locker readiness timestamp
must fall after the request
and before systemd's sleep-start journal event; require a successful sleep-stop
event and kernel suspend-entry/exit records. The resumed session remains locked,
suppresses its launcher binding and unlocks only after the fixture password.
Require swayidle to reacquire its own sleep/delay inhibitor after resume.

Retain structured inhibitor, timing and monitor evidence, bounded journals and a
resumed-lock screenshot on success. On failure, wake the guest if necessary and
emit available structured results and bounded diagnostics into the build log.
The probe introduces no production unit or host-policy changes. Physical lid
policy and backlight restoration, and native-distro acceptance, remain separate
obligations; package auto-start remains disabled.
