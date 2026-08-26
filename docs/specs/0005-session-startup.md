# SPEC 0005 — Session startup and desktop integration

- **Status:** Draft — open questions below are unresolved (`needs-human`)
- **Milestone:** M3
- **Decisions:** [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md),
  [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0010](../adr/0010-nix-flake-as-reference-build.md)
- **Supersedes / Superseded by:** —
- **Implements:** ADR 0011's packager checklist, steps 1–9

> Written before the code, as S14 requires. The **Test** column is deliberately
> empty. Almost every row here is an integration test that cannot run in a
> container, so each is marked **CI**, **VM** or **HARDWARE**; the HARDWARE rows
> need a human and are flagged as such under standing order S3.
>
> This spec says what *should* be true. `packaging/` is being written in
> parallel and diverges from it in several places; those divergences are listed
> in the report accompanying this file, not fixed here.

## Purpose

A helm login must produce a desktop in which file dialogs open, screen sharing
works, the cursor is helm's cursor, and a crashed component is repaired rather
than fatal. None of that follows from the compositor starting. It follows from
an ordering contract between five things that do not know about each other: the
display manager, river, helm's window manager, the systemd user manager and the
D-Bus session bus.

This is the single most common way a minimal Wayland desktop breaks, and it
breaks *late* and *elsewhere*: everything looks fine until the first Ctrl+O
hangs for twenty-five seconds, and the user files a bug against Firefox. Under
river 0.4 the stakes are higher still, because the compositor does no window
management of its own — a session that comes up in the wrong order can be not
merely unthemed but literally unusable, with windows that are never placed.

Without this component helm is a compositor with an environment hole in it.

## Scope

**In:** the session entry `helm-session` and the order it does things in; the
exact environment set and where it is published; discovery of `WAYLAND_DISPLAY`
and `DISPLAY`; the systemd user units and their supervision policy; portal
backend selection and verification; the non-systemd and non-D-Bus degraded
paths; session teardown; the log-line contract; and the list of checks
`helm ctl doctor` owes the user.

**Out:**

- The implementation of `doctor` itself — SPEC 0006. This spec names the checks
  and says what each must prove; it does not say how they are written.
- The window manager's protocol work: `river-window-management-v1`,
  `river-layer-shell-v1`, `river-xkb-bindings-v1`,
  `river-input-management-v1` — [INTERFACES.md §1](../INTERFACES.md), M2.
- Theme generation and the palette — [SPEC 0002](0002-theme-pipeline.md). This
  spec only requires that the cursor theme name reaches three places.
- Distribution packaging mechanics (deb/rpm/flake layout) — `packaging/`,
  `docs/INSTALL.md`. This spec constrains what those must install and enable.
- Choosing the lock screen and the idle defaults — see **Open questions**.

## Behaviour

### Naming, so the rest of this reads unambiguously

| Name | What it is |
|---|---|
| `helm-session` | The **session entry**: the shell script a display manager execs. It owns the ordering contract. |
| `helm-sessiond` | The **window manager and session daemon** binary (crate `helm-session`, ADR 0003). Under river it *is* the window manager. |
| `helm-session.target` | The systemd user target that anchors everything needing a display. |
| `helm-daemon.service` | The unit that supervises `helm-sessiond`. |

The four-way collision between these names is a real hazard for readers and is
raised as **OQ-4**.

### 1. The startup sequence

The order is the contract. Each step below says what breaks if it moves.

#### Step 1 — Identity, before anything is started

```sh
export XDG_CURRENT_DESKTOP=helm
export XDG_SESSION_TYPE=wayland
export XDG_SESSION_DESKTOP=helm
export XCURSOR_THEME="$HELM_CURSOR_THEME"
export XCURSOR_SIZE="$HELM_CURSOR_SIZE"
```

`XDG_RUNTIME_DIR` is *not* set here. It is set by `pam_systemd` at login and is
not ours to invent. If it is unset or is not a directory, the entry exits
immediately with `FATAL NO-RUNTIME-DIR`: nothing Wayland works without it, and
proceeding produces a cascade of unrelated-looking errors.

**If this moves later:** portal backend selection reads `XDG_CURRENT_DESKTOP`
at the moment `xdg-desktop-portal` is *activated*, which can be before any helm
code runs — the first D-Bus-activated application in the session wins. A late
`XDG_CURRENT_DESKTOP` means a portal that already chose the wrong backend, and
the symptom is screen sharing that offers no sources with no error anywhere.
Cursor theme is read once by each client as it starts, so a late `XCURSOR_*`
leaves an X11 black arrow over anything that started early.

#### Step 2 — Start river, in the background, never `exec`

The entry must survive the compositor: it has work to do afterwards, and it is
the session's teardown owner. river is started as a child, its pid recorded, and
the entry's own pid written to `$XDG_RUNTIME_DIR/helm/session.pid` for the abort
path in §2.

river is started with no helm-specific arguments. In particular the entry does
**not** use a compositor-side startup hook to launch the window manager, because
that would run it before step 4.

**If this moves earlier (before step 1):** river inherits no identity, and every
client it spawns inherits the hole.

#### Step 3 — Wait for `WAYLAND_DISPLAY` to actually exist

Not a sleep. Never a sleep. The socket name is assigned by the compositor and
must be discovered, in two phases:

1. **Discovery.** Before starting river, snapshot the set of
   `$XDG_RUNTIME_DIR/wayland-*` entries, ignoring `*.lock`. After starting it,
   poll that set every 50 ms until a *new* name appears. The new name is
   `WAYLAND_DISPLAY`. Guessing `wayland-0` or `wayland-1` is wrong: a nested
   session, another user on the same machine, or a leftover socket all shift
   the number.
2. **Liveness.** The socket file appearing proves `bind(2)`, not that the
   compositor is dispatching. Confirm with a connect-and-roundtrip probe —
   connect, bind `wl_registry`, `wl_display_roundtrip` — and proceed only when
   it returns. `helm ctl wait-display --timeout <s>` is the required
   implementation of this probe and is a hard requirement on `helm-ctl`. Until
   it exists, the entry falls back to file existence alone and logs
   `DEGRADED NO-DISPLAY-PROBE`, because file existence is a weaker claim than
   the one the next step relies on.

Both phases run under one deadline (`HELM_WAIT_SECONDS`, default 10) and abort
early if the compositor pid dies. On expiry: `FATAL NO-SOCKET`, naming the
runtime directory that was watched.

**If this moves, or is replaced by a fixed delay:** everything downstream
inherits an empty `WAYLAND_DISPLAY`. `systemctl --user import-environment
WAYLAND_DISPLAY` on an unset variable is **not an error** — it is a silent
no-op that is indistinguishable from success in the logs. This is the single
most expensive line in the whole document.

#### Step 4 — Publish the environment into both launchers, before any client

```sh
systemctl --user import-environment \
    WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP \
    XDG_RUNTIME_DIR XCURSOR_THEME XCURSOR_SIZE

dbus-update-activation-environment --systemd \
    WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP \
    XDG_RUNTIME_DIR XCURSOR_THEME XCURSOR_SIZE
```

Both, every time. They are two different launchers with two different
environments: `systemctl --user import-environment` feeds the **systemd user
manager**, which is what user units inherit; `dbus-update-activation-environment`
feeds the **session bus's activation environment**, which is what bus-activated
services inherit. A desktop that does only the first still has hanging portals.

`dbus-update-activation-environment --systemd` also forwards to the systemd user
manager, so on a healthy box the two calls overlap. That redundancy is
deliberate: either binary can be absent, and the failure modes are different.
On a box with no systemd user manager the call must be made **without**
`--systemd`, or it fails trying to reach `org.freedesktop.systemd1` and masks
whether the D-Bus half succeeded.

**If this moves earlier:** it imports nothing (step 3). **If this moves later —
after any client:** the client itself works, because it inherited the entry's
environment directly, but the *activation* environment is still empty, so the
next thing D-Bus spawns is broken while the thing you tested by hand is fine.
That asymmetry is precisely why this bug survives manual testing.

`DISPLAY` is handled separately; see §3.

#### Step 5 — Mirror the cursor into gsettings

```sh
gsettings set org.gnome.desktop.interface cursor-theme "$HELM_CURSOR_THEME"
gsettings set org.gnome.desktop.interface cursor-size  "$HELM_CURSOR_SIZE"
```

**Why here and not earlier:** `gsettings` writes through dconf, which is
D-Bus-activated. Running it before step 4 activates `ca.desktop.dconf` with an
empty activation environment and poisons it for the rest of the session. It is
the first and easiest violation of "start nothing between steps 2 and 4".

Missing `gsettings` or missing `gsettings-desktop-schemas` is
`DEGRADED NO-GSETTINGS`, not fatal: Wayland clients still get the cursor from
`XCURSOR_*`, GTK apps do not.

#### Step 6 — Start `helm-session.target`

One target, never a list of services. Adding a client to helm means adding a
unit and a `.wants` symlink, not editing this script.

```sh
systemctl --user start helm-session.target
```

**This step's exit status is not a success signal.** See §2 and failure mode
**N1**: a unit whose `ConditionEnvironment=` is unmet is *skipped*, the target
still activates, and `systemctl start` still exits 0. After starting the target
the entry must verify, per unit:

```sh
systemctl --user show -p ActiveState -p ConditionResult -p Result helm-daemon.service
```

`ActiveState=active` is the only acceptable answer for `helm-daemon.service`.
`ConditionResult=no` is reported as `FATAL WM-ABORT` with the specific message
that the environment handshake did not reach the user manager.

#### Step 7 — Wait on the compositor; teardown on its exit

The compositor exiting *is* the end of the session. The entry `wait`s on it and
runs the teardown in §7 from an `EXIT INT TERM HUP` trap.

### 2. The river-specific ordering problem

Under river 0.4 the compositor performs no window management. Between step 2 and
the moment `helm-sessiond` binds `river_window_manager_v1`, there is a window in
which river is running and nothing is placing windows.

**What the user sees in that window.** The background and the cursor, and
nothing else. There is no bar, because `wlr-layer-shell` is served under river
only if the window manager implements `river-layer-shell-v1` — so during the
gap even a running `helm-bar` maps nothing. Any window opened in the gap exists
but is unplaced and, depending on river's defaults, may not be drawn at all.
The correct name for this state is "inert", and it is indistinguishable by eye
from a crashed session.

**Requirements:**

1. The gap is bounded and measured. `helm-sessiond` reaches readiness within the
   cold-start budget (§Budgets). `Type=notify` with `sd_notify(READY=1)` sent
   once the window-management global **and** the layer-shell manager global are
   both bound is the required readiness definition, so that "the target is
   active" and "windows can be placed" are the same statement. (**OQ-6**: the
   current unit is `Type=exec` pending the daemon existing.)
2. **`helm-sessiond` never connects.** After the start limit is reached the
   entry must not leave the user staring at an inert compositor. It logs
   `FATAL WM-ABORT`, with the one-line explanation *"river is running but helm
   is not managing it: no window can be placed. See <log path>."*, tears the
   session down and returns the user to the display manager. A returned login
   is strictly better than a black screen, because the black screen has no
   surface on which to tell the user anything — with no window manager there is
   no layer shell, so helm cannot draw its own error.
3. **`helm-sessiond` dies later.** Supervised restart, with the ledger recovered
   rather than lost. On every ledger mutation the daemon queues a snapshot to
   `$XDG_RUNTIME_DIR/helm/ledger.snapshot`; the write happens on a separate
   thread and never on the protocol input path, because under river a stall is a
   session failure and not a slow frame (ADR 0013). On restart, if the snapshot's
   compositor instance id matches the running river, the ledger is restored:
   windows river no longer reports are dropped, windows the snapshot does not
   know about are appended in river's order.
4. **river refuses the connection.** Only one window-management client may
   connect; river answers `unavailable` to a second. A leftover `helm-sessiond`
   from a previous session, or one started by hand, therefore makes the
   supervised one unstartable *forever* — and a naive `Restart=` turns that into
   a permanent restart loop. See failure mode **N2**.

**Supervision policy, stated so the unit can be written from it:**

| Property | Value | Reason |
|---|---|---|
| `Restart=` | `always` | Quit is expressed through river's `exit_session`, not through the daemon exiting (see below), so a clean exit on its own is not a normal path and must not leave river unmanaged. |
| `RestartSec=` | `1s` | Every second here is a second the user cannot move a window. |
| `RestartPreventExitStatus=` | `69 78` | `69` = river answered `unavailable`; `78` = protocol version mismatch against the pinned river. Restarting cannot help with either, and looping hides the message. |
| `StartLimitIntervalSec=` / `StartLimitBurst=` | `30` / `5` | Five failures in thirty seconds is a real bug. It must surface as a dead unit, not a hot laptop. |
| `OnFailure=` | `helm-session-abort.service` | Fires when the unit enters `failed`, which with `Restart=always` means *after* the start limit is hit — exactly once, at the right moment. That unit runs `helm-session --abort`, which reads `$XDG_RUNTIME_DIR/helm/session.pid` and signals the entry to tear down (requirement 2). |
| `TimeoutStopSec=` | `10s` | Bounded, so teardown cannot hang on it. |
| `Slice=` | `session.slice` | The window manager is essential to the session; under memory pressure it must not be the first thing killed. |

**Quit is not the daemon exiting.** `Request::Quit` (ADR 0004) is implemented as
river's `exit_session`, which is documented as being for user-requested logout.
river exits, the entry's `wait` returns, teardown runs. This keeps a single
shutdown path and is what makes `Restart=always` safe.

### 3. The environment set

Exactly this set is exported, and exactly this subset is imported. The list
exists in one place in the source; SPEC 0006's `doctor` reads the same list, and
a CI test asserts they are equal (A3).

| Variable | Set by | Exported before river | Imported | Why it is in the set |
|---|---|---|---|---|
| `XDG_CURRENT_DESKTOP=helm` | entry | yes | **yes** | Selects the portal backend: `xdg-desktop-portal` matches this (split on `:`) against `helm-portals.conf`. Also drives `OnlyShowIn`/`NotShowIn` in `.desktop` files. |
| `XDG_SESSION_DESKTOP=helm` | entry | yes | **yes** | The session's identity for logind and for tools that read it instead of the above. Cheap to set, confusing to omit. |
| `XDG_SESSION_TYPE=wayland` | entry | yes | **yes** | Toolkit backend selection. logind usually sets it, but a start from a bare TTY does not, and helm must work from a TTY. |
| `XDG_RUNTIME_DIR` | `pam_systemd` | inherited | **yes** | Not ours to set; imported because a bus-activated service that cannot find the runtime directory cannot find any socket in it. Fatal if absent. |
| `WAYLAND_DISPLAY` | river, discovered in step 3 | no (does not exist yet) | **yes** | The display socket. This is the one whose absence costs twenty-five seconds. |
| `DISPLAY` | XWayland, discovered (§XWayland) | no | **yes, when known** | XWayland clients, and D-Bus-activated X11 applications. |
| `XCURSOR_THEME` | entry | yes | **yes** | Cursor for Wayland and XWayland clients; read once at client start. |
| `XCURSOR_SIZE` | entry | yes | **yes** | Same. A size set in only one place gives a cursor that changes size as it crosses a window. |

**Exported but deliberately not imported** — toolkit preferences, not session
facts:

| Variable | Value | Why not imported |
|---|---|---|
| `MOZ_ENABLE_WAYLAND` | `1` | A wrong value stuck in the user manager outlives the session and is very hard to diagnose. Anything launched from the session inherits it anyway. |
| `QT_QPA_PLATFORM` | `wayland;xcb` | Ordered preference with an X11 fallback, so a Qt app without Wayland support still starts. |
| `QT_WAYLAND_DISABLE_WINDOWDECORATION` | `1` | helm draws the seams; Qt must not add its own. |
| `GDK_BACKEND` | `wayland,x11` | Forcing this into every user unit breaks GTK apps that legitimately want X11. |
| `_JAVA_AWT_WM_NONREPARENTING` | `1` | Java toolkits assume a reparenting WM; without it menus land in the top-left corner under every tiling WM. |

**`PATH` is not in the import set.** Importing it replaces the user manager's
`PATH` for every user unit for the lifetime of the manager, including units that
have nothing to do with helm, and it persists after logout on a lingering user.
Where a `PATH` fix is genuinely needed — a Nix profile that the user manager
does not know about — it is opt-in through `HELM_IMPORT_PATH=1`. See **OQ-3**;
the current session entry imports it unconditionally.

**XWayland.** Enabled (ADR 0011 step 7). Scaling for X11 clients is integer
only: X11 has no protocol for fractional scale and applying one produces blur
(`docs/PITFALLS.md`). X11 client colours come from a generated `Xresources`
(ADR 0005, SPEC 0002).

`DISPLAY` is discovered by the same snapshot-and-diff technique as the Wayland
socket, against `/tmp/.X11-unix/X*`: snapshot before starting river, and take
the new entry afterwards. `X<N>` yields `DISPLAY=:<N>`. wlroots creates the
listening socket up front even in lazy mode, so this resolves at compositor
start rather than at first X client. If no new socket appears within the same
deadline, `DEGRADED NO-XWAYLAND` is logged and `DISPLAY` is simply absent from
both imports — a stated gap rather than an imported empty string. *Unverified
for river 0.4.8: whether river enables XWayland by default and whether the
socket is created eagerly must be confirmed on a real river before this is
relied upon.*

`doctor` must not shell out to `xlsclients` or `xdpyinfo` to check this: neither
is guaranteed installed on any of the three targets. It connects to
`/tmp/.X11-unix/X<N>` itself.

### 4. systemd user units

```
                graphical-session-pre.target
                            │
   helm-session.target ─────┤  BindsTo= + Before= graphical-session.target
                            ▼
                  graphical-session.target
                     │            │            │
        helm-daemon.service   helm-bar.service   xdg-desktop-portal.service
        (the window manager)   (Wants=daemon)    (upstream; PartOf= the target)
                     │
             helm-idle.service  ← blocked on OQ-1
```

| Unit | `[Unit]` | `[Service]` | `[Install]` |
|---|---|---|---|
| `helm-session.target` | `BindsTo=graphical-session.target`, `Before=graphical-session.target`, `Wants=graphical-session-pre.target`, `After=graphical-session-pre.target` | — | **none** |
| `helm-daemon.service` | `PartOf=graphical-session.target`, `After=graphical-session.target`, `ConditionEnvironment=WAYLAND_DISPLAY`, `StartLimitIntervalSec=30`, `StartLimitBurst=5`, `OnFailure=helm-session-abort.service` | `Type=notify`, `Restart=always`, `RestartSec=1`, `RestartPreventExitStatus=69 78`, `TimeoutStopSec=10`, `Slice=session.slice` | `WantedBy=helm-session.target` |
| `helm-bar.service` | `PartOf=graphical-session.target`, `After=graphical-session.target helm-daemon.service`, `Wants=helm-daemon.service`, `ConditionEnvironment=WAYLAND_DISPLAY`, `StartLimitIntervalSec=30`, `StartLimitBurst=5` | `Type=exec`, `Restart=on-failure`, `RestartSec=1`, `TimeoutStopSec=5`, `Slice=app.slice` | `WantedBy=helm-session.target` |

The reasoning behind each relationship, because these are easy to copy wrongly:

- **`BindsTo=graphical-session.target` on the target, plus `Before=`.** `BindsTo`
  implies `Requires`, so starting `helm-session.target` brings
  `graphical-session.target` up, and anything else on the system that keys off
  `graphical-session.target` — a notification daemon, an idle daemon, the
  upstream portal unit — works under helm without knowing what helm is. The
  `Before=` is what makes `After=graphical-session.target` on the client units a
  real ordering barrier rather than a coincidence of an empty target activating
  quickly. This is systemd's documented shape for a session unit
  (`systemd.special(7)`).
- **`PartOf=`, never `BindsTo=`, on the clients.** `PartOf` propagates *stop*
  downwards: ending the session stops the clients. `BindsTo` would additionally
  propagate *failure* upwards, so a crashed bar would take the session with it —
  which is a listed pitfall, not a design.
- **`Wants=`, never `Requires=`, from the target to the clients.** A bar that
  cannot start must leave a usable desktop.
- **`Wants=helm-daemon.service` from the bar.** The bar reconnects to the
  control socket on its own, so a window manager that is briefly down — a
  restart, exactly the case §2 designs for — must not stop the bar running.
- **Different slices.** The window manager is in `session.slice`; the bar is in
  `app.slice`. Under memory pressure the bar should be reaped first, because it
  is restartable and the window manager is the session.
- **No `[Install]` section on the target.** It is started explicitly by the
  entry. A `WantedBy=default.target` would start it at boot for a lingering
  user, with no compositor and no display, and every unit under it would be
  silently condition-skipped.
- **The `.wants` symlinks must be shipped by the package.** `[Install]` only
  takes effect when something processes it (`systemctl --user enable`,
  `dh_installsystemduser`, an rpm preset). Relying on `[Install]` alone is not
  portable across the three packagers, and if the symlinks are missing then
  `systemctl --user start helm-session.target` starts *nothing at all* and
  reports success. Ship
  `lib/systemd/user/helm-session.target.wants/helm-daemon.service` and
  `.../helm-bar.service` as real symlinks in every package.
- **`ConditionEnvironment=` requires systemd ≥ 246**, which all three targets
  exceed. It is worth having because it converts "started with an empty
  display" into "did not start" — but see **N1**: it converts it into a *silent*
  did-not-start, which the entry and `doctor` must both check for explicitly.

### 5. Portals

**Backend selection.** `xdg-desktop-portal` (≥ 1.18, which all three targets
ship) reads, in order: `$XDG_CONFIG_HOME/xdg-desktop-portal/helm-portals.conf`,
then `.../portals.conf`, then `/etc/xdg-desktop-portal/helm-portals.conf`, then
`/usr/share/xdg-desktop-portal/helm-portals.conf`. The `helm` in those names is
`XDG_CURRENT_DESKTOP`, split on `:`. That is the whole of what
`XDG_CURRENT_DESKTOP=helm` implies for portals — and it is why the variable must
be set before the first activation, not merely before helm's own code runs.

**helm ships `configs/portal/helm-portals.conf`**, installed to
`/usr/share/xdg-desktop-portal/helm-portals.conf`, naming a concrete backend per
interface rather than letting behaviour depend on what happens to be installed:

```ini
[preferred]
default=gtk
org.freedesktop.impl.portal.ScreenCast=wlr
org.freedesktop.impl.portal.Screenshot=wlr
```

The values are `.portal` file basenames (`gtk.portal` → `gtk`). The GTK backend
implements `FileChooser` and `Settings` but **not** `ScreenCast` on
wlroots-based compositors, so screen sharing must be routed to a wlroots
backend explicitly. This file does not exist yet; today only the Nix module
expresses the equivalent, which means the deb and the rpm have no portal policy
at all.

**Package dependencies.** Every package depends on `xdg-desktop-portal` **and**
on a named backend set — not on a disjunction that a solver may satisfy with the
wrong one. "No portal backend installed" presents to the user as "Open File does
nothing in Firefox", with no error anywhere.

**`xdg-desktop-portal-wlr` requires the compositor to serve
`wlr-screencopy-unstable-v1`** and, for output selection, a chooser
(`slurp` for the default `simple` chooser). *Whether river 0.4.8 still exports
`wlr-screencopy-unstable-v1`, and whether xdpw functions when window management
lives outside the compositor, is unverified.* This is **OQ-2** and it is the
reason A15 is a hardware row.

**Verification, not assumption.** A portal that answers on D-Bus is not proof
that it works.

- Cheap check, safe to run any time — the interface resolves without waiting:
  ```sh
  busctl --user get-property org.freedesktop.portal.Desktop \
      /org/freedesktop/portal/desktop \
      org.freedesktop.portal.FileChooser version
  ```
  It must answer immediately. A pause of about twenty-five seconds *is* the
  D-Bus activation timeout and *is* the diagnosis.
- Round trip, on demand and in the VM test — `helm ctl doctor --portal-roundtrip`:
  issue `org.freedesktop.portal.FileChooser.OpenFile` (signature `ssa{sv}` →
  object path), assert a handle within two seconds, then close it via
  `org.freedesktop.portal.Request.Close`. This opens a real dialog, which is why
  it is not the default. *The exact `busctl call` spelling is derived from the
  interface signature and should be confirmed once in the VM test rather than
  trusted from here.*
- ScreenCast: `doctor` asserts the `org.freedesktop.portal.ScreenCast` interface
  is present and that the configured impl is one that implements it. Whether a
  frame actually arrives is a hardware test with a human at the keyboard.

### 6. Non-systemd and non-D-Bus paths

The rule is absolute: **degrade with a named log line, never a silent hang.**
Every degradation emits exactly one line of the form

```
<timestamp> helm-session: DEGRADED <CODE>: <one sentence naming what the user will lose>
```

with a stable `CODE`, so the line can be grepped, documented and referenced by
`doctor` and by `docs/INSTALL.md`. Fatal conditions use `FATAL <CODE>` and exit.

| Code | Condition | Behaviour |
|---|---|---|
| `NO-RUNTIME-DIR` | `XDG_RUNTIME_DIR` unset or not a directory | **Fatal.** Nothing works without it. |
| `NO-COMPOSITOR` | river not on `PATH` | **Fatal.** |
| `NO-SOCKET` | no new Wayland socket within the deadline | **Fatal.** |
| `WM-ABORT` | `helm-daemon.service` failed, or was condition-skipped | **Fatal**, per §2 requirement 2. |
| `NO-SESSION-BUS` | no `DBUS_SESSION_BUS_ADDRESS` and no `$XDG_RUNTIME_DIR/bus` | Re-exec once under `dbus-run-session -- helm-session`, guarded by `HELM_DBUS_REEXEC=1` so it cannot loop. If that binary is absent, continue and state plainly that portals, file dialogs and screen sharing will not work in this session. |
| `NO-DBUS-ACTIVATION` | `dbus-update-activation-environment` absent | Continue. State that D-Bus-activated services will not see the display, so file dialogs will hang for twenty-five seconds. Suggest `dbus-user-session` (Debian/Ubuntu) or the equivalent dbus package (Fedora, Nix). |
| `NO-SYSTEMD-USER` | `systemctl --user show-environment` does not succeed | Continue on the direct-launch path below. |
| `NO-DISPLAY-PROBE` | `helm ctl wait-display` not installed | Continue with file-existence detection only. |
| `NO-XWAYLAND` | no new X11 socket appeared | Continue; `DISPLAY` is absent from both imports. |
| `NO-GSETTINGS` | `gsettings` or the schemas absent | Continue; GTK apps get the wrong cursor. |
| `NO-CURSOR-THEME` | the named theme resolves to no directory under any icon path | Continue; the cursor will be the default arrow. |

**Detection of a usable systemd user manager is behavioural, not
path-based.** `command -v systemctl` is not evidence: containers, elogind
systems and `su -` sessions all have the binary and no reachable manager. The
test is `systemctl --user show-environment` succeeding.

**The direct-launch path** (`NO-SYSTEMD-USER`), used on elogind-based systems
and in containers:

1. `dbus-update-activation-environment` is called **without** `--systemd`.
2. `helm-sessiond` and `helm-bar` are started directly, each under a bounded
   shell respawn loop with the same policy as the units: restart on non-zero
   exit, at most five times in thirty seconds, never on exit codes 69 or 78.
3. Exceeding the window manager's limit is `FATAL WM-ABORT`, exactly as under
   systemd. Exceeding the bar's limit logs and continues.
4. `doctor` reports overall status `degraded`, not `ok`, and names which of the
   two channels is missing. A degraded session that says so is supportable; one
   that pretends to be healthy is not.

Both degraded paths are exercisable in a plain container, because neither needs
a display: with a stub compositor that creates a socket file, the entry's
ordering, discovery, logging and fallback logic all run. That is what makes A1,
A2 and A4 CI rows rather than VM rows.

### 7. Session teardown

Triggered by the compositor exiting, or by `EXIT INT TERM HUP` on the entry, or
by `helm-session --abort` (§2). The whole sequence has a hard deadline of 15
seconds, after which the entry proceeds regardless — teardown may never be the
thing that hangs a logout.

1. **Stop the target first.** `systemctl --user stop helm-session.target`.
   systemd stops `WantedBy` units in reverse dependency order, so the bar (which
   is `After=helm-daemon.service`) stops before the window manager. That order
   matters: a bar still drawing against a departed window manager is a burst of
   errors in the journal at exactly the moment the user is trying to read why
   their session ended.
2. **A client that refuses to exit** is in its unit's cgroup and is killed:
   `SIGTERM`, then `SIGKILL` after `TimeoutStopSec` (5 s for the bar, 10 s for
   the window manager). No client can extend the teardown beyond those bounds.
   Applications the *user* launched are not part of `helm-session.target` — they
   live in `app.slice` scopes under the user manager — and helm does not kill
   them. Whether they survive logout is `logind`'s `KillUserProcesses` policy,
   which is a system decision and not helm's to override.
3. **Then, and only then, clear the environment.** Order matters here too: a
   unit restarting during teardown must not come up after the display is gone
   but before it is stopped.
   ```sh
   systemctl --user unset-environment \
       WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE \
       XDG_SESSION_DESKTOP XCURSOR_THEME XCURSOR_SIZE
   ```
   `XDG_RUNTIME_DIR` is **not** unset: it was never ours, and the user manager
   needs it.
4. **D-Bus has no unset.** Assigning empty values is the only available
   mechanism:
   ```sh
   dbus-update-activation-environment --systemd \
       WAYLAND_DISPLAY= DISPLAY= XDG_CURRENT_DESKTOP= XDG_SESSION_TYPE= \
       XDG_SESSION_DESKTOP=
   ```
   An empty value is not the same as an absent one, and here the difference
   favours us: libwayland falls back to `wayland-0` when `WAYLAND_DISPLAY` is
   absent, which on a multi-seat box or a fast relogin can connect a service to
   *someone else's live compositor*. An empty value fails immediately and
   visibly instead. This is written down because it looks like sloppiness and is
   not.
5. **Kill the compositor** if it is still alive, then reap the pid file and the
   runtime directory `$XDG_RUNTIME_DIR/helm/`.

Why any of this is needed at all: the systemd user manager and the session bus
routinely **outlive the session** — always for a lingering user, and often for a
plain relogin. Without step 3 the next login inherits a `WAYLAND_DISPLAY`
naming a dead socket, and the symptoms are identical to never having imported it
(failure mode **N3**).

### 8. What `helm ctl doctor` must check (input to SPEC 0006)

Every step above has a check. The names below are the contract; SPEC 0006 owns
the implementation. Each check must print **the symptom it prevents**, not a
pass mark, and `doctor` must exit non-zero on any failure so it can be a CI
gate (ADR 0011's guard).

| Check id | Proves | Symptom prevented | Where testable |
|---|---|---|---|
| `env/identity` | `XDG_CURRENT_DESKTOP=helm`, `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP=helm` in the process | Portal picks the wrong backend | VM |
| `env/wayland-display/process` | Present in `doctor`'s own environment | — | VM |
| `env/wayland-display/systemd` | Present in `systemctl --user show-environment` | Nothing themed after login; units come up displayless | VM |
| `env/wayland-display/dbus` | Present in the bus activation environment | File dialogs hang ~25 s | VM |
| `env/desktop/systemd`, `env/desktop/dbus` | `XDG_CURRENT_DESKTOP=helm` in both | Screen share offers no sources | VM |
| `env/agree` | All three views hold the *same* values | The import ran too early or was skipped | VM |
| `env/list-matches-entry` | `doctor`'s variable list equals the session entry's | A variable added in one place and forgotten in the other | **CI** |
| `env/cursor` | `XCURSOR_THEME`/`SIZE` in all three, theme resolves on disk, gsettings agrees | Black X11 arrow; cursor resizes across windows | VM |
| `env/xwayland` | `DISPLAY` in all three when XWayland is up; integer-scale policy in force | X11 apps absent or blurred | VM |
| `units/target` | `helm-session.target` active, and its `.wants` symlinks exist | A target that starts nothing and reports success | VM |
| `units/wm` | `helm-daemon.service` `ActiveState=active`; `ConditionResult` reported separately | **N1** — a condition-skipped unit read as success | VM |
| `units/bar` | `helm-bar.service` active or cleanly restarting | Bar gone unnoticed | VM |
| `units/restart-policy` | The shipped units carry the policy in §4 | A crashed bar taking the session down | **CI** |
| `units/idle-lock` | An idle and a lock unit are part of `graphical-session.target` | Lid closes, session stays unlocked | VM *(blocked on OQ-1)* |
| `wm/attached` | helm holds river's window-management global; reports the holder if not | **N2** — inert compositor, or a restart loop against a stale holder | VM |
| `wm/layer-shell` | helm is serving `river-layer-shell-v1` | The bar never appears, and it looks like the bar's fault | VM |
| `wm/capabilities` | `Capabilities`, including `unsupported` ([INTERFACES.md §1](../INTERFACES.md)) | A backend gap that looks like a bug | VM |
| `wm/protocol-version` | Bound interface versions match the pinned river | Session fails after a routine upgrade | VM |
| `portal/answers` | `org.freedesktop.portal.Desktop` responds without a pause | The 25 s hang | VM |
| `portal/config` | A `helm-portals.conf` is found and names a backend per interface | Behaviour that changes with what is installed | **CI** (file) / VM (effect) |
| `portal/filechooser` | `--portal-roundtrip`: a handle within 2 s | "Open File does nothing" | VM |
| `portal/screencast` | The interface exists and the configured impl implements it | Screen share silently produces nothing | VM; the real capture is **HARDWARE** |
| `session/socket` | `$XDG_RUNTIME_DIR/helm/ctl.sock` answers `Hello` | — | VM |
| `session/protocol-version` | Matches `helm_core::ipc::PROTOCOL_VERSION` | Bar and session disagree | **CI** |
| `session/degraded` | Reports each `DEGRADED` code in force this session | A degraded session pretending to be healthy | **CI** (degraded paths) |
| `fonts/glyphs` | `helm_core::glyphs::Probe::summary()` | Tofu in the bar | **CI** |
| `tools/floors` | Reused tools present at their version floors (ADR 0007) | charon or horus missing | **CI** |

Two constraints on `doctor` that come from this spec rather than from SPEC 0006:

1. **It must not shell out to tools that may be absent.** `xlsclients`,
   `xdpyinfo` and `wayland-info` are not guaranteed on any of the three targets.
   `doctor` opens the sockets and makes the bus calls itself.
2. **It must run outside a session** and report "no helm session running"
   rather than failing confusingly. Half of its value is being run over SSH by
   someone whose desktop will not start.

## Acceptance criteria

Each row is one happy path and becomes one test. **Where** says whether the test
can run in ordinary CI (a container, no display), only in the NixOS VM
(ADR 0010), or only on real hardware with a human present. The **HARDWARE** rows
carry `needs-human` under standing order S3 and must not be assumed to pass.

| # | Given / When / Then | Where | Test |
|---|---|---|---|
| A1 | Given a stub compositor that creates `$XDG_RUNTIME_DIR/wayland-9` after 3 s and a pre-existing `wayland-0`, when the entry runs, then it discovers `wayland-9` (not `wayland-0`, not a guess), proceeds only after discovery, and completes within the deadline | CI | |
| A2 | Given the session entry source, when the ordering test runs, then the identity exports precede the compositor start, and the two imports precede every client start, every `gsettings` call and every other D-Bus touch | CI | |
| A3 | Given the entry's variable list, the units' `ConditionEnvironment=` set and `doctor`'s list, when the consistency test runs, then all three name the same variables | CI | |
| A4 | Given a container with no reachable `systemd --user` and no `dbus-update-activation-environment`, when the entry runs against a stub compositor, then it logs exactly one `DEGRADED NO-SYSTEMD-USER` line and one `DEGRADED NO-DBUS-ACTIVATION` line, starts the clients directly under the bounded respawn loop, and does not hang | CI | |
| A5 | Given a booted session, when `systemctl --user show-environment` and the bus activation environment are read, then every imported variable is present in both with values equal to the compositor's `/proc/<pid>/environ` | VM | |
| A6 | Given a session started with the D-Bus import deliberately suppressed, when `doctor` runs, then `env/wayland-display/dbus` fails, names the twenty-five second hang, and `doctor` exits non-zero | VM | |
| A7 | Given a booted session, when the cursor is checked, then the environment, the imported environment and `gsettings` all name the same theme and size, and the theme resolves to a directory on disk | VM | |
| A8 | Given river started, when `helm-sessiond` attaches, then `doctor` reports `wm/attached` and `wm/layer-shell` served, and the measured unmanaged interval is inside the cold-start budget | VM | |
| A9 | Given `helm-sessiond` removed from the image, when the session starts, then the entry logs `FATAL WM-ABORT` with the "no window can be placed" message, tears river down, and exits non-zero — and the same happens when the unit is condition-skipped rather than failed | VM | |
| A10 | Given a running session with three windows, when `helm-sessiond` is killed, then it is restarted within `RestartSec`, the ledger is recovered from the snapshot, the three windows return to their projected rectangles, and `helm-bar.service` never leaves `active` | VM | |
| A11 | Given a stale window manager already holding river's window-management global, when `helm-daemon.service` starts, then it exits 69, is not restarted, and `doctor` reports `wm/attached` as failed naming the holding process | VM | |
| A12 | Given a running session, when `helm-bar` is killed, then it is restarted, and `helm-session.target` and `helm-daemon.service` both stay `active` throughout | VM | |
| A13 | Given a booted session, when `doctor --portal-roundtrip` issues a `FileChooser.OpenFile`, then a request handle is returned within 2 s, and `portal/config` confirms the running portal chose the backends named in `helm-portals.conf` | VM | |
| A14 | Given a session that is ending, when teardown runs, then the target is stopped before the environment is cleared, a client that ignores `SIGTERM` is killed after its `TimeoutStopSec`, the whole teardown completes within 15 s, and a second login gets a fresh `WAYLAND_DISPLAY` rather than the previous session's | VM | |
| A15 | Given a browser on a real machine, when the user starts a screen share, then a source list appears and the captured stream shows the desktop | **HARDWARE** | |
| A16 | Given a real laptop, when the lid is closed, then the session locks within the configured delay and the screen is blank on reopen until authentication | **HARDWARE** *(blocked on OQ-1)* | |

**Split: 16 criteria — 4 CI, 10 VM, 2 HARDWARE.**

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md); no new numbers are invented here.

| Path | Budget | How it is measured |
|---|---|---|
| Cold session start → usable | **< 900 ms** (§4) | From the entry's first line to `helm-sessiond` sending `READY=1`. river's own start-up dominates and is measured separately so a river regression is attributable. |
| The unmanaged window (§2) | **< 300 ms**, hard ceiling the 900 ms above | From the Wayland socket becoming live to the window-management global being bound. `doctor` reports the measured value. |
| Environment publication (step 4) | **< 50 ms** | Two D-Bus calls. If this is ever slow, the bus is the problem, not helm. |
| Portal `FileChooser` round trip | **< 2 s** | ADR 0011's guard. The failure it exists to catch is 25 s. |
| Teardown, total | **< 15 s** | Hard deadline; the entry proceeds regardless afterwards. |
| Wayland socket wait | **10 s deadline**, 50 ms poll | A *timeout*, not a budget. |
| Ledger snapshot write | **off the input path entirely** | ADR 0013: under river a stall is a session failure, not a slow frame. |

**No fixed sleep is a synchronisation primitive anywhere in this component.**
The only permitted `sleep` is the poll interval inside a loop that has both a
condition and a deadline.

## Failure modes

Rows this component is responsible for not causing, from
[PITFALLS.md](../PITFALLS.md):

| Pitfall row | Section | Guard |
|---|---|---|
| `WAYLAND_DISPLAY` never reaches D-Bus | Session integration | Steps 3–4; `env/wayland-display/dbus`; A5, A6 |
| `XDG_CURRENT_DESKTOP` unset | Session integration | Step 1; `env/desktop/*`; A5 |
| No portal backend installed | Session integration | §5 named dependencies and `helm-portals.conf`; `portal/config`; A13 |
| Session dies with a client | Session integration | §4 `PartOf`/`Wants`, never `BindsTo`/`Requires`; A12 |
| No lock/idle handling | Session integration | §4 `helm-idle.service`; `units/idle-lock`; A16 — **blocked on OQ-1** |
| XWayland apps unstyled or scaled wrong | Session integration | §3 XWayland; `env/xwayland` |
| Cursor theme unset | Session integration | Steps 1 and 5; `env/cursor`; A7 |
| `helm-session` dies | river | §2 supervision policy and ledger recovery; A10 |
| Layer-shell not served | river | `wm/layer-shell`; A8 |
| Protocol version drift after a river bump | river | Exit 78 + `RestartPreventExitStatus`; `wm/protocol-version` |
| Works on the author's distro only | Packaging | Three distro jobs plus the VM test; §4's shipped-symlink requirement |

**Proposed new rows.** Each was found while writing this spec and is not in the
register yet. They are recorded here as findings for a human to add.

- **N1 — A unit is skipped, not failed.** *(Session integration.)* When
  `ConditionEnvironment=WAYLAND_DISPLAY` is not met, the unit does not fail: it
  is `inactive (dead)` with `ConditionResult=no`, and `systemctl --user start
  helm-session.target` still exits 0. The user gets an inert desktop with
  nothing in `systemctl --user --failed`, which is the hardest possible thing to
  diagnose. *helm's answer:* the entry verifies `ActiveState=active` per unit
  after starting the target, and `doctor` reports `ConditionResult` separately
  from `ActiveState`. *Guard:* `units/wm`; A9.
- **N2 — A stale window manager holds river's global.** *(river.)* Only one
  window-management client may connect; river answers `unavailable` to a second.
  A leftover `helm-sessiond`, or one started by hand for testing, makes the
  supervised one permanently unstartable, and a naive restart policy turns that
  into an endless loop that buries the one message explaining it. *helm's
  answer:* a distinct exit code (69) plus `RestartPreventExitStatus=`, and
  `doctor` names the holding process. *Guard:* A11.
- **N3 — The user manager outlives the session.** *(Session integration.)* With
  lingering enabled, or on a quick relogin, `systemd --user` and the session bus
  persist across logout, so the next login inherits the previous session's
  `WAYLAND_DISPLAY` — pointing at a socket that no longer exists — unless
  teardown clears it. The symptoms are identical to never having imported it,
  which sends the diagnosis to the wrong place entirely. *helm's answer:*
  teardown clears both environments, in that order, after stopping the target;
  and the entry treats an inherited `WAYLAND_DISPLAY` naming a non-existent
  socket as stale rather than as a value. *Guard:* A14.

## Open questions

- **OQ-1 — the lock screen and the idle defaults. `needs-human`, and this one
  matters most.** Carried forward from ADR 0011, sharpened by ADR 0013.

  *Locker.* river 0.4 implements `ext-session-lock-v1` and reports
  `session_locked`/`session_unlocked` to the window manager, so helm can disable
  every non-lock binding while locked. That makes an `ext-session-lock-v1`
  client a hard requirement rather than a preference: a locker that draws a
  layer-shell overlay instead depends on helm serving `river-layer-shell-v1` and
  on helm granting it exclusive focus, so a crash in *helm* would expose the
  desktop. Options: **gtklock** (proper `ext-session-lock-v1`; GTK, which we
  already theme); **waylock** (same protocol, minimal, but Zig — a toolchain we
  otherwise removed from the workspace by ADR 0013); **swaylock**, only in
  versions that speak `ext-session-lock-v1`, older ones must be excluded;
  **`helm-ward`**, our own, which is the worst class of bug to get wrong and is
  not before M6. *Recommendation: gtklock for M3*, per ADR 0011, with waylock as
  the minimalist alternative if the Zig dependency is acceptable to packaging.

  *Idle.* Also unverified: whether river 0.4.8 implements `ext-idle-notify-v1`.
  If it does not, no idle daemon works at all and this is blocking rather than
  merely undecided.

  *Defaults.* Candidates: (a) no idle action by default; (b) blank at 5 min,
  lock at 10 min, lid-close always locks; (c) lock at 15 min, lid-close locks
  unless on external power. A proposal, not a decision: (b). **These are
  user-visible security defaults and must not be guessed.** A laptop that does
  not lock on lid-close is a security failure; a laptop that locks after 60
  seconds is one whose owner disables locking entirely, which is worse than
  either. A human decides, and A16 stays unproven until they do.

- **OQ-2 — ScreenCast under river 0.4.** Does river 0.4.8 still export
  `wlr-screencopy-unstable-v1`, and does `xdg-desktop-portal-wlr` work when
  window management lives outside the compositor? *Recommendation:* ship the
  `helm-portals.conf` routing above, mark screen sharing **unverified** in
  `docs/INSTALL.md` until A15 passes on hardware, and do not claim MVP
  capability 11 complete on the strength of the file chooser alone.

- **OQ-3 — `PATH` and the toolkit hints in the import set.** This spec keeps
  `PATH` out of the default import and imports no toolkit hints, on the grounds
  that the systemd user manager's environment outlives the session and a wrong
  value there is very hard to diagnose. The counter-argument is real: a
  D-Bus-activated Electron app will not see `MOZ_ENABLE_WAYLAND`.
  *Recommendation:* keep the narrow list; add `HELM_IMPORT_PATH=1` for Nix-style
  profiles; revisit if a concrete app is shown to need a hint at activation
  time. The session entry currently imports `PATH` unconditionally.

- **OQ-4 — the four-way name collision.** `helm-session` (script),
  `helm-session` (crate), `helm-sessiond` (binary), `helm-session.target`
  (target) and `helm-daemon.service` (the unit running `helm-sessiond`) are five
  names for three things. *Recommendation:* rename `helm-daemon.service` →
  `helm-sessiond.service` so the unit matches its binary, and leave the entry
  script's name alone because display managers and `helm.desktop` already point
  at it. Low stakes, certain to confuse every future reader if left.

- **OQ-5 — does river honour a pre-set `WAYLAND_DISPLAY`?** If river 0.4.8 uses
  an explicit socket name when one is given, rather than
  `wl_display_add_socket_auto`, the entry could set the name itself and the
  whole discovery race in step 3 disappears. *Recommendation:* verify against
  river 0.4.8 during M3; keep discovery regardless, because it is also the
  fallback and because the liveness probe is needed either way.

- **OQ-6 — `Type=notify` for `helm-daemon.service`.** Readiness should mean "the
  window-management global and the layer-shell manager are both bound", which is
  what makes `After=helm-daemon.service` meaningful for the bar and what makes
  the unmanaged-window budget measurable. *Recommendation:* adopt in M2 when the
  daemon exists; the unit is `Type=exec` until then, and the budget in §Budgets
  is unmeasurable until it changes.
