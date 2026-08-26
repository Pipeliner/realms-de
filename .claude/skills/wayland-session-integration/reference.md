# Wayland session integration reference

Verification commands, unit relationships and the portal shape. The ordering
contract and the rules live in `SKILL.md`. Everything here targets **M3**;
none of helm's own session files exist yet, so the commands below are how you
check a running system, not a description of code in this repo.

## Contents

- [Symptom to cause, with verification](#symptom-to-cause-with-verification)
- [Verifying the handshake itself](#verifying-the-handshake-itself)
- [systemd user units](#systemd-user-units)
- [Portals](#portals)
- [Cursor](#cursor)
- [XWayland](#xwayland)
- [What `helm ctl doctor` owes the user](#what-helm-ctl-doctor-owes-the-user)

## Symptom to cause, with verification

| Symptom | Cause | Verify |
|---|---|---|
| GTK file chooser hangs about 25 s, then falls back to the toolkit's own dialog | `WAYLAND_DISPLAY` absent from the D-Bus activation environment; the portal service starts, cannot connect to the display, and D-Bus times out after 25 s | `busctl --user get-property org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop org.freedesktop.portal.FileChooser version` — should answer immediately, not after a pause |
| Screen share dialog lists no windows or outputs, or capture produces nothing and no error | `XDG_CURRENT_DESKTOP` unset or wrong at portal start, so `portals.conf` matching failed and a backend with no ScreenCast was chosen; or no capable backend is installed | `busctl --user introspect org.freedesktop.portal.Desktop /org/freedesktop/portal/desktop \| grep -i screencast` and `systemctl --user show-environment \| grep XDG_CURRENT_DESKTOP` |
| A `.desktop` entry with `DBusActivatable=true` does nothing when launched | the D-Bus activation environment has no display, so the activated service exits immediately | `gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetId` works, then compare `systemctl --user show-environment` against `env` |
| Cursor is the default black X11 arrow, or vanishes over some surfaces | `XCURSOR_THEME` / `XCURSOR_SIZE` not exported before clients started, or not mirrored into gsettings for GTK | `systemctl --user show-environment \| grep XCURSOR` and `gsettings get org.gnome.desktop.interface cursor-theme` |
| Nothing themed after login, though `helm ctl theme apply` works by hand | user units started before the import, so they inherited an empty environment | `systemctl --user show-environment` compared against the compositor process's own `/proc/<pid>/environ` |
| XWayland apps do not start, or are scaled wrongly | XWayland disabled in the compositor, or `DISPLAY` set in the shell but never imported into systemd and D-Bus | `xlsclients` and `systemctl --user show-environment \| grep DISPLAY` |
| Screen stays unlocked when the lid closes | no idle/lock unit is part of `graphical-session.target` | `systemctl --user list-dependencies graphical-session.target` |

## Verifying the handshake itself

The three views must agree. Any disagreement is the bug:

```sh
# 1. what the compositor itself has
tr '\0' '\n' < /proc/$(pgrep -x niri | head -1)/environ | grep -E 'WAYLAND_DISPLAY|XDG_'

# 2. what systemd will give a user unit
systemctl --user show-environment | grep -E 'WAYLAND_DISPLAY|XDG_|XCURSOR'

# 3. what D-Bus will give an activated service
gdbus call --session \
  --dest org.freedesktop.DBus \
  --object-path /org/freedesktop/DBus \
  --method org.freedesktop.DBus.ListNames | tr ',' '\n' | grep portal
busctl --user get-property org.freedesktop.systemd1 /org/freedesktop/systemd1 \
  org.freedesktop.systemd1.Manager Environment
```

If (1) has `WAYLAND_DISPLAY` and (2) or (3) does not, the import ran too early
or was skipped. That is the whole diagnosis, every time.

## systemd user units

```
              graphical-session-pre.target
                        │
              graphical-session.target        ← the compositor reaches this
                 │        │        │             once WAYLAND_DISPLAY is
                 │        │        │             imported
        helm-bar.service  │   xdg-desktop-portal.service
                          │
                 helm-idle.service (lock, dpms)
```

Rules:

- Every helm client unit: `PartOf=graphical-session.target` (so it stops with
  the session) and `After=graphical-session.target` (so it starts after the
  import). `BindsTo=` is stronger than usually wanted — a bar that fails should
  not tear the session down.
- The compositor is *not* a unit that clients depend on directly; they depend on
  the target. That is the seam that lets `NiriBackend` be swapped for
  `NativeBackend` in M5 (ADR 0002) without touching a single unit file.
- Clients are `Restart=on-failure` with a bounded `RestartSec`. "A crashed bar
  taking the session with it" is a listed pitfall; restartability is the answer.
- The session entry starts one target, never a list of services. Adding a client
  should mean adding a unit, not editing the entry script.

## Portals

- `XDG_CURRENT_DESKTOP=helm` is what `portals.conf` matches on. It must be set
  before `xdg-desktop-portal` starts, which in practice means before anything
  can D-Bus-activate it.
- Ship a `helm-portals.conf` naming a concrete backend per interface rather than
  relying on the default, so behaviour does not change with what happens to be
  installed. Packages must depend on a backend: "no portal backend installed"
  presents to the user as "Open File does nothing in Firefox", with no error
  anywhere.
- helm's own file dialog (`charon portal`, via
  `xdg-desktop-portal-termfilechooser` pointed at yazi) is **M4**. The MVP uses
  the toolkit's own dialog, themed (`docs/MVP.md`).
- A portal that answers on D-Bus is not proof it works. Test the actual
  interaction: open a file in a real GTK app and share a screen in a browser.

## Cursor

Three places, all of them required, because three different client families read
three different sources:

1. `XCURSOR_THEME` and `XCURSOR_SIZE` exported in the session entry (Wayland
   clients and XWayland).
2. Both imported into systemd and D-Bus with everything else (activated apps).
3. `gsettings set org.gnome.desktop.interface cursor-theme` and `cursor-size`
   (GTK's own preference path).

The theme itself is generated from `palette.toml` like everything else — see the
`helm-theming` skill.

## XWayland

- Enable it explicitly, with a stated scaling policy, rather than leaving it to
  the compositor's default.
- `DISPLAY` gets the same two-way import as `WAYLAND_DISPLAY`.
- XWayland apps are themed from the same palette via `Xresources`; that is a
  template like any other.

## What `helm ctl doctor` owes the user

`doctor` exists so a user diagnoses this without us. Each check should print the
symptom it prevents, not just a pass mark:

1. `WAYLAND_DISPLAY` present in the process, in systemd, and in D-Bus — three
   separate checks, because they fail separately.
2. `XDG_CURRENT_DESKTOP=helm` and `XDG_SESSION_TYPE=wayland`, likewise.
3. A portal backend answers, and it implements FileChooser and ScreenCast.
4. Cursor theme resolvable and set in gsettings.
5. Font stack covers the glyph inventory — `helm-core::glyphs::Probe::summary()`
   already produces exactly this line.
6. Session protocol version matches `helm-core::ipc::PROTOCOL_VERSION`.
7. The `WmBackend`'s reported `Capabilities` (`docs/INTERFACES.md` §1),
   including its `unsupported` list, so a niri-shaped gap is visible rather than
   mysterious.
