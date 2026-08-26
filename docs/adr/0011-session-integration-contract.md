# ADR 0011 — The session entry owns the environment handshake

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

This is the single most common way a minimal Wayland desktop breaks for users,
and it does not look like a bug in the desktop.

The user logs in. Everything appears to work. Then they press Ctrl+O in Firefox
and nothing happens for about twenty-five seconds. Or screen sharing offers no
windows. Or the cursor is a black X11 arrow. None of these point at the desktop
environment, so the user files a bug against the browser.

The cause is almost always the same. `xdg-desktop-portal` is D-Bus activated: it
inherits the bus's activation environment, not the environment of whatever
started the session. Without `WAYLAND_DISPLAY` there it cannot reach the
compositor. Without `XDG_CURRENT_DESKTOP` it cannot choose a backend. The
twenty-five seconds is a D-Bus activation timeout expiring.

Two mechanisms have to be fed, and feeding one is not enough:

- `systemctl --user import-environment` puts variables into the **systemd user
  manager's** environment, which is what systemd-activated user units inherit.
- `dbus-update-activation-environment --systemd` puts variables into the
  **D-Bus session bus's** activation environment, which is what bus-activated
  services inherit.

Desktops that set only the first still break bus-activated portals. Desktops
that set only the second still break systemd user units. helm must do both, and
must do both *before* starting anything that could trigger an activation.

`docs/PITFALLS.md` has a whole section for this called "the classic killers" and
`docs/MVP.md` lists working portals as MVP capability 11, because "browsers and
Electron apps are non-negotiable in a work week".

## Decision

The session entry point is a contract, not a convenience script. It performs the
steps below in this order, and `helm ctl doctor` verifies each one at runtime.

### The checklist a packager must follow

**1. Set the identity variables before anything else.**

```
XDG_CURRENT_DESKTOP=helm
XDG_SESSION_TYPE=wayland
XDG_SESSION_DESKTOP=helm
```

`XDG_CURRENT_DESKTOP` selects the portal backend. It must be `helm`, and the
portal configuration in `configs/portal/helm-portals.conf` must name a backend
for `helm` or portals will find nothing.

**2. Start the compositor and wait for `WAYLAND_DISPLAY` to be real.**

The compositor assigns the socket name. Do not guess it, and do not proceed
until the socket exists on disk under `$XDG_RUNTIME_DIR`. A race here produces
exactly the same symptoms as forgetting the import.

**3. Import into BOTH environments, before starting anything else.**

```
systemctl --user import-environment \
    WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP \
    XDG_RUNTIME_DIR DISPLAY XCURSOR_THEME XCURSOR_SIZE

dbus-update-activation-environment --systemd \
    WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP \
    XDG_RUNTIME_DIR DISPLAY XCURSOR_THEME XCURSOR_SIZE
```

Both. Every time. This is the step that is skipped.

**4. Only now start the session daemon, the bar and the clients.**

`helm-session` is a systemd user unit ordered after the import. Clients are
separate restartable units bound to it (ADR 0003), so a crashed bar does not
end the session.

**5. Declare a portal backend dependency.**

The packages must depend on `xdg-desktop-portal` plus at least one backend.
helm's own choice for M3 is `xdg-desktop-portal-gtk` for the file chooser and
`xdg-desktop-portal-wlr` for screencast, with
`xdg-desktop-portal-termfilechooser` pointed at yazi as the charon stopgap
(ADR 0007). Ship `configs/portal/helm-portals.conf` mapping each interface to a
backend for `XDG_CURRENT_DESKTOP=helm`.

**6. Set the cursor theme and size in both places.**

`XCURSOR_THEME` and `XCURSOR_SIZE` in the environment (and therefore in both
imports), *and* the matching `gsettings set org.gnome.desktop.interface
cursor-theme/cursor-size`. Environment covers Wayland and XWayland clients; the
gsettings keys cover GTK. Setting only one leaves the cursor wrong somewhere.

**7. State the XWayland policy explicitly.**

XWayland is enabled. Scaling is integer only for X11 clients; fractional scaling
is not applied to them, because X11 has no protocol for it and the result is
blur (`docs/PITFALLS.md`). X11 client colours come from an `Xresources` file
generated from the same `palette.toml` (ADR 0005).

**8. Ship an idle and lock unit.**

An idle daemon and a lock screen, started as part of the session, wired to
lid-close and to `loginctl lock-session`. A laptop that closes its lid and stays
unlocked is a security failure, not a missing feature.

**9. `helm ctl doctor` verifies every one of the above.**

Not a subset. The doctor is what turns this contract from documentation into
something that fails loudly. Its checks:

| Check | Covers |
|---|---|
| `WAYLAND_DISPLAY` and `XDG_CURRENT_DESKTOP=helm` present in the systemd user environment **and** in the D-Bus activation environment, separately reported | steps 1, 3 |
| A portal backend answers on `org.freedesktop.portal.Desktop`, the config names one for `helm`, and `FileChooser` and `ScreenCast` both resolve | step 5 |
| Cursor theme resolvable and gsettings agrees with the environment | step 6 |
| XWayland running if enabled; scale policy as configured | step 7 |
| Idle and lock units active | step 8 |
| Font stack covers the glyph inventory (ADR 0012); reused tools present at their version floors (ADR 0007) | dependencies |
| `helm-session` socket present and answering `Hello` | session up |

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Let systemd's graphical session target handle it** (`graphical-session.target` with a generator) | The modern, correct-looking approach; less shell in the session entry; better integration with logind | It only covers the systemd side. Bus-activated services still inherit the bus's activation environment, so the portal problem survives untouched. It is a good idea *in addition to* the explicit import, not instead of it |
| **Set the variables in `~/.profile` or a PAM environment file** | Applies before anything starts; no ordering problem | `WAYLAND_DISPLAY` is not known until the compositor is running, so it cannot be set ahead of time. Splitting the mechanism across two places also makes it harder to verify |
| **Document the requirement and rely on the display manager** | Display managers already set some of this; less code for us | Display manager behaviour varies enormously and the user may be starting helm from a TTY. Relying on it means helm works for some users and mysteriously does not for others, which is the current state of the art and is what this ADR rejects |

## Consequences

### Good

- The most common minimal-Wayland-desktop failure is closed by construction, and
  verified by `doctor` rather than by hope.
- The failure mode, when it does occur, produces a diagnosis instead of a
  twenty-five second hang.
- Packagers get a checklist rather than folklore. Steps 1 to 8 are the whole job.
- The contract is testable in the NixOS VM (ADR 0010), which is the only place
  a full session can be exercised in CI.

### Bad

- The session entry is shell, and shell in the critical path is unpleasant. It
  must be defensive about a missing `dbus-update-activation-environment` binary,
  a missing `systemctl`, and a compositor that fails to start.
- The variable list in step 3 is a thing that will be forgotten when a new
  variable is added. The doctor check must be extended in the same commit.
- Ordering is genuinely fragile: anything that starts before step 3 and touches
  D-Bus poisons the activation environment with a missing `WAYLAND_DISPLAY`.
- We take on a hard dependency on portal backends we do not write.

### Neutral

- Much of this is what every Wayland desktop does. The difference is writing it
  down and testing it.

## Reversal

Low. The contract lives in the session entry script, the systemd user units and
the doctor's check list, all under `packaging/` and `crates/helm-ctl`. Changing
the mechanism is a rewrite of one script and one check module. The *requirement*
is not reversible: it is imposed by how D-Bus activation works.

## Guard

- *Planned (M3):* the NixOS VM test boots the session and runs
  `helm ctl doctor`, failing the build on any non-zero exit. This is the guard.
- *Planned (M3):* a negative test that deliberately skips the
  `dbus-update-activation-environment` call and asserts `doctor` reports it.
  A health check that has never been seen to fail is not a health check.
- *Planned (M3):* a portal round-trip test in the VM asserting a `FileChooser`
  request answers within two seconds, which is what the hang violates.
- *Planned (M3):* a consistency test asserting the variable list in the session
  entry matches the list the doctor checks.

## Needs a human

**Which lock screen does helm ship?** This is a security decision and it is not
one the repository can make on its own.

1. **`swaylock` (or `swaylock-effects`).** Ubiquitous, packaged everywhere,
   well understood. It is not a Wayland session-lock protocol client in older
   versions, which means on some compositors a crash can leave the session
   unlocked. It also cannot be themed from `palette.toml` beyond colours.
2. **`gtklock`.** Uses `ext-session-lock-v1` properly, so a crash leaves the
   session locked rather than exposed. GTK-based, so it drags GTK into the lock
   path but is themable through the same `gtk.css` we already generate.
3. **`waylock`.** Minimal, `ext-session-lock-v1`, very small attack surface.
   Zig, a toolchain we otherwise do not have (see ADR 0002).
4. **Write `helm-ward`.** Consistent with the design, but getting a lock screen
   wrong is the worst class of bug in a desktop. Not before M6, if ever.

**Recommendation: `gtklock` for M3.** `ext-session-lock-v1` is the property that
matters — the compositor keeps the session locked even if the locker crashes —
and GTK is already a theming target so it costs us nothing new. `waylock` is the
better answer on minimalism grounds if the Zig dependency is acceptable.

A human must also confirm the **idle policy defaults**: how long until the
screen blanks, how long until lock, and whether lid-close locks unconditionally.
These are user-visible security defaults and should not be guessed.

Tracked as a `needs-human` issue (standing order S3).
