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
while the service is starting or active; it tracks the locker lifetime. Verify
main-PID tracking with the actual packaged swaylock/PAM process tree, not mocks.
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
