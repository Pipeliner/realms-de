# Integration surface — session and desktop services

> **Status: research, not a decision.** This file inventories the OS-level
> session and desktop services a desktop environment is expected to provide or
> integrate with, assesses candidate implementations against helm's stated
> vision, and proposes a priority for each. Nothing here supersedes an ADR.
> Where a finding contradicts something already written down — and two do — the
> contradiction is called out in [§20](#20-corrections-to-things-already-written-down)
> rather than smuggled in.
>
> Scope: **session and desktop services**. Hardware- and system-facing services
> (network, audio graph, Bluetooth, power profiles, display configuration,
> input devices) belong to the sibling register. Where the boundary is blurred —
> media keys, idle inhibition, removable media — the item is kept here and the
> overlap is noted, because the failure is a desktop failure even when the
> daemon underneath is a system one.
>
> Method: every maintenance and capability claim below was checked against a
> primary source in August 2026 unless it is marked *assumed* or *unverified*.
> [§22](#22-verification-ledger) lists exactly which claims are which. A
> confidently wrong maintenance claim is worse than an honest gap.

---

## 0. The four constraints that decide most of this

Before the component list, four facts that do more work in the assessments
below than any individual project's merits.

### 0.1 What river already gives helm, verified against the source

helm is river's window manager ([ADR 0013](../adr/0013-river-window-management-backend.md)),
so a large part of this surface is *already provided by the compositor* and
helm's job is only to not break it. Reading
[`river/Server.zig`](https://codeberg.org/river/river/raw/branch/main/river/Server.zig),
[`river/LockManager.zig`](https://codeberg.org/river/river/raw/branch/main/river/LockManager.zig)
and [`river/IdleInhibitManager.zig`](https://codeberg.org/river/river/raw/branch/main/river/IdleInhibitManager.zig)
on `main` gives the following inventory. This is the single most useful table in
this document, because it converts about half the "must provide" questions into
"must not break".

| Protocol / global | Present in river 0.4 | What it unlocks for helm |
|---|---|---|
| `ext-session-lock-v1` (`SessionLockManagerV1`) | yes, in `LockManager.zig` | Any of the three lock screens, with crash-safe locking |
| `zwp_idle_inhibit_manager_v1` (`IdleInhibitManagerV1`) | yes, in `IdleInhibitManager.zig` | Firefox/mpv keeping the screen awake **without D-Bus** |
| `ext-idle-notify-v1` | referenced as `server.input_manager.idle_notifier` | `swayidle` and every idle daemon |
| `ext-data-control-v1` **and** `wlr-data-control-unstable-v1` | both created | Clipboard managers, on the new protocol and the old |
| `wl_data_device_manager`, `PrimarySelectionDeviceManagerV1` | both created | Copy/paste, middle-click paste, drag and drop |
| `wlr-screencopy-v1`, `ext-image-copy-capture-v1`, `ext-output-image-capture-source-v1`, `ext-foreign-toplevel-image-capture-source-v1`, `ExportDmabufManagerV1` | all created | `grim`, screen recorders, and **both** generations of screencast portal |
| `wlr-foreign-toplevel-management-v1`, `ext-foreign-toplevel-list-v1` | both created | Window pickers for screen share; taskbars we do not want |
| `xdg-activation-v1` | created | "Open link" raising the browser instead of doing nothing |
| `wlr-gamma-control`, `wlr-output-power-management`, `wlr-output-management` | created (via the output manager) | Night light, DPMS blanking, output config |
| `wp_security_context_v1` | created | Flatpak/sandbox identification for portals |
| `wlr-layer-shell-v1` | **only if helm serves `river-layer-shell-v1`** | The bar, the launcher, **and every notification daemon, lock screen and OSD on this page** |

The last row is the load-bearing one and it is easy to miss. Under river 0.4,
`wlr-layer-shell` is the window manager's responsibility, so until
`helm-session` implements the manager half of `river-layer-shell-v1` (scheduled
M2, [ADR 0013](../adr/0013-river-window-management-backend.md) §3), *no*
layer-shell client works — not `helm-bar`, and not `mako`, `fnott`, `swaylock`,
`waylock` or `wlogout` either. Every "reuse this" recommendation below is
implicitly gated on M2 landing that protocol.

### 0.2 `XDG_CURRENT_DESKTOP=helm` means nothing recognises helm

[ADR 0011](../adr/0011-session-integration-contract.md) step 1 sets
`XDG_CURRENT_DESKTOP=helm`. That is correct and necessary for portal backend
selection, and it has a consequence worth stating: helm is not in anybody's
`OnlyShowIn`/`NotShowIn` list, no upstream has a helm code path, and any tool
that branches on desktop identity will take its generic branch. This is mostly
good — generic branches are the honest ones — but it means autostart files with
`OnlyShowIn=GNOME;KDE;` silently do not run, and it means helm inherits nothing
for free.

### 0.3 The themability test is stricter than it looks

[ADR 0005](../adr/0005-palette-toml-single-source.md): anything user-visible must
be themeable **from `palette.toml` alone**, through a generated template. In
practice a candidate passes only if it takes colours as text in a config file it
re-reads or can be restarted against. It fails if its colours live in a compiled
theme, in an icon set, in a GTK theme it looks up by name, or in a `.qss` we
would have to hand-maintain. Two whole categories on this page — tray icons and
polkit dialogs — fail this test structurally, not incidentally.

### 0.4 The budgets forbid some perfectly normal desktop behaviour

[ARCHITECTURE §4](../ARCHITECTURE.md#4-what-robust-and-snappy-mean-here) sets
"cold session start → usable < 900 ms" with the explicit note **no icon-cache
scan, no thumbnailer**, and "bar idle CPU ~0%". [ADR 0009](../adr/0009-no-animation-budget.md)
forbids animation. That rules out, by construction: an icon-themed system tray,
a thumbnailing file dialog, a sliding notification, a fading OSD, and a
desktop-search indexer that walks `$HOME` at login. Where the ecosystem's best
answer is one of those, this document says so rather than pretending.

---

## 1. Notifications

**What it provides.** A daemon owning the well-known D-Bus name
`org.freedesktop.Notifications`, implementing the
[Desktop Notifications Specification](https://specifications.freedesktop.org/notification-spec/latest/):
`Notify`, `CloseNotification`, `GetCapabilities`, `GetServerInformation`, the
`NotificationClosed`/`ActionInvoked` signals, urgency levels, actions, hints,
and a replaceable-id mechanism.

**What the user loses without it.** Not "reduced functionality" — specific
things stop. A calendar reminder never appears. Signal, Element and Slack become
silent: messages arrive and nothing tells you. `notify-send` in a build script
returns an error instead of telling you the build finished. Firefox's download
completion, its "allow notifications?" grants and its site notifications all
vanish. `systemd` failure notifications from user units disappear. Worse than
silence: some clients block on the D-Bus call. With **no** daemon at all, the
call fails with `ServiceUnknown` after the bus tries and fails to activate one,
which most toolkits swallow — but the pattern the user experiences is "this
application is broken", filed against the application.

**Provide, integrate, or not break.** **The DE must provide one.** Nothing else
in the stack will. There is no fallback.

**Candidates.**

| Project | Language | Toolkit | Maintenance (Aug 2026) | Config |
|---|---|---|---|---|
| [`fnott`](https://codeberg.org/dnkl/fnott) | C | fcft + pixman + fontconfig, **no GTK** | 1.8.0, 16 Jul 2025; same author as `foot` | INI at `~/.config/fnott/fnott.ini`, colours as `RRGGBBAA` |
| [`mako`](https://github.com/emersion/mako) | C | pango + cairo + gdk-pixbuf (icons optional), no GTK | 1.11.0, active | `~/.config/mako/config`, colours `#RRGGBBAA`, criteria sections |
| [`dunst`](https://github.com/dunst-project/dunst) | C | Xlib-optional, native Wayland | v1.13.2, 18 Mar 2026 | `dunstrc`, INI |
| [`swaync`](https://github.com/ErikReider/SwayNotificationCenter) | Vala | **GTK4 + libadwaita** | active | GTK CSS + JSON |
| [`wired-notify`](https://github.com/Toqozz/wired-notify), [`nwg-notifications`](https://docs.rs/crate/nwg-notifications), [`rustyfications`](https://github.com/bzglve/rustyfications), [`Lucent`](https://github.com/CPT-Dawn/Lucent) | Rust | varies; Lucent and nwg-notifications are GTK4 + gtk4-layer-shell | small projects, single-maintainer | varies |

**Fit against helm's vision.**

- **`fnott` — 5/5.** It is the closest thing on this page to a component written
  for helm. Keyboard-driven by design (`fnottctl dismiss`/`pause` bound to
  chords, which is exactly the which-key model). No toolkit. It uses `fcft` and
  `fontconfig`, the same font path as `foot`, so
  [ADR 0012](../adr/0012-font-fallback-is-a-contract.md)'s fallback chain
  actually applies to it rather than being bypassed. `border-radius` can be set
  to 0 and `border-size` to 1, which is the design's whole visual grammar. Its
  `[low]`/`[normal]`/`[critical]` sections map one-for-one onto
  `palette.toml`'s `dim` / `violet` / `gold` semantics. Colours are `RRGGBBAA`
  hex — the alpha-beside-the-colour convention `palette.toml` already uses. No
  animation.
  The one cost: **not packaged in Fedora** as far as I could find (it *is* in
  Debian trixie at 1.7.1 and sid/forky at 1.8.0, and in nixpkgs). Fedora 44 is
  the sole pre-alpha Fedora packaging target; this is a dated package search,
  not evidence that a Helm Fedora session works.
- **`mako` — 4/5.** Same shape, wider reach: packaged in Fedora, Debian, Ubuntu
  and nixpkgs. Costs pango and cairo, and `gdk-pixbuf` if icons are on — turn
  icons off and the dependency is optional. Pango font descriptions are a second
  font-selection mechanism that the glyph probe does not cover. Config is at
  least as templatable as fnott's.
- **`dunst` — 3/5.** Works, well maintained, but carries an X11 heritage and a
  larger surface than helm needs, and its Wayland support is a port rather than
  its native model.
- **`swaync` — 1/5.** GTK4 and libadwaita, a notification *centre* panel that
  the design does not have, animations, icons. It is a good project and a bad
  fit.
- The Rust options are the Rust-first temptation and should be resisted for M3:
  every one is a single-maintainer project younger than the alternatives, and
  the two most complete ones (Lucent, nwg-notifications) are GTK4, so they
  neither win on maintenance nor on toolkit.

**Integration cost.** `config + generated template`. One `fnott.ini.tmpl` or
`mako/config.tmpl` in `configs/templates/`, one systemd user unit, one catalogue
entry, two which-key bindings. The supported generation path affects future
launches only; `fnottctl reload` / `makoctl reload` would require #22's future
generation-aware owned-process protocol and is not part of this estimate. Half
a day.

**Priority: MVP. Milestone: M3.** A desktop where Slack is silent for a week
fails the week test on the second day.

**Recommendation.** `fnott` if the Fedora packaging gap can be closed by
building it in helm's own RPM (it is a small meson C project); Fedora's native
River candidate removes the earlier compositor-vendoring comparison, so this
remains its own maintenance choice. Use `mako` otherwise. Put the choice behind
the template name so it is a one-file swap, per
[ADR 0007](../adr/0007-reuse-yazi-btop-starship.md)'s seam rule.

**Two things neither daemon gives us**, worth recording now: notifications never
appear in `helm-bar` (there is no notification indicator in
`Desktop v3.dc.html` — see §9), and there is no history/notification-centre.
Both are M4-or-later questions for `helm-bar`, not reasons to reject a daemon.

---

## 2. Screen lock and idle management

**What it provides.** Two separable things that are usually confused. *Locking*
is a client that takes the `ext-session-lock-v1` session-lock role, blanks every
output, and refuses to release until PAM authenticates. *Idle management* is a
daemon that watches `ext-idle-notify-v1` and runs commands at timeouts —
typically dim, then blank via `wlr-output-power-management`, then lock — plus
`before-sleep` and `lock` hooks wired to logind.

**What the user loses without it.** With no locker: the laptop lid closes in a
café and the screen is live when it opens. `loginctl lock-session` (which
`systemd` and remote-management tooling both call) does nothing. With no idle
daemon: the display never blanks, so the battery drains and the panel burns; and
`systemd-suspend` resumes to an unlocked desktop. This is not a missing feature,
it is a security defect, and
[PITFALLS](../PITFALLS.md) already lists it as one.

**Provide, integrate, or not break.** **Must provide**, and must ship it in the
session by default rather than documenting it —
[ADR 0011](../adr/0011-session-integration-contract.md) step 8 already says so.

**Candidates — lockers.**

| Project | Language | Protocol | Maintenance (Aug 2026) | Theming |
|---|---|---|---|---|
| [`waylock`](https://codeberg.org/ifreund/waylock) | Zig | `ext-session-lock-v1` | 1.7.0-dev; last commit 2026-05-21; **by river's own author** | `-init-color` / `-input-color` / `-fail-color` CLI flags |
| [`swaylock`](https://github.com/swaywm/swaylock) | C | `ext-session-lock-v1` since 1.7 | 1.8.6 latest release; low but non-zero activity | ~40 colour options in `~/.config/swaylock/config` |
| [`gtklock`](https://github.com/jovanlanik/gtklock) | C + **GTK3** | `ext-session-lock-v1` | v4.0.0; last commit 4 Feb 2026 | GTK CSS — the same `gtk.css` helm already generates |
| [`hyprlock`](https://github.com/hyprwm/hyprlock) | C++ | `ext-session-lock-v1` | active | own config; animations are its selling point |

**Candidates — idle.**

| Project | Language | Maintenance | Config |
|---|---|---|---|
| [`swayidle`](https://github.com/swaywm/swayidle) | C | maintained; speaks `ext-idle-notify` | CLI arguments or `~/.config/swayidle/config` — a list of `timeout N 'cmd'` clauses |
| [`hypridle`](https://github.com/hyprwm/hypridle) | C++ | active | Hyprland config syntax |
| [`wlsleephandler-rs`](https://github.com/fishman/wlsleephandler-rs) | Rust | small, single-maintainer | Lua |

**Fit.**

- **`waylock` — 5/5,** and this materially changes the open question in
  [ADR 0011](../adr/0011-session-integration-contract.md). The ADR rejected
  waylock on the grounds that it is "Zig, a toolchain we otherwise do not have
  (see ADR 0002)". That objection was written before
  [ADR 0013](../adr/0013-river-window-management-backend.md), but ADR 0015 now
  makes River sourcing target-specific and gives Fedora 44 a native candidate.
  A Zig toolchain cost therefore has to be evaluated per target rather than
  assumed universal. waylock is by Isaac Freund, the same
  maintainer as river, so it is the locker most likely to keep working against
  the compositor helm ships. It has three colours, settable on the command line,
  which is the cheapest possible template. It draws no widgets, has no font
  dependency, no animation, and the smallest attack surface of the four.
- **`swaylock` — 4/5.** Correct on the property that matters (its README states
  it "is compatible with any Wayland compositor which implements the
  `ext-session-lock-v1` Wayland protocol"), packaged everywhere, and richly
  themeable from a plain config file. Larger than waylock, and its release
  cadence has slowed.
- **`gtklock` — 3/5.** ADR 0011's current recommendation. It is a real
  `ext-session-lock-v1` client and it is themeable from the `gtk.css` helm
  already generates, which is a genuine advantage. Against it: GTK3 in the lock
  path is a lot of code between the user and their session, its release cadence
  is slow (v4.0.0, with translation commits since), and helm would be theming
  the lock screen through a stylesheet designed for application windows.
- **`hyprlock` — 1/5.** Animation is its differentiator. Rejected on
  [ADR 0009](../adr/0009-no-animation-budget.md).
- **`swayidle` — 4/5.** Boring, exactly the right size, and its config is a list
  of timeouts and commands, which is the most auditable form a security default
  can take. It is not Rust; it does not need to be. It composes with anything.

**Integration cost.** `config + generated template` for the locker (three
colours), `config` for the idle daemon, plus two systemd user units and a logind
`HandleLidSwitch` policy. Plus a `doctor` check that both units are active,
which ADR 0011 already schedules.

**Priority: MVP. Milestone: M3.** Already on the critical path and already
blocked on a `needs-human` decision.

**Recommendation.** **`waylock` + `swayidle`**, and update ADR 0011's option
list with the Zig finding above. This is a change of recommendation from
`gtklock`, argued in [§21](#21-needs-a-human).

---

## 3. Polkit authentication agent

**What it provides.** A client registering with `polkitd` as the session's
authentication agent, so that when a privileged action is requested by an
unprivileged process, someone can be asked for a password. Without a registered
agent, polkit denies every action requiring `auth_admin` or `auth_self`
immediately.

**What the user loses without it.** Concretely: plugging in a USB stick and
opening it in a file manager fails, because `udisks2` mounting a non-`fstab`
device is a polkit action. `pkexec anything` fails. GNOME Disks, GParted and
`blueman` refuse to start their privileged half. Changing the power profile via
`power-profiles-daemon` fails. Updating packages through any GUI updater fails.
NetworkManager's system-wide connections cannot be edited (per-user Wi-Fi still
works). The failure text is usually a bare "Not authorized", which reads as a
permissions bug in the application.

Mitigating factor specific to helm: the target user is keyboard-first and lives
in a terminal, where `sudo` covers most of this. The failure that survives that
mitigation is USB media, and it happens on day one for a lot of people.

**Provide, integrate, or not break.** **Must provide.** It is one process, and
there is no fallback.

**Candidates.**

| Project | Language | Toolkit | Maintenance | Config |
|---|---|---|---|---|
| [`soteria`](https://github.com/ImVaskel/soteria) | **Rust** (relm4) | **GTK4** | active; MSRV 1.85 — the same edition-2024 floor helm uses | `~/.config/soteria/config.toml` (helper paths only) |
| [`hyprpolkitagent`](https://github.com/hyprwm/hyprpolkitagent) | C++ | **Qt/QML**, latterly hyprtoolkit | active | minimal |
| [`lxqt-policykit`](https://github.com/lxqt/lxqt-policykit) | C++ | **Qt** | active, part of LXQt | LXQt theming |
| `polkit-gnome` | C | **GTK3** | effectively unmaintained upstream; still packaged | none |
| `mate-polkit` | C | **GTK3** | maintained as part of MATE | none |
| [`manthanabc/polkit-agent`](https://github.com/manthanabc/polkit-agent) | Rust | — | tiny, single-author, unproven | — |

**Fit.** This is the first genuinely bad row on the page. **Every maintained
polkit agent draws a modal dialog with a toolkit.** There is no toolkit-free,
layer-shell, keyboard-first, `palette.toml`-themeable polkit agent in 2026.

- **`soteria` — 3/5.** The least-bad. It is Rust, so it is at least in helm's
  language, and its MSRV matches helm's. But it is GTK4/libadwaita-shaped, which
  means helm can recolour it via the generated `gtk.css` and libadwaita named
  colours ([HANDOFF](../../design/HANDOFF.md) §1c: "colours yes, shapes no") and
  cannot make it square-cornered or make it look like anything in
  `Desktop v3.dc.html`. Keyboard usability is unstated in its documentation and
  was not verifiable without running it.
- **`hyprpolkitagent` / `lxqt-policykit` — 2/5.** Qt drags a *second* toolkit
  into a desktop that has committed to theming GTK properly and Qt only through
  `qt6ct` colours until M6 ([MVP.md](../MVP.md), deferred list). Using a Qt
  agent at M3 means the one privileged dialog a user sees is the one surface
  helm has explicitly deferred theming for.
- **`polkit-gnome` — 2/5.** Ubiquitous and dead. Its only argument is that it is
  in every distribution.

**Integration cost.** `config` — one systemd user unit, `Wants=` from the
session target — plus the GTK theme it inherits for free. The cost is not in the
wiring, it is in accepting the appearance.

**Priority: MVP. Milestone: M3.** Ruthlessly judged: a week in which you cannot
mount a USB stick is a failed week, and the fix costs one unit file.

**Recommendation.** **`soteria`**, on the "at least it is Rust and at least
`gtk.css` reaches it" argument, with the honest note that this is the worst
visual compromise in the M3 set. See the [uncomfortable list](#18-the-uncomfortable-list).

---

## 4. Clipboard, primary selection, and clipboard history

Three separate problems that get treated as one.

### 4a. Copy and paste at all

Provided by `wl_data_device_manager`, created by river (§0.1). Primary selection
(middle-click paste) is provided by `PrimarySelectionDeviceManagerV1`, also
created by river. **helm must not break either.** The only way helm *could*
break them is by mishandling keyboard focus during a data transfer, which is a
window-management concern. Cost: nothing. Verify in the M3 session test.

### 4b. Clipboard survival

The Wayland failure everyone hits: the clipboard is owned by the *source client*,
so copying a URL out of a terminal and closing the terminal leaves the clipboard
empty. There is no compositor-side clipboard.

**What the user loses.** Copy a path from `yazi`, quit `yazi`, paste into
Firefox: nothing. Copy from a build log, close the pane, paste: nothing. On a
keyboard-first tiling desktop where windows are opened and banished constantly,
this bites far harder than it does on GNOME.

**Candidates.** [`wl-clip-persist`](https://github.com/Linus789/wl-clip-persist)
(Rust; watches data-control and re-offers the selection after the source dies),
or any clipboard-history daemon, which incidentally solves it. Note that a
history daemon and `wl-clip-persist` are documented as composable.

**Priority: MVP. Milestone: M3.** `config` cost — one unit. If the history
manager below is adopted at M3, this is subsumed.

### 4c. Clipboard history

**What the user loses without it.** The last thing copied over the last thing
copied. Not fatal for a week — but it is the single most-requested desktop
feature after a tray.

**Candidates.**

| Project | Language | Protocol | Notes |
|---|---|---|---|
| [`wl-clipboard-rs`](https://github.com/YaLTeR/wl-clipboard-rs) | **Rust** | `ext-data-control` **or** `wlr-data-control` | Library *and* drop-in `wl-copy`/`wl-paste`/`wl-clip` binaries. By niri's author |
| [`clipvault`](https://github.com/rolv-apneseth/clipvault) | **Rust** | via `wl-paste --watch` | Daemon + picker; regex ignore-patterns and per-window filtering for password managers |
| [`cliphist`](https://github.com/sentriz/cliphist) | Go | via `wl-paste --watch` | The de-facto standard; text and images; optional config file |
| [`wl-clipboard`](https://github.com/bugaevc/wl-clipboard) | C | `wlr-data-control`; `ext-data-control` merged (issue #242 → PR #255) but I could **not** confirm it is in a tagged release (latest tag 2.3) | The reference implementation |

**Fit.** `wl-clipboard-rs` is the standout and the reason is architectural, not
tribal: it is the only candidate that speaks **`ext-data-control-v1`**, which is
the protocol that replaced `wlr-data-control` in wayland-protocols 1.39 and
which river creates *first*. Choosing the C `wl-clipboard` binds helm's
clipboard to a deprecated protocol whose replacement is already shipping.
`clipvault` sits on top and is also Rust, with sensitive-content filtering that
matters (a clipboard history that silently records your password manager's
output is a security bug).

**The picker is the interesting part.** Every history manager in this space
delegates recall to `dmenu`/`rofi`/`fuzzel`. helm already has a fuzzy picker in
the MVP (`fuzzel` as hecate, [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md))
and a native one at M4 (`helm-hecate` on `nucleo`). So the correct shape is:
reuse the *store*, build the *picker* into hecate as a source alongside PATH,
desktop entries and spells. That costs nothing extra at M4 and gives one
consistent, themed, keyboard-driven surface instead of two.

**Integration cost.** `config` at M3 (persistence only); `shim` at M4 (a hecate
source that shells out to `clipvault list`/`decode`).

**Priority: STRATEGIC. Milestone: M4** for history; **MVP/M3** for 4b
persistence.

---

## 5. Screenshot and screen recording

**What it provides.** Capture of an output, a region or a window, to a file or
the clipboard; and continuous capture to a video file.

**What the user loses without it.** No way to put a screenshot in a bug report
or a chat message without installing something. `Print` does nothing. For a
developer's work week this is a daily action, not an occasional one.

**Provide, integrate, or not break.** **Integrate.** river provides the capture
protocols (both generations, §0.1); helm provides keybindings, a save path, and
a themed region selector.

**Candidates.**

| Project | Language | Requires | Notes |
|---|---|---|---|
| [`grim`](https://sr.ht/~emersion/grim/) | C | `wlr-screencopy-v1` | The standard. Packaged in Debian, Ubuntu, Fedora, nixpkgs |
| [`slurp`](https://github.com/emersion/slurp) | C | layer-shell | Region select. Colours and border thickness are **CLI flags** — `-b`, `-c`, `-s`, `-w` |
| [`wayshot`](https://github.com/waycrate/wayshot) | **Rust** | screencopy | grim-alike; `libwayshot` is the capture backend used by portal-luminous |
| [`wf-recorder`](https://github.com/ammen99/wf-recorder) | C++ | screencopy | The standard recorder |
| [`wl-screenrec`](https://github.com/russelltg/wl-screenrec) | **Rust** | screencopy + VA-API | Much lower CPU where VA-API exists |
| [`satty`](https://github.com/gabm/Satty), [`swappy`](https://github.com/jtheoof/swappy) | Rust / C+GTK | — | Annotation. Satty is Rust but GTK4 |

**Fit.** `grim` + `slurp` is a 5/5 for helm and it is slightly surprising:
`slurp`'s selection rectangle is configured entirely by command-line colour
arguments, which means the region selector — the one *visible* part — can be
driven straight from `palette.toml` with no theme file at all. Both are
single-purpose, non-resident, zero-idle-CPU, no-toolkit tools invoked from a
keybinding. That is exactly the composition model helm is built on. `wayshot` is
the Rust alternative and is worth keeping in view, but `grim`'s packaging reach
across all three target distributions decides M3.

Recording: `wl-screenrec` is the Rust-first answer and is genuinely better on
battery where VA-API is present, but it is a narrower hardware path.
`wf-recorder` is the safe default. Neither is week-test critical.

**Integration cost.** `config` — three keybindings (`Print` = output,
`mod+Print` = region, `shift+Print` = region to clipboard), a `slurp` invocation
whose colours come from a tiny generated fragment, and an `xdg-user-dirs`
`$XDG_PICTURES_DIR` to save into. An afternoon.

**Priority: MVP (screenshot) / LATER (recording). Milestone: M3 / M6.**

---

## 6. xdg-desktop-portal and its backends

**What it provides.** The D-Bus interface layer through which sandboxed and
non-sandboxed applications ask the desktop for things they cannot do
themselves: `FileChooser`, `ScreenCast`, `Screenshot`, `Settings`, `Inhibit`,
`OpenURI`, `Secret`, `GlobalShortcuts`, `RemoteDesktop`, `Print`. The front-end
(`xdg-desktop-portal`) routes each interface to an *implementation* backend
selected by `XDG_CURRENT_DESKTOP` via `portals.conf`.

**What the user loses without it, per interface.**

| Interface | Concretely lost |
|---|---|
| `FileChooser` | **Firefox cannot upload a file.** Ctrl+O does nothing, or hangs for 25 s. Flatpak apps cannot open or save anything |
| `ScreenCast` | Screen sharing in Zoom, Teams-in-a-browser, Discord and OBS offers no sources and fails silently |
| `Screenshot` | Flatpak screenshot tools return nothing |
| `Settings` | **GTK4 and libadwaita applications render in light mode on helm's black desktop**, because `color-scheme` reads as `no-preference`. Electron apps ignore dark mode |
| `Inhibit` | See below — the failure is *worse* when a backend is present than when it is absent |
| `OpenURI` | Clicking a link inside a Flatpak app opens nothing |
| `GlobalShortcuts` | Push-to-talk in Discord/Element does not work |
| `RemoteDesktop` | "Give control" in a screen-share session is unavailable |

**Provide, integrate, or not break.** **Must provide** — a backend must be
installed, declared and reachable. Already MVP capability 11.

**Candidates.**

| Backend | Language | Interfaces | Maintenance (Aug 2026) |
|---|---|---|---|
| [`xdg-desktop-portal-wlr`](https://github.com/emersion/xdg-desktop-portal-wlr) | C | ScreenCast, Screenshot | Latest tag 0.8.4; **master active — commits dated 13 Aug 2026** (verified). Supports `ext-image-copy-capture-v1` |
| [`xdg-desktop-portal-gtk`](https://github.com/flatpak/xdg-desktop-portal-gtk) | C + GTK3 | FileChooser, AppChooser, Print, Settings, Inhibit, Notification, Account, Email, Lockdown, Wallpaper | maintained under flatpak/ |
| [`xdg-desktop-portal-luminous`](https://github.com/waycrate/xdg-desktop-portal-luminous) | **Rust** | ScreenCast, Screenshot (+ its own picker) | active; uses `libwayshot`; standalone, no grim dependency |
| [`xdg-desktop-portal-generic`](https://github.com/lamco-admin/xdg-desktop-portal-generic) | — | ScreenCast v6, RemoteDesktop input, Screenshot, clipboard | new; unproven |
| [`xdg-desktop-portal-termfilechooser`](https://github.com/GermainZ/xdg-desktop-portal-termfilechooser) (+ [boydaihungst](https://github.com/boydaihungst/xdg-desktop-portal-termfilechooser) and [hunkyburrito](https://github.com/hunkyburrito/xdg-desktop-portal-termfilechooser) forks) | C | FileChooser only | original plus two active forks; the boydaihungst fork is the one with a yazi wrapper |

**Fit, and three findings ADR 0011 does not yet contain.**

1. **`Inhibit` must be routed to `none`, not to a backend.** This is the
   sharpest concrete finding on this page. `xdg-desktop-portal-gtk`'s Inhibit
   provider tries `org.gnome.SessionManager`, falls back to
   `org.freedesktop.ScreenSaver`, and — per
   [xdg-desktop-portal-gtk#465](https://github.com/flatpak/xdg-desktop-portal-gtk/issues/465)
   — if both fail it *logs a warning, does nothing, and reports success*. Under
   helm neither service will exist. Firefox tries the D-Bus route before the
   Wayland one, so it will be told "inhibited", stop trying, and **the screen
   will blank in the middle of a video call**. The fix is one line in
   `configs/portal/helm-portals.conf`:
   `org.freedesktop.impl.portal.Inhibit=none`, which makes the portal refuse and
   Firefox fall through to `zwp_idle_inhibit_manager_v1`, which river provides
   (§0.1). This should be in the shipped config and checked by `doctor`.
2. **`Settings` is not optional and nobody has flagged it.** Without a
   `org.freedesktop.impl.portal.Settings` provider reporting
   `color-scheme = 1 (prefer-dark)`, every GTK4/libadwaita application — and
   every Flatpak — renders light-themed on a `#05060c` desktop. `gtk.css`
   does not fix this, because libadwaita's stylesheet selection happens before
   the user stylesheet applies. `xdg-desktop-portal-gtk` provides Settings by
   reading GSettings, so routing Settings to it *and* setting
   `gsettings set org.gnome.desktop.interface color-scheme prefer-dark` in the
   session entry closes it. That gsettings call belongs next to the cursor-theme
   call in [ADR 0011](../adr/0011-session-integration-contract.md) step 6.
   ([`darkman`](https://gitlab.com/WhyNotHugo/darkman) is a lighter Settings-only
   backend if the GTK dependency is ever dropped.)
3. **`xdg-desktop-portal-wlr` has no RemoteDesktop**, so "give control" in a
   screen share will never work under helm on the M3 backend set. Worth
   documenting as a known limit rather than discovering in a meeting.

The rest of the fit assessment is unremarkable: helm has no choice but GTK for
`FileChooser` at M3 (`termfilechooser` is already scheduled as the M3→M4 charon
stopgap in ADR 0007, and it is a fork-of-a-fork whose upstream story is
genuinely muddled), and `xdpw` for ScreenCast. `xdg-desktop-portal-luminous` is
the Rust alternative and is the one to revisit at M5 when `helm-compositor`
owns capture directly.

**Integration cost.** `config` — `helm-portals.conf` plus package dependencies
plus the two lines above. Already scoped in ADR 0011 step 5; this section adds
`Inhibit=none`, `Settings`, and the RemoteDesktop caveat.

**Priority: MVP. Milestone: M3.**

---

## 7. Secrets and keyring

**What it provides.** `org.freedesktop.secrets` — the Secret Service API — a
D-Bus service storing named secrets in encrypted collections, unlocked once per
session, ideally by PAM at login.

**What the user loses without it.** Chrome and Chromium fall back to their
`basic` plaintext store and print a warning; saved passwords sit unencrypted on
disk. VS Code cannot store its GitHub/settings-sync token and re-prompts every
launch. Element and other Electron apps using `keytar`/`safeStorage` re-prompt
or fail to persist a session. `git-credential-libsecret` stops working, so every
`git push` over HTTPS asks for a token. `nextcloud-client` and `evolution`
refuse to save accounts. NetworkManager is *not* affected — it keeps its own
secrets — so Wi-Fi still works, which makes the failure look narrower than it is.

**Provide, integrate, or not break.** **Must provide.** No fallback that is not
a security regression.

**Candidates.**

| Project | Language | Notes |
|---|---|---|
| [`oo7`](https://github.com/linux-credentials/oo7) — `oo7-daemon`, `oo7-portal`, `oo7-cli`, `oo7-pam` | **Rust** | Cross-desktop `org.freedesktop.secrets`. Native secret service for COSMIC. **GNOME OS switched to it by default**, per [This Week in GNOME #254, June 2026](https://thisweek.gnome.org/posts/2026/06/twig-254/) |
| `gnome-keyring` (`gnome-keyring-daemon --start --components=secrets`) | C | The incumbent. Packaged everywhere; PAM auto-unlock is a solved, documented problem |
| `kwalletmanager` + `kwallet-secretservice` | C++/Qt | Drags Qt and KDE Frameworks |
| [KeePassXC](https://keepassxc.org/) with Secret Service integration | C++/Qt | Excellent, but a user choice, not a DE default — it requires an already-open database |

**Fit.** `oo7` is what helm would build if helm built one: Rust, cross-desktop,
no session-manager assumptions, a PAM module, and a portal backend for
`org.freedesktop.portal.Secret` so Flatpaks work. It is also, at the time of
writing, a component whose GitHub releases page shows a sparse and old-looking
tag history while its actual adoption (GNOME OS default) says the opposite. That
mismatch is exactly the kind of thing that should not be resolved by guessing.

`gnome-keyring` fits helm's aesthetic badly in one specific way — its unlock
prompt is a GTK dialog helm cannot restyle beyond colours — and fits helm's
*risk posture* very well.

Neither is user-visible except at unlock, so the theming argument is weak on
both sides and the maintenance argument is the whole decision.

**Integration cost.** `config` either way: a systemd user unit plus a PAM line
(`pam_gnome_keyring.so` or `oo7-pam`) in the display-manager and login stacks,
which packaging must add on three distributions.

**Priority: MVP. Milestone: M3.** A week in which every `git push` asks for a
token is a failed week.

**Recommendation.** `gnome-keyring` at M3, `oo7` behind the same config seam,
revisit at M4. Flagged for a human in [§21](#21-needs-a-human) because this is a
credential-security decision of the same class as the lock screen, and the
repository should not pick one silently — the same reasoning ADR 0011 applied.

---

## 8. Autostart, `.desktop` entries, MIME associations, default applications

Four related pieces of freedesktop plumbing that are individually boring and
collectively decide whether the desktop feels wired up.

**What it provides.**
- **`.desktop` entries** — `$XDG_DATA_DIRS/applications/*.desktop`: the list of
  installed applications, their names, exec lines, `Terminal=true` handling,
  `DBusActivatable` support and `Actions`. This is hecate's index.
- **MIME associations** — `~/.config/mimeapps.list` plus the shared MIME
  database: which application opens a `.pdf`.
- **`xdg-open`** — the indirection every application uses to open a URL or file
  in "the right thing".
- **Autostart** — `$XDG_CONFIG_DIRS/autostart/*.desktop`, run at session start.

**What the user loses without each.** No desktop-entry indexing: hecate can
launch things on `PATH` but not Firefox-with-its-real-name, and cannot pass
`%U`. No MIME/`xdg-open`: **clicking a link in Slack or a terminal opens
nothing**; `yazi`'s `open` does nothing; a downloaded PDF has no handler. No
autostart: Nextcloud, Syncthing's tray, a corporate VPN agent and `keepassxc`
never start; users who installed something expecting it to run at login find it
silently absent.

**Provide, integrate, or not break.**
- Desktop entries and MIME: **integrate** — read them, do not reinvent them.
- `xdg-open`: **must ensure one works.** With `XDG_CURRENT_DESKTOP=helm`,
  `xdg-utils`' `xdg-open` falls to its generic path, which tries `gio open` then
  a list of browsers. It usually works. "Usually" is not a contract.
- Autostart: **must provide** a runner. Nothing runs `autostart/` by itself.

**Candidates.**

| Concern | Candidate | Language | Notes |
|---|---|---|---|
| `xdg-open` replacement | [`handlr`](https://github.com/chmln/handlr) / [`handlr-regex`](https://github.com/Anomalocaridid/handlr-regex) | **Rust** | Drop-in `xdg-open`/`xdg-mime`; content- and extension-based detection; wildcard `text/*`; prunes invalid `mimeapps.list` entries. `handlr-regex` is the maintained fork |
| `xdg-open` baseline | `xdg-utils` | shell | Ubiquitous; a large, fragile shell script |
| Autostart | `systemd-xdg-autostart-generator` + `xdg-desktop-autostart.target` | — | Already present wherever systemd is; opt-in by starting the target. Handles `OnlyShowIn`/`NotShowIn` correctly |
| Autostart | [`dex`](https://github.com/jceb/dex) | Python | Standalone; `dex -a -e helm` |
| Whole session | [`uwsm`](https://github.com/Vladimir-csp/uwsm) | Python | See below |
| Desktop-entry parsing (for hecate) | [`freedesktop-desktop-entry`](https://crates.io/crates/freedesktop-desktop-entry) | **Rust** | Used by COSMIC; the obvious dependency for `helm-hecate` |
| User directories | `xdg-user-dirs` | C | Creates and records `$XDG_DOWNLOAD_DIR` etc. Portals, browsers and yazi all assume these exist |

**A finding worth raising: `uwsm`.**
[ADR 0011](../adr/0011-session-integration-contract.md) writes the environment
handshake by hand in shell and says so with visible discomfort ("shell in the
critical path is unpleasant"). `uwsm` does exactly the job that ADR describes —
idempotent environment export into **both** the systemd user manager and the
D-Bus activation environment, XDG autostart into a slice that is stopped before
the compositor, bi-directional binding to `graphical-session.target`, and
variable cleanup on exit — and it is a maintained project with compositor
plugins. Two things argue against adopting it for M3: it is Python, in a session
critical path helm wants to be able to reason about completely; and its plugin
list covers sway, wayfire, labwc, hyprland, niri and mango but **not river**,
so helm would be writing the plugin anyway. The right use of this finding is not
"adopt uwsm" but "read uwsm's environment handling before finalising the session
entry script, because it has already found the edge cases".

**Fit.** `handlr-regex` is 4/5 — Rust, fast, single-binary, and it makes
`helm ctl` able to *set* defaults declaratively, which suits a config-file
desktop. Against it: replacing `xdg-open` system-wide is a slightly aggressive
move for M3, and if it misbehaves the symptom is "links do nothing", which is
the exact failure class ADR 0011 exists to prevent. `systemd-xdg-autostart-
generator` is 5/5 for autostart: it costs one `Wants=xdg-desktop-autostart.target`
line and no new dependency at all.

**Integration cost.** `config` throughout. Ship `configs/mimeapps.list` defaults
(browser, terminal, image, PDF), depend on `xdg-user-dirs` and call
`xdg-user-dirs-update` in the session entry, enable
`xdg-desktop-autostart.target`, and add a `doctor` check that `xdg-open
https://example.com` resolves to a real handler.

**Priority: MVP. Milestone: M3.** Cheap, and its absence produces the "this
isn't a desktop, it's a pile of programs" feeling
[MVP.md](../MVP.md) names as the thing to avoid.

---

## 9. Status notifier / system tray (SNI)

**What it provides.** `org.kde.StatusNotifierWatcher` plus a *host* that renders
each registered `StatusNotifierItem` — icon, tooltip, and a `com.canonical.dbusmenu`
context menu — and forwards activation.

**What the user loses without it.** Specific, and worse than people expect:
- **Slack, Discord, Element, Telegram and Signal**: "close to tray" means the
  window closes and the process keeps running with no way to get it back. The
  user thinks they quit; they did not.
- **Steam** minimises to a tray that does not exist.
- **Nextcloud and Dropbox clients** have no UI at all other than a tray icon —
  sync status and conflict resolution become unreachable.
- **KeePassXC** with "minimise to tray" behaves the same as Slack.
- `nm-applet`, `blueman-applet`, `udiskie --tray` and most battery/VPN applets
  are tray-only by construction.

**Provide, integrate, or not break.** If helm ships a tray, **helm must provide
the host** — it is a bar feature, not a separate process. If helm does not, this
is a documented limitation, not a bug.

**Candidates.**

| Project | Language | Notes |
|---|---|---|
| [`system-tray`](https://github.com/JakeStanger/system-tray) | **Rust** | Async SNI + DBusMenu client, built for bars; used by `ironbar`. Requires tokio |
| [`stray`](https://github.com/jgarvin/stray) | Rust | Older, smaller |
| `waybar`'s tray module | C++ | Would mean abandoning `helm-bar` |
| [`snixembed`](https://git.sr.ht/~steef/snixembed) | C++ | The *reverse* bridge (SNI → XEmbed); irrelevant here. An XEmbed → SNI bridge is what would be needed for legacy X11/Wine icons |

**Fit — and this is where helm's design and the ecosystem genuinely disagree.**

I extracted the text of `design/Desktop v3.dc.html` to check rather than
assuming. The bar's right-hand segment is, in order:
`↑ 18k ↓ 1.2M` · `cpu 31%` · `mem 9.8G` · `gpu 44°` · `♪ 64%` · `⚡ 87%` ·
`26·08·2026 ☾ 14:32`. **There is no tray region in the design, and no
notification indicator.** There is also no icon anywhere in the entire mockup —
[HANDOFF](../../design/HANDOFF.md) says "Assets: None — no images. All
iconography is Unicode glyphs."

An SNI host is an *icon* renderer. Items publish either an icon name (requiring
an icon theme lookup, and therefore an icon cache) or raw ARGB pixmaps over
D-Bus. Both contradict:
- the glyph-only design language,
- [ARCHITECTURE §4](../ARCHITECTURE.md#4-what-robust-and-snappy-mean-here)'s
  "cold session start < 900 ms, **no icon-cache scan**",
- [ADR 0008](../adr/0008-layer-shell-rendering-stack.md)'s renderer, which is
  `tiny-skia` + `cosmic-text` — a text-and-shapes renderer, not an icon-theme
  engine,
- and the DBusMenu context menu, which is a mouse-driven popup in a desktop
  where "mouse works but is never required".

So: **2/5 at best, and the score is a property of helm, not of the projects.**
`system-tray` is a good crate. It would be a good crate to build a thing helm
has decided not to have.

There is a middle path worth naming: a **text-only tray segment** — render each
registered item as its `Title`/`Id` in `text.mid`, with `mod+t` cycling items and
`Enter` sending `Activate`, and the DBusMenu rendered as a which-key-style list
rather than a popup menu. That is keyboard-first, glyph-free, themeable, and
recovers the actual lost capability (getting Slack back) without the icon
machinery. It is also real work in `helm-bar` and a visible departure from the
mockup, so it is not a decision this document can take.

**Integration cost.** `real work` — an SNI host, a DBusMenu client, a new bar
segment, and a design decision. Not less than a week, and it touches the one
crate with a hard frame budget.

**Priority: STRATEGIC. Milestone: M4.** Not MVP, and this is the most
uncomfortable MVP exclusion on the page. Justification for holding the line: the
week test is one person, and a person who knows their desktop has no tray
configures Slack not to close to tray and uses `helm ctl run` to bring windows
back. A person who does not know will lose an application on day one. That is a
documentation problem at M3 and a product problem by M4.

Flagged for a human in [§21](#21-needs-a-human).

---

## 10. Session management: logout, reboot, shutdown, suspend, lock

**What it provides.** A way to end the session and to reach the machine's power
states, plus the logind plumbing under it: `org.freedesktop.login1.Manager`
(`PowerOff`, `Reboot`, `Suspend`, `Hibernate`), `.Session.Lock`/`Terminate`,
inhibitor locks, and lid/power-key handling.

**What the user loses without it.** No way to log out, reboot or shut down
without switching to a TTY or running `loginctl` by hand — which, in a desktop
whose entire promise is that everything has a chord, is a conspicuous hole.
`loginctl lock-session` from another terminal does nothing (see §2). Closing the
lid does not suspend.

**Provide, integrate, or not break.** **Must provide** the user-facing part;
**integrate** with logind for the mechanism. logind itself is a system service
and is the sibling register's concern; what belongs here is the surface.

**Candidates.**

| Project | Language | Toolkit | Fit |
|---|---|---|---|
| [`wlogout`](https://github.com/ArtsyMacaw/wlogout) | C | **GTK3** | A grid of large SVG-icon buttons. 1/5 |
| [`wleave`](https://github.com/AMNatty/wleave) | Rust | **GTK4 + libadwaita** | Same shape, newer toolkit, wlogout-compatible config. 2/5 |
| `loginctl` / `systemctl` directly | — | none | 5/5 mechanically, 0/5 discoverably |
| **Build it into `helm-ctl` + which-key** | Rust | none | see below |

**Fit.** Every packaged logout menu is a full-screen grid of icon buttons — the
opposite of a keyboard-first desktop with a which-key strip. helm already has
the correct UI for this and it is not a new surface: a `mod+q`-style leader
opening a which-key row reading
`l logout · r reboot · p poweroff · s suspend · L lock`, with the actions being
`zbus` calls to `org.freedesktop.login1`. That is perhaps 150 lines in
`helm-ctl` plus a keymap table entry, and it is themed, glyph-based and
animation-free for free because the which-key strip already is.

**Integration cost.** `small Rust shim` — `helm ctl session {logout,reboot,
poweroff,suspend,lock}` over `zbus`, plus keymap entries.

**Priority: MVP. Milestone: M3.** See [§19](#19-what-helm-should-build-itself).

---

## 11. Drag and drop, and the data-control protocols

**What it provides.** `wl_data_device` DnD (with `wl_data_offer` actions and the
compositor-driven grab), the primary selection protocol, and the privileged
`ext-data-control-v1` / `wlr-data-control-unstable-v1` protocols that let a
non-focused client read and set selections.

**What the user loses without it.** Dragging a file from `yazi` onto a browser
upload target, or an image from Firefox into an editor, does nothing. Between
XWayland and Wayland clients, DnD stops at the boundary.

**Provide, integrate, or not break.** **Must not break.** river creates every
one of these globals (§0.1) and implements the DnD grab itself. helm's only
exposure is that, as window manager, it owns keyboard focus and window
stacking — a DnD grab that outlives a focus change, or a window raised
mid-drag, could plausibly disrupt a transfer.

**Candidates.** None; there is nothing to choose.

**Integration cost.** `config` — none, but a **test**: an M3 session test that
performs a cross-client DnD and a clipboard round-trip, including an XWayland
client. `wl-clipboard-rs` (§4) is the right tool for the clipboard half because
it speaks `ext-data-control-v1` directly.

**Priority: MVP (as a test). Milestone: M3.**

One genuine risk to record: `ext-data-control-v1` is a *privileged* protocol,
and river creates it unconditionally alongside `wp_security_context_v1`. Any
client can read the clipboard continuously. That is a property of the platform
helm ships, worth one sentence in the security documentation rather than a
change of behaviour.

---

## 12. Desktop search and application indexing

**What it provides.** Two very different things: application/command search (what
hecate does) and content search across the user's files (what Tracker, Recoll or
Spotlight do).

**What the user loses without it.** Application search: nothing — hecate is MVP
capability 5. Content search: there is no "search my documents" surface. The
user runs `rg` and `fd` in a terminal, which for helm's stated audience is
arguably the preferred outcome.

**Provide, integrate, or not break.** **Not break**, and deliberately do not
provide.

**Candidates.** `tracker-miners` (GNOME; a resident indexer, drags GNOME
infrastructure, and walks `$HOME` at login), `recoll` (Qt + Python + Xapian;
powerful and unlovely), `fsearch` (C + GTK3; filename-only), `plocate` (a system
`updatedb` cron; filename-only but essentially free), and the composition
helm actually wants: `fd` + `ripgrep` + `fzf`/`nucleo` behind a hecate source.

**Fit.** Every packaged desktop-search product is 1–2/5 for helm. Each is a
resident indexer with a GUI in a toolkit helm does not use, and a background
`$HOME` walk is precisely what the 900 ms cold-start budget and the ~0% idle
target rule out. Conversely `fd`/`ripgrep` are 5/5, already installed on any
developer machine, and compose into hecate at M4 as another source alongside
PATH, desktop entries, spells and clipboard history.

**Integration cost.** `config` now (document the composition); `shim` at M4/M6
(a hecate `find:` source).

**Priority: LATER. Milestone: M6**, and possibly never as a product. This is a
case where the honest answer is "helm does not have desktop search, it has
`fd`", and saying so is better than shipping something nobody would open twice.

---

## 13. Trash, and `gio`/`gvfs` file operations

**What it provides.** The
[freedesktop Trash specification](https://specifications.freedesktop.org/trash-spec/trashspec-latest.html):
`$XDG_DATA_HOME/Trash/{files,info}`, `.trashinfo` records with original path and
deletion time, per-volume `.Trash-$uid` directories, restore, and empty. Plus
the `gio`/`gvfs` layer that GTK applications use for trash, network mounts
(smb://, sftp://, mtp://), the "Recent" list and bookmark resolution.

**What the user loses without it.** Deleting from a GTK file dialog either fails
or deletes permanently with no undo. A phone plugged in over MTP does not appear
in any file chooser. `gio trash --list` errors. There is no way to recover a
file deleted five minutes ago — for a *first* week on a new desktop, that is a
trust problem out of proportion to its frequency.

**Provide, integrate, or not break.** **Integrate.** helm should not implement
the trash spec; it should ensure the tools it ships honour it and that recovery
is reachable.

**Candidates.**

| Concern | Candidate | Language | Notes |
|---|---|---|---|
| Trash from charon | **yazi, already** | Rust | `d` trashes, `D` deletes permanently, and it follows the freedesktop spec natively on Linux. No work needed |
| Restore / empty / list | [`trashy`](https://github.com/oberblastmeister/trashy) | **Rust** | `trash list/restore/empty`; faster and richer than trash-cli |
| Restore / empty / list | [`trash-cli`](https://github.com/andreafrancia/trash-cli) | Python | The reference implementation; what most yazi trash plugins wrap |
| Restore inside charon | [`restore.yazi`](https://github.com/boydaihungst/restore.yazi), [`recycle-bin.yazi`](https://github.com/uhs-robert/recycle-bin.yazi) | Lua plugins | `recycle-bin.yazi` wraps trash-cli and gives browse/restore/empty-by-age inside yazi |
| GTK apps' trash and mounts | `gvfs` | C | A runtime dependency, not a helm component. Without it `gio trash` is degraded and MTP/SMB do not appear |

**Fit.** 5/5 and mostly already done: charon *is* yazi and yazi already
implements the spec. What is missing is a *restore* path, which is a yazi plugin
plus a keymap entry, and a `gvfs` package dependency so GTK applications behave.
`trashy` is the Rust CLI to reach for when a plugin needs a backend.

**Integration cost.** `config` — a package dependency on `gvfs` and
`trash-cli` (or `trashy`), a yazi plugin, and two keymap lines.

**Priority: STRATEGIC. Milestone: M4.** Not MVP because the destructive path
(yazi's `d`) is already safe today; the missing half is recovery, which is
STRATEGIC rather than survival.

---

## 14. Media keys and MPRIS

Two separable concerns again.

### 14a. Media and volume keys

**What it provides.** Binding `XF86AudioRaiseVolume`, `XF86AudioLowerVolume`,
`XF86AudioMute`, `XF86AudioMicMute`, `XF86MonBrightnessUp/Down`, and
`XF86AudioPlay/Next/Prev` to actions, and giving the user feedback that
something happened.

**What the user loses without it.** The volume keys on the keyboard do nothing.
The brightness keys do nothing. On a laptop this is noticed within about ninety
seconds of first login and it reads as "this desktop is unfinished" more
strongly than almost anything else on this page.

**Provide, integrate, or not break.** **Must provide.** helm owns the keymap
under `river-xkb-bindings-v1` ([ADR 0013](../adr/0013-river-window-management-backend.md)),
so nothing else *can* bind these keys.

**Candidates for the action.** `wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%+`
(WirePlumber, ships with PipeWire), `pactl` (PulseAudio-compatible),
[`brightnessctl`](https://github.com/Hummer12007/brightnessctl) (C, tiny,
standard) or `light`.

**Candidates for the feedback.** [`swayosd`](https://github.com/ErikReider/SwayOSD)
(Rust but **GTK4**, animated), [`wob`](https://github.com/francma/wob) (C, a
layer-shell bar, no toolkit), [`avizo`](https://github.com/misterdanb/avizo)
(Vala + GTK) — **or the bar helm already designed**. The mockup's right-hand
segment already contains `♪ 64%`, so the feedback surface exists and is already
themed. An OSD would be a second layer-shell surface, a second theme target, and
a strong temptation towards a fade-out animation
([ADR 0009](../adr/0009-no-animation-budget.md)).

**Fit.** `wpctl`/`brightnessctl` are 5/5 — non-resident, no toolkit, one line
each. Every OSD daemon is 2/5 or worse for helm and none of them is needed.

**Integration cost.** `config` for the bindings; `shim` for the bar module to
subscribe to volume changes event-driven rather than polling — which
[ARCHITECTURE §4](../ARCHITECTURE.md#4-what-robust-and-snappy-mean-here)
requires anyway ("bar modules update event-driven; only the clock ticks").

**Priority: MVP. Milestone: M3** (bindings) / **M2–M3** (the bar module, which
is already in scope as `vol`).

### 14b. MPRIS

**What it provides.** `org.mpris.MediaPlayer2.*` — a standard D-Bus control and
metadata surface implemented by mpv, VLC, Spotify, Firefox, Chromium, mpd (via
`mpDris2`) and most players. Lets play/pause keys work across whichever player
happens to be running, and lets a bar show "now playing".

**What the user loses without it.** The play/pause key pauses nothing, or pauses
the wrong thing. There is no "what is playing" anywhere.

**Provide, integrate, or not break.** **Integrate**, and it is optional.

**Candidates.** [`playerctl`](https://github.com/altdesktop/playerctl) (C
library + CLI; the standard; `playerctl play-pause` is the whole integration) or
the [`mpris`](https://crates.io/crates/mpris) Rust crate if `helm-bar` ever
wants metadata natively.

**Fit.** `playerctl` is 5/5 as a *binding target* — a one-line invocation from a
keybinding, non-resident, no toolkit. A now-playing bar module is 3/5: the
design has no media module and the bar is already at seven right-hand segments;
adding one is a design change, not an integration.

**Integration cost.** `config` — three keybindings.

**Priority: STRATEGIC. Milestone: M4.** The keys are cheap enough that they
could ride along at M3 if `playerctl` is already a dependency.

---

## 15. Additional components not in the brief

These are gaps found while working through the list. Each is in the same class —
a session or desktop service users notice immediately.

| # | Component | What is lost without it | Verdict | Priority / Milestone |
|---|---|---|---|---|
| 15.1 | **Idle inhibition** | Video calls and full-screen video blank the screen mid-use. Two mechanisms: Wayland `zwp_idle_inhibit_manager_v1` (river provides) and D-Bus `org.freedesktop.ScreenSaver` (nothing provides). The **failure mode is a silent false success** — see §6 finding 1 | **Must not break**, via `Inhibit=none` in `helm-portals.conf` plus a `doctor` check | **MVP / M3** |
| 15.2 | **`org.freedesktop.portal.Settings` `color-scheme`** | Every GTK4/libadwaita and Flatpak app renders light on a black desktop. `gtk.css` cannot fix it | **Must provide** (route Settings to a backend **and** set the gsettings key) | **MVP / M3** |
| 15.3 | **`xdg-user-dirs`** | `$XDG_DOWNLOAD_DIR` and `$XDG_PICTURES_DIR` do not exist, so Firefox downloads to `$HOME`, screenshots have nowhere to go, and portal file dialogs open in the wrong place | **Integrate** — depend on it, call `xdg-user-dirs-update` in the session entry | **MVP / M3** |
| 15.4 | **`xdg-activation-v1` handling** | Clicking a link when Firefox is already open on another orbit does nothing visible; the window never raises and no urgency is signalled. river creates the global; the **window manager** must act on the token | **Must provide** (in `RiverBackend`) — overlaps the window-management register | **MVP / M2–M3** |
| 15.5 | **Removable media (udisks2 + a mount agent)** | Plugging in a USB stick or an SD card does nothing; no notification, no mount, nothing in charon. [`udiskie`](https://github.com/coldfix/udiskie) is the standard agent (Python; its tray mode is optional and should be off — use `--no-tray --notify`) | **Integrate** | **STRATEGIC / M4** |
| 15.6 | **Cursor theme** | Already in [ADR 0011](../adr/0011-session-integration-contract.md) step 6. Worth noting it is *also* a `palette.toml` surface: HANDOFF specifies a "mono-line icon + cursor set", which does not yet exist and cannot be generated from a colour file | **Must provide** — and the mono-line cursor set is an unfunded asset | **MVP (env) / M6 (the set)** |
| 15.7 | **Night light / colour temperature** | Nothing at sunset; the design's urania pane literally says "20:14 ☉ sunset · night palette engages", so the *design promises this*. river provides `wlr-gamma-control`; [`wlsunset`](https://sr.ht/~kennylevinsen/wlsunset/) (C) or [`gammastep`](https://gitlab.com/chinstrap/gammastep) are the tools | **Integrate** | **LATER / M6** |
| 15.8 | **`GlobalShortcuts` portal** | Push-to-talk in Discord/Element/Teams does not work; global media shortcuts from sandboxed apps do not register. No wlroots backend implements it | **Not break**; document as a limit | **LATER / M6** |
| 15.9 | **Notification sounds / sound theme** | Silent everything. `canberra`/`sound-theme-freedesktop` if wanted; arguably correct to omit | **Not provide** | **LATER** |
| 15.10 | **Accessibility (AT-SPI, `orca`)** | A screen reader sees nothing: `tiny-skia` + `cosmic-text` surfaces expose no accessibility tree at all. Already named in M6, but the honest statement is that helm is currently unusable with a screen reader | **Must eventually provide**; be honest now | **LATER / M6** |
| 15.11 | **Thumbnails** | Deliberately refused — HANDOFF: "No thumbnails, ever". Recorded so it is a decision, not an omission | **Will not provide** | — |
| 15.12 | **PipeWire + WirePlumber** | `ScreenCast` cannot work without PipeWire — it is how the portal delivers frames. Mostly the sibling register's, but the portal dependency belongs here | **Integrate** (hard dependency of §6) | **MVP / M3** |

---

## 16. Summary table

Cost key: `config` = a config file or a unit; `config+tmpl` = plus a template
generated from `palette.toml`; `shim` = a small amount of Rust; `work` = real
engineering.

| # | Component | Provide / integrate / not break | Recommended choice | Fit | Cost | Priority | Milestone |
|---|---|---|---|---|---|---|---|
| 1 | Notifications | **provide** | `fnott` (fallback `mako`) | 5 / 4 | config+tmpl | **MVP** | M3 |
| 2 | Screen lock | **provide** | **`waylock`** (was `gtklock`) | 5 | config+tmpl | **MVP** | M3 |
| 2 | Idle management | **provide** | `swayidle` | 4 | config | **MVP** | M3 |
| 3 | Polkit agent | **provide** | `soteria` | 3 | config | **MVP** | M3 |
| 4a | Copy/paste, primary | not break | river | — | none (test) | **MVP** | M3 |
| 4b | Clipboard survival | provide | `wl-clip-persist` | 4 | config | **MVP** | M3 |
| 4c | Clipboard history | provide | `clipvault` + `wl-clipboard-rs`, picker in hecate | 4 | config → shim | STRATEGIC | M4 |
| 5 | Screenshot | integrate | `grim` + `slurp` | 5 | config | **MVP** | M3 |
| 5 | Screen recording | integrate | `wl-screenrec` / `wf-recorder` | 4 | config | LATER | M6 |
| 6 | Portals — FileChooser | **provide** | `xdg-desktop-portal-gtk` → termfilechooser at M4 | 2 | config | **MVP** | M3 |
| 6 | Portals — ScreenCast | **provide** | `xdg-desktop-portal-wlr` | 3 | config | **MVP** | M3 |
| 6 | Portals — Settings | **provide** | `xdp-gtk` + gsettings `color-scheme` | 3 | config | **MVP** | M3 |
| 6 | Portals — Inhibit | **provide (as `none`)** | `Inhibit=none`, fall through to Wayland | 5 | config | **MVP** | M3 |
| 7 | Secrets / keyring | **provide** | `gnome-keyring` at M3; `oo7` behind the seam | 3 / 5 | config | **MVP** | M3 |
| 8 | Desktop entries + MIME | integrate | shipped `mimeapps.list`; `handlr-regex` optional | 4 | config | **MVP** | M3 |
| 8 | Autostart | **provide** | `xdg-desktop-autostart.target` | 5 | config | **MVP** | M3 |
| 8 | `xdg-user-dirs` | integrate | `xdg-user-dirs` | 5 | config | **MVP** | M3 |
| 9 | Tray (SNI) | provide *if at all* | none at M3; text-only segment at M4 | 2 | work | STRATEGIC | M4 |
| 10 | Logout / power | **provide** | **build** `helm ctl session` + which-key | 5 | shim | **MVP** | M3 |
| 11 | DnD / data-control | not break | river | — | none (test) | **MVP** | M3 |
| 12 | Desktop search | not provide | `fd` + `rg` via hecate | 5 | config → shim | LATER | M6 |
| 13 | Trash | integrate | yazi (already) + `trashy` + a restore plugin | 5 | config | STRATEGIC | M4 |
| 13 | `gio`/`gvfs` | integrate | `gvfs` as a dependency | 3 | config | STRATEGIC | M4 |
| 14a | Media / volume keys | **provide** | `wpctl` + `brightnessctl`, feedback in the bar | 5 | config + shim | **MVP** | M3 |
| 14b | MPRIS | integrate | `playerctl` bindings | 5 | config | STRATEGIC | M4 |
| 15.1 | Idle inhibition | not break | river + `Inhibit=none` | 5 | config | **MVP** | M3 |
| 15.4 | `xdg-activation` | **provide** | `RiverBackend` handles tokens | — | shim | **MVP** | M2–M3 |
| 15.5 | Removable media | integrate | `udisks2` + `udiskie --no-tray` | 3 | config | STRATEGIC | M4 |
| 15.7 | Night light | integrate | `wlsunset` | 4 | config+tmpl | LATER | M6 |
| 15.8 | GlobalShortcuts portal | not break | none exists | — | doc | LATER | M6 |
| 15.10 | Accessibility | eventually provide | — | 1 | work | LATER | M6 |
| 15.12 | PipeWire | integrate | dependency of ScreenCast | — | config | **MVP** | M3 |

**The MVP list, short form (M3):** notifications · lock + idle · polkit agent ·
clipboard survival · screenshot · portals (FileChooser, ScreenCast, Settings,
Inhibit=none) · secrets · MIME + autostart + `xdg-user-dirs` · logout/power ·
media and volume keys · `xdg-activation` · plus tests for copy/paste and DnD.
Thirteen items, of which ten are pure configuration and one is new Rust.

---

## 17. Where this touches the existing register

Rows this document would add to [PITFALLS.md](../PITFALLS.md) if adopted. Not
added here — that file belongs to whoever is editing it.

| Pitfall | What users see | Proposed answer | Guard |
|---|---|---|---|
| Inhibit portal answers when nothing is behind it | Screen blanks during a video call, though Firefox "inhibited" it | `org.freedesktop.impl.portal.Inhibit=none`; Firefox falls through to `zwp_idle_inhibit_manager_v1` | `doctor` asserts Inhibit is routed to `none` |
| No `Settings` portal / `color-scheme` unset | libadwaita and Flatpak apps are bright white on a black desktop | Route Settings to a backend and set the gsettings key in the session entry | `doctor` reads `color-scheme` back over D-Bus |
| No notification daemon owns the name | Slack, Element and calendar reminders are silent; some clients hang on the D-Bus call | Ship one as a session unit | `doctor` checks `org.freedesktop.Notifications` has an owner |
| No polkit agent registered | "Not authorized" when mounting a USB stick | Ship an agent as a session unit | `doctor` checks an agent is registered |
| `xdg-user-dirs` absent | Downloads land in `$HOME`; screenshots have nowhere to go | Depend on it; run `xdg-user-dirs-update` before clients | `doctor` checks `$XDG_DOWNLOAD_DIR` resolves |
| Layer-shell clients silently absent | Notifications and the lock screen never appear, and it looks like they crashed | Same root cause as the existing "layer-shell not served" row — `river-layer-shell-v1` gates **every** layer-shell client, not just `helm-bar` | extend the M2 layer-shell guard to a third-party client |

---

## 18. The uncomfortable list

Components where the best available option fits helm's vision **badly** and
there is no good answer. Stated plainly. These are `needs-human` candidates.

1. **The system tray.** The design has no tray region and no icons at all; SNI
   is an icon protocol with a mouse-driven context menu, and rendering it needs
   an icon-theme lookup that the 900 ms cold-start budget explicitly forbids.
   But Slack, Element, Steam, Nextcloud and KeePassXC all use it, and "close to
   tray" without a tray means the application vanishes while still running. Both
   positions are right. There is no version of this where helm both keeps the
   design and does not lose applications.

2. **The polkit agent.** Every maintained agent — `soteria`, `hyprpolkitagent`,
   `lxqt-policykit`, `polkit-gnome`, `mate-polkit` — is a toolkit modal dialog.
   None is layer-shell, none is keyboard-first by design, none is themeable from
   `palette.toml`. The best available is `soteria`, whose merit is that it is
   Rust and that GTK4 is a toolkit helm already recolours. The privileged
   password prompt — the one dialog where the user most needs to trust that they
   are looking at their own desktop — will be the least helm-looking surface in
   the session.

3. **The file chooser.** `xdg-desktop-portal-gtk` is a GTK3 dialog with icons,
   sidebars, a thumbnail grid and rounded corners. `termfilechooser` is a
   stopgap whose upstream is a three-way fork with no clear canonical
   maintainer, and it works by spawning a terminal, which means a floating
   window in a gapless tiling desktop. charon-as-portal at M4 fixes the
   aesthetics and inherits termfilechooser's fragility until then. There is no
   third option.

4. **The screencast backend.** `xdg-desktop-portal-wlr` is the only realistic
   choice, its last tagged release is 0.8.4 while its master branch is actively
   moving (commits dated 13 August 2026), its output-picker is a `slurp` popup
   rather than a helm surface, and it implements no `RemoteDesktop`, so "give
   control" in a screen share will simply not exist. `xdg-desktop-portal-luminous`
   is Rust and interesting and much less proven.

5. **Secrets.** `gnome-keyring` is proven and drags GNOME assumptions and an
   unstylable unlock dialog. `oo7` is Rust, cross-desktop, and now GNOME OS's
   default — and its public release history looks thin for something holding a
   user's credentials. Choosing the Rust-first option here means being early
   with other people's passwords.

6. **Notification appearance.** `fnott` and `mako` can be coloured from
   `palette.toml` and can be made square and 1px-bordered, and neither can be
   made to look like the window headers in `Desktop v3.dc.html`. They also draw
   text through their own font stack, so [ADR 0012](../adr/0012-font-fallback-is-a-contract.md)'s
   glyph contract does not cover them: a notification containing a rune can
   render tofu even though the bar cannot. That is a real, small, permanent
   inconsistency.

7. **Accessibility.** `tiny-skia` + `cosmic-text` surfaces expose no
   accessibility tree. helm is not usable with a screen reader today and will
   not be at M3. M6 lists it, but no candidate library was found that would
   retrofit AT-SPI onto a custom-drawn layer-shell surface without significant
   work.

8. **Desktop search.** There is no good option, and the recommendation is to
   ship none. That is defensible for helm's audience and it *is* a missing
   desktop feature, and users migrating from GNOME or KDE will notice.

---

## 19. What helm should build itself

[ADR 0007](../adr/0007-reuse-yazi-btop-starship.md) demands a written reason for
any rewrite. Three items qualify; everything else on this page should be reused.

### 19.1 Session control — `helm ctl session {logout,reboot,poweroff,suspend,lock}`

**Reason.** The reusable options (`wlogout`, `wleave`) are full-screen grids of
large icon buttons in GTK3 and GTK4 respectively. They contradict four separate
commitments at once: no icons ([HANDOFF](../../design/HANDOFF.md) — "All
iconography is Unicode glyphs"), keyboard-first, no toolkit beyond what theming
already requires, and no additional layer-shell surface with its own theme file.
Meanwhile helm *already has* the correct interface: the which-key strip. The
implementation is a `zbus` client for `org.freedesktop.login1` — five methods —
plus one keymap table entry, and it inherits the theme, the glyph contract and
the animation-free rendering for free because the which-key strip already has
them. Building is cheaper than integrating, which is rare enough to be worth
saying out loud. **Cost: ~150 lines. Milestone: M3.**

### 19.2 Volume, brightness and media feedback in `helm-bar`

**Reason.** Every OSD daemon (`swayosd`, `avizo`, `wob`) is a second
layer-shell surface that appears, waits, and fades — and the fade is the point
of them. [ADR 0009](../adr/0009-no-animation-budget.md) forbids it. The design
already places the feedback where it belongs: the bar's `♪ 64%` and `⚡ 87%`
segments exist in `Desktop v3.dc.html`. Making the existing `vol` module
event-driven on WirePlumber is work `helm-bar` owes
[ARCHITECTURE §4](../ARCHITECTURE.md#4-what-robust-and-snappy-mean-here) anyway.
Adopting an OSD would add a surface, a theme target, a dependency and an
animation to deliver information the bar is already showing.
**Cost: a bar module. Milestone: M2–M3.**

### 19.3 Clipboard-history recall inside hecate

**Reason.** The *store* should absolutely be reused (`clipvault` /
`wl-clipboard-rs`); the *picker* should not. Every clipboard manager in this
space delegates recall to `dmenu`/`rofi`/`fuzzel` precisely because a picker is
not their job. helm is already building `helm-hecate` on `nucleo` at M4, with
sources for PATH, desktop entries, spells and commands. Clipboard history is
another source. Adopting a second picker means a second fuzzy-match
implementation, a second theme template, a second keybinding vocabulary and a
second set of glyph assumptions, to do a thing hecate does. **Cost: one hecate
source. Milestone: M4.**

### Explicitly *not* to build

- **A notification daemon.** The spec is larger than it looks (actions, hints,
  replacement ids, urgency, icon data, capabilities negotiation), the reusable
  options are small C programs with no toolkit that theme cleanly, and getting
  it subtly wrong means an application's notifications silently do not appear.
  Revisit only if `helm-bar` gains a notification centre, which is not designed.
- **A lock screen.** ADR 0011 already says it: "getting a lock screen wrong is
  the worst class of bug in a desktop." `waylock` is ~1000 lines by the
  maintainer of the compositor helm ships. There is no argument for `helm-ward`
  before M6, if ever.
- **A secrets daemon.** Same reasoning, more so.
- **A portal backend.** Until `helm-compositor` exists (M5), helm has no
  privileged access the existing backends lack. After M5, revisit.

---

## 20. Corrections to things already written down

Two claims in the repository that this research contradicts. Recorded here
rather than edited into their files, because those files belong to other work.

1. **[ADR 0011](../adr/0011-session-integration-contract.md), *Needs a human*,
   option 1** says swaylock "is not a Wayland session-lock protocol client in
   older versions, which means on some compositors a crash can leave the session
   unlocked". True historically; misleading now. Current swaylock's own README
   states it "is compatible with any Wayland compositor which implements the
   `ext-session-lock-v1` Wayland protocol", and the support landed in swaylock
   1.7. The caveat should be rewritten as "older than 1.7" or dropped.

2. **[ADR 0011](../adr/0011-session-integration-contract.md), *Needs a human*,
   option 3** rejects waylock partly because it is "Zig, a toolchain we
   otherwise do not have (see ADR 0002)". That was true when written and was
   later revisited by [ADR 0013](../adr/0013-river-window-management-backend.md).
   [ADR 0015](../adr/0015-fedora-44-pre-alpha-baseline.md) then selected Fedora
   44's native River candidate and made compositor sourcing target-specific.
   Zig is therefore not automatically present in every packaging pipeline; the
   objection must be assessed per target. waylock still shares a maintainer
   with River, but that fact does not decide the lock-screen question.

---

## 21. Needs a human

Four decisions this document can research but must not take.

### 21.1 Which lock screen — a revised recommendation

Already an open `needs-human` question. This research adds evidence that changes
the answer.

| Option | For | Against |
|---|---|---|
| **`waylock`** | `ext-session-lock-v1`; smallest attack surface; three colours settable on the command line, so the cheapest possible template; **same maintainer as river**; actively developed (last commit 2026-05-21) | Zig must be accounted for per target; Fedora's native River candidate does not supply that toolchain. Version is `1.7.0-dev`, i.e. no recent tag |
| `gtklock` | ADR 0011's current pick; themeable from the `gtk.css` helm already generates | GTK3 in the lock path; slow cadence (v4.0.0, translation commits since) |
| `swaylock` | Packaged on all three target distributions; richest config; `ext-session-lock-v1` since 1.7 | Larger; cadence has slowed (1.8.6) |

**Research preference: `waylock`**, changed from ADR 0011's `gtklock`, because
a lock screen is the one component where "smallest thing that does the job, by
River's maintainer" may beat "most themeable". The independent lock-screen
decision remains human-gated, and its per-target Zig cost is not resolved here.

**Also still needed from a human:** the idle policy numbers. A defensible
starting proposal, offered so there is something to argue with rather than a
blank: dim at 5 min, blank at 10 min, lock at 10 min, lock unconditionally on
lid close and on `before-sleep`. These are security defaults and should be
decided, not inherited from this paragraph.

### 21.2 Does helm ship a system tray?

| Option | Consequence |
|---|---|
| (a) No tray, ever | Faithful to the design. Slack, Element, Steam, Nextcloud and KeePassXC lose their "close to tray" recovery path. Must be documented prominently, not discovered |
| (b) Text-only SNI segment in `helm-bar` at M4 | Keeps glyph-only rendering and keyboard-first activation; recovers the real capability; departs visibly from `Desktop v3.dc.html`; real work in the crate with the tightest frame budget |
| (c) Full icon tray | Recovers everything, breaks the design language, and breaks the "no icon-cache scan" cold-start budget |

**Recommendation: (a) for M3 with a prominent note in the install
documentation, (b) for M4 behind a config flag defaulting off.** A human should
decide whether (b) is an acceptable departure from the mockup before anyone
builds it.

### 21.3 `gnome-keyring` or `oo7`?

The same class of decision as the lock screen: it is a credential-security
posture, not a technical preference.

| Option | For | Against |
|---|---|---|
| `gnome-keyring` | Packaged on all three targets; PAM auto-unlock is documented and understood; a decade of scrutiny | GNOME session assumptions; an unstylable GTK unlock prompt; not Rust |
| `oo7` | Rust; cross-desktop by design; `oo7-pam`, `oo7-portal` and `oo7-cli`; **GNOME OS's default since ~June 2026**; COSMIC's native provider | Public release history looks thin relative to that adoption; being early with a user's stored passwords is a different risk from being early with a bar |

**Recommendation: `gnome-keyring` at M3, `oo7` documented as the alternative
behind the same unit-file seam, revisit at M4 once its release story is
legible.**

### 21.4 The notification daemon's Fedora gap

`fnott` is the best technical fit on this page and appears not to be in Fedora's
repositories (verified present in Debian trixie/sid and nixpkgs; not found for
Fedora). Fedora 44 is the sole pre-alpha Fedora packaging target; this dated
search is not Fedora runtime evidence.

| Option | Consequence |
|---|---|
| (a) `mako` everywhere | One daemon, packaged everywhere, slightly worse fit (pango/cairo, a second font-selection path) |
| (b) `fnott` everywhere, building it in helm's own RPM | Best fit; adds one small meson C package to Fedora packaging as an independent maintenance burden |
| (c) `fnott` where packaged, `mako` on Fedora | Two theme templates and two behaviours to support; rejected |

**Recommendation: (b)** on technical fit, while recognizing it as a new Fedora
packaging-maintenance commitment. Fedora's native River candidate does not
remove that cost, and packaging maintenance is a person's time.

---

## 22. Verification ledger

Because "a confidently wrong maintenance claim is worse than an honest 'could
not verify'".

**Verified against a primary source, August 2026:**

- river's protocol globals, read from
  [`river/Server.zig`](https://codeberg.org/river/river/raw/branch/main/river/Server.zig),
  [`river/LockManager.zig`](https://codeberg.org/river/river/raw/branch/main/river/LockManager.zig)
  and [`river/IdleInhibitManager.zig`](https://codeberg.org/river/river/raw/branch/main/river/IdleInhibitManager.zig)
  on `main`. `ext-session-lock-v1` and `zwp_idle_inhibit_manager_v1` confirmed by
  their creation lines; `ext-idle-notify` confirmed only as a *reference* to
  `server.input_manager.idle_notifier` (see assumed, below).
- river releases: v0.4.8 (7 Aug 2026), v0.4.7 (4 Aug 2026), v0.4.6 (29 Jul 2026),
  from [the releases page](https://codeberg.org/river/river/releases).
- river XWayland is an opt-in `-Dxwayland` build flag.
- `xdg-desktop-portal-wlr` master has commits dated 13 August 2026; latest tag is
  0.8.4.
- `fnott` 1.8.0, 16 July 2025; C; fcft/pixman/fontconfig, no GTK;
  `RRGGBBAA` colours; `[low]`/`[normal]`/`[critical]` sections; packaged in
  Debian trixie (1.7.1) and sid/forky (1.8.0) and in nixpkgs.
- `mako` README says "Works on Sway." (**not** "Sway, Hyprland and
  river-classic" — that phrasing came from an aggregator, not upstream);
  dependencies meson, wayland, pango, cairo, sd-bus, optional gdk-pixbuf, dbus.
- `dunst` v1.13.2, 18 March 2026, native Wayland.
- swaylock README states `ext-session-lock-v1` compatibility; latest release
  1.8.6; support landed in 1.7.
- `waylock`: Zig, `ext-session-lock-v1`, 1.7.0-dev, latest commit 2026-05-21,
  colours via `-init-color`/`-input-color`/`-fail-color`.
- `gtklock` v4.0.0; latest commit 4 February 2026.
- `soteria`: Rust + GTK4 + relm4, MSRV 1.85, config at
  `~/.config/soteria/config.toml`.
- `wl-clipboard-rs`: library plus `wl-copy`/`wl-paste`/`wl-clip`; "the protocol
  used for clipboard interaction is `ext-data-control` or `wlr-data-control`".
- `wl-clipboard` (C): latest tag 2.3; ext-data-control requested in
  [issue #242](https://github.com/bugaevc/wl-clipboard/issues/242), closed via
  PR #255.
- `clipvault`: Rust, daemon + external picker, argfile config, regex ignore
  patterns and window-based filtering.
- `cliphist`: Go, `wl-paste --watch` model, optional config file, text and images.
- `oo7`: Rust `org.freedesktop.secrets`; components `oo7-daemon`, `oo7-portal`,
  `oo7-cli`, `oo7-pam`; **GNOME OS default** per
  [This Week in GNOME #254](https://thisweek.gnome.org/posts/2026/06/twig-254/).
- `xdg-desktop-portal-gtk`'s Inhibit provider tries `org.gnome.SessionManager`,
  then `org.freedesktop.ScreenSaver`, and if both fail "will throw a warning, do
  nothing, and pretend to have succeeded"
  ([issue #465](https://github.com/flatpak/xdg-desktop-portal-gtk/issues/465));
  `org.freedesktop.impl.portal.Inhibit=none` is a supported `portals.conf` value.
- `xdg-desktop-portal-luminous`: Rust, `zwlr_screencopy` + `ext-image-copy`, uses
  `libwayshot`, no grim dependency.
- `termfilechooser`: original by GermainZ, plus active `boydaihungst` (yazi) and
  `hunkyburrito` forks.
- `uwsm`: Python; exports environment diffs to both the systemd user manager and
  the D-Bus activation environment; XDG autostart into `app-graphical.slice`;
  `graphical-session.target` binding; plugins for sway, wayfire, labwc, hyprland,
  niri, mango — **no river plugin listed**.
- `systemd-xdg-autostart-generator` exists, is opt-in via
  `xdg-desktop-autostart.target`, and handles `OnlyShowIn`/`NotShowIn`.
- yazi trashes to the freedesktop trash on Linux; `d` trashes, `D` deletes.
- `handlr` is Rust; `handlr-regex` is the maintained fork.
- The design's bar has **no tray and no notification region**: extracted from
  `design/Desktop v3.dc.html`, the right-hand segment is
  `↑ 18k ↓ 1.2M · cpu 31% · mem 9.8G · gpu 44° · ♪ 64% · ⚡ 87% · 26·08·2026 ☾ 14:32`.
- `system-tray` (JakeStanger): async Rust SNI + DBusMenu, requires tokio.
- mako, grim, slurp, wl-clipboard, swayidle and swaylock are all in Fedora.

**Assumed, inferred, or could not verify — treat with suspicion:**

- **`ext-idle-notify-v1` creation in river.** I verified a reference to
  `server.input_manager.idle_notifier` but did not read `InputManager.zig`. High
  confidence it exists (`swayidle` is widely used with river) but not
  first-hand.
- **`mako` 1.11.0's release date.** The releases page rendered "March 26" with
  no year. Almost certainly March 2026; not verified.
- **`swaylock` 1.7's release date.** The fetch reported "November 27, 2019",
  which cannot be right — `ext-session-lock-v1` did not exist then. The
  *capability* claim is verified from the README; the *date* is wrong and I did
  not establish the correct one.
- **Whether `ext-data-control-v1` support is in a tagged `wl-clipboard` release.**
  The issue is closed and a PR is linked; the latest tag is 2.3 and I could not
  confirm the code is in it. This is the main argument for preferring
  `wl-clipboard-rs`, which states the support in its own documentation.
- **`fnott` in Fedora.** Searched, not found. "Not found by search" is weaker
  than "confirmed absent".
- **`oo7`'s current stable version.** The GitHub releases page showed a sparse
  and old-looking tag list that contradicts the GNOME OS adoption. One of the
  two is stale; I could not determine which.
- **`xdg-desktop-portal-wlr` 0.8.4's release date.** Two sources disagreed
  (a "July 2024" reading against Debian's "0.8.0 accepted November 2025,
  0.8.1 January 2026"). Master's activity is verified; the tag date is not.
- **Whether `mako`, `fnott`, `swaylock`, `waylock` etc. actually run under river
  0.4 with helm as the window manager.** This is an *inference* from §0.1: they
  are ordinary `wlr-layer-shell` clients and river exposes layer-shell when the
  window manager serves `river-layer-shell-v1`. Nobody has run it, and
  [the README says so](../../README.md#how-this-repository-was-built):
  "Nothing here has run on real hardware." **Every recommendation on this page
  is subject to that.**
- **Whether river forwards `xdg-activation-v1` tokens to the window manager**
  (§15.4), and how. The global is created; the manager-side handling was not
  checked against `river-window-management-v1`'s XML.
- **Whether river creates `xdg-output-unstable-v1`.** Not in `Server.zig`. Older
  `grim` needed it for output naming; current `grim` may use `wl_output` v4
  names instead. Worth checking before assuming `grim -o` works.
- **`soteria`'s keyboard usability.** Its documentation does not say, and this
  could only be settled by running it.
- **`gtklock` v4.0.0's release date** (October 2024 assumed from the page).
- **Fedora/Ubuntu package availability** for `waylock`, `soteria`, `clipvault`
  and `wl-clip-persist`. Not checked, and it matters for M3 packaging.
