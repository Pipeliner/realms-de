# SPEC 0033 — First functional prototype using packaged Sway

- Status: Accepted — owner directed radical reuse and implementation, 2026-10-02;
  selection of existing tools is a delegated engineering decision.
- Target: prototype, not certification of the previous Rust/River MVP.

## Outcome and scope

Produce the shortest runnable keyboard-first desktop using distribution Sway,
Foot, Fuzzel, Thunar, swaybar/i3status, swaylock and swayidle. Use ordinary stock
application themes. Sway owns windows, workspaces, input, output and its bar.
Realm supplies only a small configuration and relocatable launcher.

This prototype supersedes conflicting custom-WM/bar/palette/tool-vendoring
prerequisites for the first usable desktop. Preserve existing Rust/River work
as shelved experiments; do not delete it or mistake its CI for prototype proof.
Uniform palette management, ledger/undo, custom orbit semantics, custom shell
tools, cross-toolkit theme guarantees and framework work are outside this path.
Suspend/resume remains deferred. Manual lock and idle dim at 300 seconds /
lock-blank at 600 seconds remain configured prototype behavior, with explicit
hardware/PAM verification limits until tested.

## Behavior

1. Run the launcher directly from checkout or extracted CI artifact without
   compiling any Realm crate or building a private compositor/tool closure.
2. Launch Sway with the adjacent explicit config; preserve caller arguments and
   set session desktop identity to Sway for upstream desktop integration.
   Missing required commands produce a useful error before starting Sway.
   The first prototype starts from a TTY: reject inherited nonempty `DISPLAY`
   or `WAYLAND_DISPLAY` with a useful TTY instruction before launching or
   changing D-Bus activation state. Nested desktops are outside this scope;
   their session variables must not overwrite the host desktop's activation
   environment. Headless CI clears both inherited display variables.
3. Provide terminal, launcher, files, browser, close, focus/move, six workspaces,
   fullscreen, floating, reload, lock and exit bindings, documented in README.
   Existing application settings are not rewritten. A neutral static config
   is permitted; the palette literal ban does not apply to this prototype.
4. Swaybar and i3status show ordinary upstream status. Use upstream idle and
   lock commands. Brightness dim/restore uses packaged brightnessctl when
   available; absence of a backlight is documented rather than fabricated.
5. Reuse distribution Sway/portal integration where installed, with explicit
   requirements/instructions. Do not promise tested screen sharing, suspend,
   hardware support or PAM correctness from headless tests.
6. CI is the only package installation/archive/build environment. A separate
   bounded workflow installs upstream Ubuntu packages, validates the config,
   boots a real headless Sway session, opens Foot through the configured binding,
   exercises focus/workspaces and captures a real screenshot. CI retains the
   relocatable launcher/config artifact and runtime evidence. KVM is unnecessary
   for this proof; actual laptop testing remains a separate observation.
   Workspace-return verification counts both Foot windows within workspace 1,
   rather than across the whole compositor tree. Wait for terminal input before
   typing screenshot labels so the image shows readable successful commands.
   The terminal `exit` IPC command may close Sway's socket before its reply;
   accept only that documented missing-response condition, and require the
   actual compositor process to exit with status zero within a bounded wait.
   Other IPC errors and nonzero compositor exit status remain test failures.
   Exercise configured Mod4+D to launch Fuzzel and Escape to dismiss it, and
   Mod4+E to map a real Thunar window followed by Mod4+Shift+Q to close it.
   Identify exact application processes and match Thunar's mapped window PID;
   retain process/tree evidence and application screenshots before restoring
   the final two-terminal desktop. Browser/network and PAM tests remain outside
   this bounded headless proof.

## Verification and delivery

Before implementation, behavioral launcher tests fail on missing files; then
prove config selection, missing-dependency errors and argument preservation.
Prove inherited X11/Wayland displays are rejected before compositor startup.
Remote Sway validation and rendered-session tests are required before claiming
the prototype works. A screenshot must be inspected, with exact run/revision
and limitations in README. Fast configuration checks are separate from the
historic Rust/River verification and cannot be blocked by unrelated old jobs.

## Implementation plan

1. Add `prototype/realm-prototype` and its adjacent Sway config; test invocation.
2. Add bounded `prototype.yml` CI for stock packages, real session and artifacts.
3. Update README and repository map with the prototype command and shelved scope.
4. Commit, push, inspect CI, fix failures, inspect screenshot, record evidence.

Sources: [Sway](https://swaywm.org/),
[upstream configuration](https://github.com/swaywm/sway/blob/master/config.in),
[swayidle manual](https://github.com/swaywm/swayidle/blob/master/swayidle.1.scd).
