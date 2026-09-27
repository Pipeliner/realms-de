# SPEC 0032 — Upstream idle and lock integration

- **Status:** Accepted technical refinement of owner-approved #79 defaults
  (2026-09-27); implementation and installed-session evidence pending
- **Depends on:** SPEC 0005 host-policy/suspend boundary, SPEC 0031 launch journey
- **Supersedes:** SPEC 0005 OQ-1's obsolete gtklock recommendation and proposed
  alternative defaults. No custom locker is required.

## Behavior and implementation boundary

### Accepted MVP scope amendment (owner, 2026-09-27)

Suspend/resume integration, diagnostics and verification are deferred until
after MVP. The before-sleep behavior described below is retained as post-MVP
intent, not a launch guarantee. Suspend failures must not gate MVP or enablement
of independently verified manual locking and idle timers. Default MVP CI must
retain manual lock, authentication, binding suppression and real idle timing
checks while excluding the suspend roundtrip; retain its reproducer separately
for explicit post-MVP execution. Do not claim laptop suspend protection.
The retained Nix driver reproducer requires the explicit environment opt-in
`REALM_POST_MVP_SUSPEND=1`; unset, empty or other values exclude only the
suspend slice. Manual lock and real idle checks before that slice, and normal
terminal/logout checks after it, remain mandatory. This opt-in is for a future
explicit CI diagnostic invocation, not a new default flake-check gate; local
packaging remains prohibited.
Source-only verification SHALL evaluate the real fixture with the current
reused-tool selection interface and parse only the lock/idle/suspend Python
slice, not unrelated raw Nix interpolation. It must still assert that manual
locking, real idle timing and terminal/logout remain outside the opt-in guard.
On an explicitly selected CI runner, build the driver with
`nix build .#checks.x86_64-linux.session-boots.driver`, then invoke
`REALM_POST_MVP_SUSPEND=1 ./result/bin/nixos-test-driver`. This sets the
environment on the driver process directly; setting it around `nix flake
check` is not a supported opt-in because the build sandbox need not inherit it.

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
idle timing and restore. Host-policy suspend evidence is post-MVP. #78 retains
the launch evidence. Source
helpers may land dormant before package enablement; do not count that as #79 done.

### Fresh-login activation (accepted implementation contract, 2026-09-27)

After the parent-reviewed manual PAM, binding suppression, readiness and real
300/600-second idle CI proof passes, packages SHALL start `realm-idle.service`
with `realm-session.target` on a fresh login. Debian, RPM and Nix ship the
relative wants link `../realm-idle.service`; the RPM owns that link and the
NixOS module declares the same wantedBy relationship. The idle unit declares
its install target; the locker remains on-demand with no wants link.
Preparation and publication of an isolated CI candidate are allowed to obtain
evidence; default integration and release require that evidence review.
Suspend is not a gate.

Installed native and NixOS VM probes SHALL require the shipped link and observe
the idle service active after graphical login without manually starting it.
They then stop idle before the long controlled acceptance journey; existing
manual locking and unchanged real 300/600-second timing checks remain required.
Stopping in the disposable fixture is not production policy. Existing session
target ownership and stop semantics remain unchanged.

### Native real idle evidence (accepted, 2026-09-27)

Ubuntu and Fedora CI run a dedicated native idle probe after the manual-lock
probe, reusing its guest transport, QEMU keyboard, process identity and PAM
helpers. The outer deadline is 900 seconds. The installed swayidle argv must
retain the production 300/600 timers. Real keyboard activity resets inactivity;
guest monotonic timestamps establish dim at 299–330 seconds and lock readiness
at 599–630 seconds, with a separate host-clock 640-second observation bound.
No injected timers, clocks or replacement locker are allowed in the VM.
The no-backlight VM must log the real dim no-op, still lock, show an opaque
uniform frame before password input, reject a wrong password, suppress the
launcher, accept the correct password and restore the launcher binding.
Activity must produce the pending
restore no-op. Stopping idle must leave its captured process identity gone.
Retain bounded journal and partial structured results on failure. These VMs
prove absence handling, not brightness restoration on physical backlights.
Final journal collection and persistence are required evidence: either failure
keeps readiness false and fails an otherwise successful probe. An existing
acceptance or cleanup failure takes precedence over this evidence failure.
Source fixtures exercise rejection of early/missing callbacks, failed auth and
cleanup without running a local VM. Readiness remains unverified until CI
evidence is reviewed; adding a probe is not passing that probe.

### Historical package staging before enablement

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

### Native manual authentication evidence

The existing Ubuntu/Fedora installed graphical-session CI VMs manually start
the unmodified dormant lock unit twice. Each cycle must retain live cgroup
process identities, show duplicate starts preserve them, suppress the launcher
binding, reject a wrong password, and unlock with the disposable guest's correct
password. Unlock must leave the unit inactive and every captured identity gone.
After each unlock, the same virtual-keyboard launcher shortcut must launch
Fuzzel, and Escape must close it; absence while locked alone is insufficient.
Monitor commands wait for a post-command prompt under a bounded deadline, and
screenshot evidence requires a complete P6 pixel payload, not merely a file.
The fixture sets a test-only guest password without enabling SSH password login,
changing distro PAM policy, or relaxing SELinux. Record installed locker/PAM
package ownership, versions and PAM configuration, bounded journals, structured
results and screenshots on success or failure when guest/monitor I/O remains
available. The idle service stays inactive and shipped wants links remain absent.
This evidence does not satisfy native idle timing, suspend/resume, or
real-hardware backlight/lid obligations.

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
sixty seconds. The suspend slice sets ten-second host socket inactivity
deadlines on both guest-shell and monitor transports, independently of the
driver's guest-side command timeout. On transport timeout it marks the guest
channel unusable, retains bounded host console/QMP evidence, and skips further
guest cleanup and journal calls rather than hanging again. Original socket
timeouts are restored at the slice boundary.
Any transport timeout, including a journal timeout after successful idle stop,
must fail the slice before subsequent guest input or commands are issued.
This is a host-observation fix, not evidence that the guest resumed or a
relaxation of the lock assertions.
The canonical Nix CI job SHALL explicitly set
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
The fixture SHALL also retain host-side QEMU process status, queued QMP events
(including shutdown/reset reasons) and the last bounded console output before
attempting guest-dependent recovery. These diagnostics SHALL remain usable when
both guest shell and human monitor sockets are closed. The pinned driver's QMP
event wait does not enforce its timeout while its event queue is empty; use only
a finite number of its nonblocking message reads, never that wait API. Keep
the kernel console enabled through suspend for this diagnostic fixture.
CI run 36325164214 proved lock readiness before deep suspend, but both sockets
were broken after wake and the log omitted the QEMU exit/event cause; it does
not establish successful resume or justify changing firmware, reboot policy or
the accepted deep-suspend path. QMP's
[shutdown/reset events](https://www.qemu.org/docs/master/interop/qemu-qmp-ref.html#event-SHUTDOWN)
distinguish those causes; the
[pinned driver implementation](https://github.com/NixOS/nixpkgs/blob/9fbb54b33e91ee4ca368e35a78e0613c720600b3/nixos/lib/test-driver/src/test_driver/machine/qmp.py)
defines the nonblocking read boundary used here.
The probe introduces no production unit or host-policy changes. Physical lid
policy and backlight restoration, and native-distro acceptance, remain separate
obligations; package auto-start remains disabled.

Bounded CI experiment after run 36327468167: QMP observed SUSPEND, WAKEUP,
then an ICH9 TCO watchdog reset about 46 seconds later while guest device resume
had not completed. This does not establish that the watchdog caused the stall.
Only the disposable Nix VM disables its virtual TCO device with QEMU 11.1.0's
`-global ICH9-LPC.enable_tco=off`; production watchdog policy is unchanged.
Enable kernel PM callback diagnostics and retain the experiment option and
observed watchdog inventory in suspend evidence. Preserve deep/logind suspend,
lock readiness, resumed-lock suppression/password unlock, inhibitor reacquisition
and every existing bound. A stalled resume still fails; neither this experiment
nor removal of a reset is successful suspend evidence. Source tests evaluate the
VM options and exercise the existing critical acceptance assertions. Evaluate
the real fixture options before derivation execution and inject their JSON into
the source test; the builder must not initialize a nested Nix store or profiles.
The exact property is defined by [pinned QEMU's ICH9 header](https://github.com/qemu/qemu/blob/v11.1.0/include/hw/acpi/ich9.h).
