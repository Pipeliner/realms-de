---
name: wayland-session-integration
description: Use when working on how a helm session starts or talks to the rest of the desktop stack - the session entry script or wrapper, .desktop session files, systemd user units and graphical-session.target, XDG_CURRENT_DESKTOP / XDG_SESSION_TYPE / WAYLAND_DISPLAY, systemctl --user import-environment, dbus-update-activation-environment, xdg-desktop-portal and portal backends, D-Bus activation, cursor theme or size, XWayland, `helm ctl doctor`, or packaging in packaging/ and configs/portal/. Also use when diagnosing symptoms like a GTK file dialog that hangs for 25 seconds, screen sharing that silently fails in Firefox or Chromium, an invisible or default-black cursor, or a DBusActivatable app that will not launch.
---

# Wayland session integration

This is the single most common way a minimal Wayland desktop breaks, and it
breaks *late*: the compositor comes up, the bar draws, everything looks fine,
and then the first file dialog hangs for 25 seconds. The cause is almost always
that the session's environment never reached the two places that launch things
on the user's behalf — systemd's user manager and the D-Bus session bus.

**Status:** helm's session entry, `configs/portal/` and the `doctor` checks are
**M3** work and do not exist yet. This skill is the contract they must satisfy,
recorded so it is not rediscovered by debugging. `docs/PITFALLS.md` § "Session
integration — the classic killers" is the failure register; ADR 0011 in
`docs/adr/` is the decision record.

## The ordering contract

Order is the whole trick. Environment must exist before it is imported, and it
must be imported before anything that could be D-Bus- or systemd-activated
starts. Deviating from this order is what produces every symptom below.

```
1. export XDG_CURRENT_DESKTOP=helm
   export XDG_SESSION_TYPE=wayland
   export XDG_SESSION_DESKTOP=helm
   (plus XCURSOR_THEME and XCURSOR_SIZE)
        │  set BEFORE the compositor starts: portal backend selection and
        │  cursor loading both read them at client start-up
        ▼
2. start the compositor
        │
        ▼
3. WAIT for WAYLAND_DISPLAY to actually exist
        │  the socket appears asynchronously; importing too early imports
        │  nothing, silently, and everything downstream inherits the hole
        ▼
4. import into BOTH:
     systemctl --user import-environment WAYLAND_DISPLAY XDG_CURRENT_DESKTOP \
         XDG_SESSION_TYPE XDG_SESSION_DESKTOP XCURSOR_THEME XCURSOR_SIZE
     dbus-update-activation-environment --systemd WAYLAND_DISPLAY \
         XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP \
         XCURSOR_THEME XCURSOR_SIZE
        │  BOTH. systemd-activated units and D-Bus-activated services are two
        │  different launchers with two different environments.
        ▼
5. systemctl --user start helm-session.target
   (bar, portals, idle/lock, everything else)
```

Two rules that follow from it:

- **Never import a fixed list "just in case" before step 3.** An import of an
  unset variable is not an error; it is a silent no-op that looks identical to
  success in the logs.
- **Never start a client between steps 2 and 4.** A client started in that
  window inherits the right environment itself but leaves the *activation*
  environment empty, so the next thing D-Bus spawns is broken while the thing
  you tested by hand works. That asymmetry is why this bug survives testing.

## Symptom to cause

The full table with verification commands is in `reference.md`. The short form:

| Symptom | First thing to check |
|---|---|
| GTK file chooser hangs ~25 s then falls back to the toolkit dialog | `WAYLAND_DISPLAY` missing from the D-Bus activation environment (25 s is the D-Bus activation timeout, not a hint about the file system) |
| Screen share offers no sources, or silently produces nothing | `XDG_CURRENT_DESKTOP` unset or wrong, so the portal picked the wrong backend; or no ScreenCast-capable backend installed |
| A `DBusActivatable=true` `.desktop` entry does nothing | the D-Bus activation environment, again — the service starts with no display to connect to |
| Cursor is a black X11 arrow, or invisible over some surfaces | `XCURSOR_THEME`/`XCURSOR_SIZE` not exported before clients start, and not mirrored into `gsettings` |
| XWayland apps unstyled, wrongly scaled, or absent | XWayland not enabled, or `DISPLAY` never imported the same two ways |

## When you touch any of this

1. Change the ordering in one place — the session entry — not in a unit file
   and the entry script both. Two places means two orders.
2. Every unit that draws or talks to the compositor gets
   `PartOf=graphical-session.target` and `After=graphical-session.target`, and
   is started by a target rather than by the entry script directly. That is what
   makes a crashed bar restartable without taking the session with it.
3. Add a matching check to `helm ctl doctor`. Every item in this skill should be
   something `doctor` can confirm, because the user hits it before we do.
4. Add or update the row in `docs/PITFALLS.md`, naming the guard.
5. **This cannot be tested in the agent container** — there is no Wayland
   display, no GPU and no D-Bus session (`.claude/memory/20-environment.md`).
   Anything needing a live session is verified in CI or on real hardware, and
   the issue carries `needs-human` rather than a guess.

## Reference

`reference.md` has the full symptom table with the exact verification command
for each, the systemd unit relationships, the portal configuration shape, and
the checks `helm ctl doctor` owes the user. Open it when diagnosing a specific
symptom or writing the units.
