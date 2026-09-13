# Installing realm

The `Mod+b` browser shortcut uses your configured default browser. Install a
browser and set its desktop entry as the default (for example through the
browser's own default-browser setting). Realm does not choose a browser brand.
The packaged `realm-browser` helper reports an absent default or a failed
launch instead of silently choosing another application.

> **realm 0.1.0 is pre-alpha.** The native recipes require the complete Realm
> runtime payload — `realmctl`, `realm-wm`, and `realm-bar` — plus the session
> contract. Fedora CI clean-installs the exact built RPM into an empty Fedora
> 44 root with normal DNF dependency resolution, then runs the installed Realm
> CLI against the installed palette and probes installed River. The installed
> NixOS package has also completed a display-manager login into a graphical
> Realm session in QEMU, including managed windows, the bar, key discovery and
> `realmctl doctor`. Ubuntu's exact Realm and private River packages also
> clean-install together in an empty amd64 Noble root. Native Ubuntu and Fedora
> graphical login and physical hardware remain unverified.
>
> A failed `realm-wm` unit returns the user to the display manager rather than
> leaving bare river. That failure policy remains required while live native
> session verification is pending ([SPEC 0005](specs/0005-session-startup.md)
> §2). Set `REALM_ALLOW_NO_WM=1` only for deliberate bare-river diagnostics.
>
> Every section below has a **What this actually installs today** block that says
> exactly what you get and what you do not.
>
> [SPEC 0012](specs/0012-activation-launch-lifecycle.md) is Accepted, and the
> shipped fixed terminal and launcher select one sealed generation and hold its
> process lease until the owned child exits. The broader durable profile-launch
> admission, record reconciliation and restart/logout ownership path is not yet
> wired into every launcher. Do not read the fixed-consumer proof as complete
> profile-lifecycle or daily-driver evidence.

realm intends to support three M3 platforms, but today's evidence differs by
target ([ARCHITECTURE.md §5](ARCHITECTURE.md)):

| Platform | Delivery | State today |
|---|---|---|
| NixOS / Nix | flake: `packages.default`, `nixosModules.realm`, `homeManagerModules.realm` | Reference build; its installed display-manager session boots River and the Realm desktop in the QEMU VM. This is not physical-hardware evidence |
| Ubuntu 24.04 LTS + | `.deb` from `packaging/debian/` and `packaging/debian-river/` | CI builds the exact Realm and private River packages and clean-installs them together in an empty amd64 Noble root. Graphical login and package publication remain unverified |
| Fedora 44 (pre-alpha) | RPM from the retained-only source kit | Builds in a pinned Fedora 44 image; its exact output clean-installs into an empty Fedora 44 root and its installed CLI/palette and River probes pass. Graphical login and SELinux remain unverified; portal effects are verified only in the NixOS reference VM |

Anything else is best-effort. The flake is the definition; the deb and the rpm
follow from the same tree.

---

## The compositor: River 0.4 protocol, target-specific sources

realm does not ship its own compositor yet. It runs on **river 0.4.x** and *is*
river's window manager, driving it over `river-window-management-v1` (ADR 0013).
Two consequences you will meet immediately:

- **river 0.4 does no window management on its own.** Until `realm-wm` attaches,
  river places nothing — and serves no layer shell, so the bar maps nothing.
  The installed NixOS VM verifies that the daemon attaches, places ordinary
  Wayland windows and serves the bar. Native Ubuntu and Fedora graphical-login
  verification remains pending; a failed unit must still return the login
  rather than leave an inert compositor.
- **`river-window-management-v1` is declared *stable* as of river 0.4.0**, with
  a forward-compatibility pledge to 1.0.0. The residual risk is not a protocol
  classification but trust in a single maintainer of a pre-1.0 project. realm
  treats a version bump as an event to re-verify, not a surprise to absorb.
  Fedora 44 has an official `river >= 0.4.0` package candidate; package
  availability alone does not prove Realm runtime compatibility.



## NixOS and Nix

The flake lives at the repository root; the parts it imports live in
`packaging/nix/`. All package builds and verification run in CI only. The
reference Nix checks include an installed display-manager login in a KVM-backed
NixOS VM; they are not instructions to rebuild release evidence locally.

> **`flake.lock` is committed.** It pins the Nix inputs used by this reference
> build. Refresh it only deliberately with `nix flake update` or `nix flake
> lock`, review the revision/hash diff, and let the matching CI workflow build
> and verify the result before merging the update.

### As a NixOS module

```nix
{
  inputs.realm.url = "github:pipeliner/realms-de";

  outputs = { nixpkgs, realm, ... }: {
    nixosConfigurations.yourhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        realm.nixosModules.realm
        {
          programs.realm.enable = true;
          # programs.realm.compositor = pkgs.river;   # default: pinned river 0.4.x
          # programs.realm.paletteFile = ./my-palette.toml;
          # programs.realm.cursorTheme = "Adwaita";
          # programs.realm.cursorSize  = 24;
        }
      ];
    };
  };
}
```

The module installs the session entry, registers it with the display manager,
pulls in the portal backends (`xdg-desktop-portal-gtk` for file dialogs,
`xdg-desktop-portal-wlr` for ScreenCast) and routes each interface explicitly,
enables dconf, polkit and XWayland, and installs the fonts the glyph inventory
needs. It deliberately does
**not** set `XDG_CURRENT_DESKTOP` globally — that variable describes one
session, and a machine that also offers GNOME must not have every session
claiming to be realm. The session wrapper exports it per login.

For the user half — palette, generated configs, user units — add
`realm.homeManagerModules.realm` to your home-manager configuration and set
`programs.realm.enable = true` there too.

### What this actually installs today

- `bin/realmctl` — the theme CLI and display readiness probe.
- `bin/realm-wm` — the Realm session daemon/window manager.
- `bin/realm-bar` — the Wayland layer-shell status surfaces.
- `bin/realm-session` — the session entry, with river, `systemctl`,
  `dbus-update-activation-environment`, `dbus-run-session` and `gsettings` on
  its PATH.
- `share/wayland-sessions/realm.desktop` — the login entry. river's own package
  also provides a `river` entry; only the realm one applies realm's environment
  contract.
- `lib/systemd/user/realm-session.target`, `realm-wm.service`,
  `realm-bar.service`, `realm-session-abort.service` — installed together with the
  `realm-session.target.wants/` symlinks that make starting the target actually
  start something.
- `share/xdg-desktop-portal/realm-portals.conf`, and the equivalent as
  `xdg.portal.config.realm` — a named backend per interface.
- `share/realm/palette.toml`, `/etc/realm/palette.toml`.
- river 0.4.x, yazi, btop, starship, zsh, fuzzel, foot and slurp as runtime
  packages.

The generated theme is not installed as a static package artifact; `realmctl`
applies it into the user's configuration.

`checks.session-boots` boots the installed NixOS display-manager session and
asserts the wrapper, units, palette and River connection. It also drives real
keybindings, observes managed windows, the bar, which-key and grimoire, and
runs the installed `realmctl doctor`. This is QEMU/KVM evidence, not a claim
about physical hardware or every M3 target.

---

## Ubuntu 24.04 LTS and newer

There is no apt repository or published native artifact yet (`NEEDS-HUMAN` in
`packaging/debian/control`: PPA, self-hosted apt, or GitHub Releases). All
package builds and verification run in CI only; this guide does not ask users
or contributors to reproduce the package toolchain locally. CI now builds the
exact `realm_0.1.0_amd64.deb` and `realm-river_0.4.8-1_amd64.deb`, then
clean-installs both into an empty amd64 Noble root with normal dependency
resolution and verifies the installed River version, private ELF resolution,
palette and session preflight. That is native package-integration evidence,
not a published installation path or a graphical-login result.

The producer copies only Debian metadata, the staging/linkage helpers, and the
retained Realm workspace bundle into the package source directory. The checkout
is intake context for running that producer; it is not the package build input.
`debian/rules` rejects a full checkout before Cargo rather than treating it as
a second workspace authority.

The retained native kit and source workspace require **Rust 1.89 or newer**.
Ubuntu 24.04's default rustc is 1.75, below that floor. Noble Updates carries
versioned `rustc-1.89`/`cargo-1.89` packages
(`1.89.0+dfsg~24.04-0ubuntu0.24.04.2`, checked 2026-09-13), and the CI recipe
selects the newest complete versioned pair it finds, failing rather than
building with the wrong compiler. This records the package-build contract; it
is not a local build instruction.

**Three runtime dependencies are not in the Ubuntu 24.04 archive** (checked
against noble's package lists):

| Missing | Effect | Current status |
|---|---|---|
| `river` (any version) | No compositor — realm cannot start | The separate `realm-river` 0.4.8-1 package supplies Realm's private River closure on amd64 Noble; its exact pair with `realm` clean-installs in CI |
| `yazi` | charon (files) is missing | Packaging remains unresolved |
| `starship` | thoth's prompt falls back to plain zsh | Packaging remains unresolved |

`fonts-ibm-plex` is in *multiverse*, so it is a Recommends rather than a
Depends: a hard dependency would make realm uninstallable on a box with only
main and universe enabled. Without it the bar renders in the system monospace
font — the glyph probe degrades rather than drawing tofu (ADR 0012), but it is
not the design's typeface. `sudo add-apt-repository multiverse && sudo apt
install fonts-ibm-plex` fixes it.

### What this actually installs today

`/usr/bin/realm-session`, `/usr/share/wayland-sessions/realm.desktop`, the four
user units under `/usr/lib/systemd/user/` **plus the
`realm-session.target.wants/` symlinks**, `/usr/share/xdg-desktop-portal/realm-portals.conf`
and `/usr/share/realm/palette.toml`, plus the mandatory `/usr/bin/realmctl`,
`/usr/bin/realm-wm`, and `/usr/bin/realm-bar` runtime payload. A missing binary
fails the package build. The companion `realm-river` package installs River at
`/usr/lib/realm/bin/river`, its private shared-library closure under
`/usr/lib/realm/lib`, and its selected libinput quirks under
`/usr/lib/realm/share/libinput`; it does not replace `/usr/bin/river`.

---

## Fedora 44 (pre-alpha)

There is no Realm Fedora repository or published RPM artifact. The tracked RPM
is pre-alpha and does not yet have a verified graphical login. Fedora 44 has
one pinned Cargo-smoke lane and one pinned retained-source RPM lane; the latter
builds the RPM from the retained-only source kit, installs that exact output
into an empty Fedora 44 root with normal DNF dependency resolution, verifies
its NEVRA, and runs the installed Realm CLI/palette and River version probes.
All package builds and verification run in CI only. This does not exercise a
graphical session, portals, SELinux, or physical hardware, and the successful
output is not currently published for download.

The resulting RPM `Source0` contains only Fedora metadata, the shared helpers,
and the same retained Realm bundle. `%prep` rejects a checkout-shaped Source0
and stages Cargo exclusively from the canonical inner `source.tar.gz`.

Fedora 44's official package listing reported `rust` and `cargo` 1.97.1 on
2026-08-29, above the current Rust 1.89 MSRV. Fedora repositories float, so
the CI lane records the selected compiler version and refuses anything below
the floor. Do not lower `rust-version` to match a distro toolchain, because the
locked dependency graph would still fail to parse. RPM builds remain governed
by the toolchain requirement in `packaging/fedora/realm.spec`.

**Command names are settled:** the CLI installs as `realmctl`, the session entry
as `realm-session`, and the window manager as `realm-wm`. These names are the
same on every supported distribution.

### SELinux

realm is intended to be SELinux-clean and to require no custom labels or policy
module. The reasoning, so it can be checked rather than trusted: everything
installs into standard locations whose default labels in the targeted policy are
already correct (`/usr/bin` → `bin_t`, `/usr/lib/systemd/user` →
`systemd_unit_file_t`, `/usr/share` → `usr_t`); everything runs in the user's own
session domain; realm's control socket lives in `$XDG_RUNTIME_DIR/realm/ctl.sock`
inside `/run/user/$UID`; and nothing is setuid, has file capabilities, listens on
a port or runs as a system service.

**This has not been verified on a Fedora box in enforcing mode.** SELinux
verification is tracked as post-MVP work and is not a launch gate; current
Fedora evidence is limited to the clean installroot and installed probes stated
above.

### What this actually installs today

The same set as the deb: the mandatory `realmctl`, `realm-wm`, and `realm-bar`
runtime payload, session entry, login entry, four user units and their `.wants`
symlinks, portal policy, and palette. A missing runtime binary fails `%install`.

---

## Why there is a wrapper script at all

`realm-session` is not boilerplate. It implements the ordering contract in
ADR 0011, and the order is the entire point:

The numbered list below describes the currently shipped SPEC 0005 path.
Accepted SPEC 0012 defines the broader crash-safe activation lifecycle. Its
sealed-generation fixed terminal/launcher slice is shipped; complete durable
profile-launch admission and reconciliation is still pending.

1. Export `XDG_CURRENT_DESKTOP=realm`, `XDG_SESSION_TYPE=wayland`,
   `XDG_SESSION_DESKTOP=realm`, `XCURSOR_THEME`, `XCURSOR_SIZE` — **before** the
   compositor starts, because portal backend selection and cursor loading both
   read them at client start-up.
2. Start river — in the background, never `exec`: the entry outlives the
   compositor because it owns teardown.
3. **Wait** for the Wayland socket to actually appear, then require
   `realmctl wait-display` to complete a real registry round trip within the
   same startup deadline. The file-existence-only `DEGRADED NO-DISPLAY-PROBE`
   path remains solely for an incomplete or manually assembled installation.
   Importing early imports nothing, silently, and everything downstream
   inherits the hole.
   XWayland's `DISPLAY` is discovered the same way, by diffing
   `/tmp/.X11-unix/X*` across the compositor start.
4. Publish the environment to **both** `systemctl --user import-environment`
   *and* `dbus-update-activation-environment --systemd`. Two launchers, two
   environments; a service D-Bus activates later gets whatever was in the
   activation environment at that moment.
5. Mirror the cursor theme into `gsettings` (GTK reads that, not the
   environment).
6. Only now start `realm-session.target` — the window manager, then the bar —
   and then **verify each unit individually**. `systemctl start` exiting 0 is
   not a success signal: a unit whose `ConditionEnvironment=` is unmet is
   *skipped*, not failed, so the target activates, `systemctl --user --failed`
   is empty, and the desktop is inert. If the window manager is not `active`,
   the entry logs `FATAL WM-ABORT` and returns you to the display manager.

On exit the current pre-alpha script stops the Realm target and clears both
environments so the next session does not inherit a `WAYLAND_DISPLAY` pointing
at a dead socket. The target is bound to `graphical-session.target`, and the
fixed terminal/launcher children hold generation process leases. This is not
proof that every SPEC 0012 profile-launch path is complete: durable admission
freeze and record reconciliation are not yet wired into every launcher.

Every degradation emits exactly one line with a stable code, so it can be
grepped, quoted in a bug report and looked up here:

```
<timestamp> realm-session: DEGRADED <CODE>: <what you are losing>
```

| Code | Means | You lose |
|---|---|---|
| `NO-SESSION-BUS` | No session bus, and `dbus-run-session` could not supply one (the entry re-execs under it once) | Portals, file dialogs, screen sharing |
| `NO-DBUS-ACTIVATION` | `dbus-update-activation-environment` missing | D-Bus-activated services never see the display: file dialogs hang ~25 s |
| `NO-SYSTEMD-USER` | No reachable `systemd --user` | Supervision; clients run under a bounded respawn loop instead |
| `NO-DISPLAY-PROBE` | `realmctl wait-display` not installed | Certainty that the compositor is dispatching, not merely bound |
| `NO-XWAYLAND` | No new X11 socket appeared | X11 applications; `DISPLAY` stays unset rather than being imported empty |
| `NO-GSETTINGS` | `gsettings` or its schemas missing | GTK apps use their own cursor |
| `NO-CURSOR-THEME` | The named theme is under no icon directory | The pointer is the default arrow |

Fatal conditions are `FATAL <CODE>` and exit: `NO-RUNTIME-DIR`, `NO-COMPOSITOR`,
`NO-SOCKET`, `WM-ABORT`. `REALM_STRICT=1` additionally makes a missing realm
binary fatal. The entry logs to
`${XDG_STATE_HOME:-~/.local/state}/realm/session.log`.

`PATH` is deliberately **not** imported into the user manager: it would replace
`PATH` for every user unit for the manager's lifetime, surviving logout on a
lingering user. `REALM_IMPORT_PATH=1` opts in, which is what a Nix profile the
user manager does not know about needs.

Preflight, without logging in:

```sh
realm-session --check
```

That checks wrapper prerequisites without starting a session. From a terminal
inside a running Realm session, use the shipped health command:

```sh
realmctl doctor
```

---

## Troubleshooting

Each entry names the symptom, what is actually happening, how to check it by
hand, and what the shipped `realmctl doctor` reports. The full failure register
is [docs/PITFALLS.md](PITFALLS.md).

### A file dialog hangs for ~25 seconds, then falls back to the toolkit's own

25 seconds is the D-Bus activation timeout, not a filesystem hint.
`xdg-desktop-portal` was activated without `WAYLAND_DISPLAY` in the activation
environment, came up with no display, and D-Bus waited.

```sh
systemctl --user show-environment | grep -E 'WAYLAND_DISPLAY|XDG_CURRENT_DESKTOP'
busctl --user get-property org.freedesktop.portal.Desktop \
  /org/freedesktop/portal/desktop org.freedesktop.portal.FileChooser version
```

The first must list both variables; the second must answer immediately. If the
compositor's own environment has `WAYLAND_DISPLAY` and systemd does not, the
import ran too early or was skipped. If the portal probe fails, a missing or
stale D-Bus activation import is one possible cause alongside a broken portal
service or selected backend; D-Bus does not expose the stored value directly.

*`realmctl doctor` compares the exact process and systemd values, then reports
the D-Bus path separately through a bounded functional portal probe. D-Bus does
not expose its stored activation-environment value, so doctor does not invent
one.*

### Screen sharing offers no sources, or produces nothing

Either `XDG_CURRENT_DESKTOP` was not `realm` when the portal started, so
`portals.conf` matching failed, or the chosen backend implements no ScreenCast.
Realm installs both `xdg-desktop-portal-gtk` and
`xdg-desktop-portal-wlr`. Its named policy routes FileChooser and Settings to
`gtk`, ScreenCast and Screenshot to `wlr`, and Inhibit to `none`. Check that the
`wlr` backend and the installed `realm-portals.conf` are both present before
changing that policy. `realmctl doctor` checks the routing and advertised
interfaces. The installed NixOS reference VM has completed the real
CreateSession/SelectSources/Start sequence and consumed a nonempty frame from
the restricted PipeWire node returned by xdpw. That proves one emulated River
output reaches one portal client; browser picker behavior and useful capture on
physical hardware remain unverified under SPEC 0005 A15.

### Tofu boxes instead of runes, or `𓂃` renders as a rectangle

The font stack does not cover realm's glyph inventory. realm's answer is a startup
probe plus a documented ASCII fallback for every glyph (ADR 0012), so a bare
font degrades instead of drawing tofu — but the design's face still has to be
installed to look right.

```sh
fc-match "IBM Plex Mono"
fc-list | grep -iE 'symbols|symbola'
```

Install `fonts-ibm-plex` + `fonts-symbola` (Ubuntu, the first from multiverse),
or `ibm-plex-mono-fonts` + `google-noto-sans-symbols2-fonts` (Fedora). The Nix
module installs IBM Plex, but does not install a Symbola or Nerd Font
automatically. These symbol-font packages are optional recommendations: the
glyph probe and ASCII fallbacks keep Realm legible when they are absent.

*`realmctl doctor` prints the glyph coverage summary —
`realm-core::glyphs::Probe::summary()` already produces exactly that line.*

### The cursor is a black X11 arrow, or disappears over some windows

`XCURSOR_THEME`/`XCURSOR_SIZE` reached some clients and not others, or reached
the environment but not `gsettings`. Three places must agree, because three
client families read three different sources: the session environment, the
imported systemd/D-Bus environment, and GTK's gsettings keys.

```sh
systemctl --user show-environment | grep XCURSOR
gsettings get org.gnome.desktop.interface cursor-theme
```

The wrapper sets all three. If the theme name is set but the cursor is still
wrong, the theme itself is not installed: `ls /usr/share/icons/*/cursors`.

*`realmctl doctor` checks that the theme resolves *and* that gsettings agrees.*

### I select realm and land straight back at the login screen

Expected in 0.1.0, and the log says exactly why:

```sh
tail -20 ~/.local/state/realm/session.log
```

Look for `FATAL WM-ABORT: river is running but realm is not managing it`. river
0.4 does no window management by itself; `realm-wm` does. The entry refuses to
leave you on an inert compositor because, with no layer shell, realm cannot draw
an explanation onto it. Check `systemctl --user status realm-wm.service` for the
daemon's concrete startup error.

`REALM_ALLOW_NO_WM=1` keeps river running without a window manager — useful for
poking at the compositor, not a desktop. Without that diagnostic override, the
abort means the window manager failed, and `systemctl --user status
realm-wm.service` is the next stop; `realm-session-abort.service` is what turns
its failure into a returned login.

If the window manager exits **69**, another window-management client already
holds river's global — a leftover `realm-wm`, or one started by hand. river
answers `unavailable` to the second, so the unit deliberately does not restart:
`pgrep -a realm-wm` finds the holder.

### The bar never appears

Under river the bar is a layer-shell client served by *realm's own window
manager*, so a missing bar usually means the window manager is not serving
layer-shell — not that the bar is broken. Check `realm-wm.service` before
`realm-bar.service`. A crashed bar restarts on its own and never takes the
session down; a crashed window manager leaves river unmanaged, which is the
sharper failure.

```sh
systemctl --user status realm-wm.service realm-bar.service
systemctl --user list-dependencies realm-session.target
journalctl --user -u realm-bar.service -b
```

If `list-dependencies` shows the target with nothing under it, the package's
`realm-session.target.wants/` symlinks are missing — starting the target then
starts nothing at all and still exits 0.

### Running apps keep the old theme after `realmctl theme apply`

This is expected: apply selects a sealed generation for future launches and
never rethemes an existing process. Test a newly launched program started
through a verified Realm launch profile. If that future launch still uses the
old generation, check the generation selection/profile integration before
debugging toolkit reload behavior.

User units started before the import and inherited an empty environment. Compare
the three views: the compositor's `/proc/<pid>/environ`, `systemctl --user
show-environment`, and what D-Bus hands an activated service. Any disagreement
is a separate session-startup bug; theme apply does not repair it.

---

## Open decisions and acceptance (`NEEDS-HUMAN`)

These remaining choices and physical acceptance checks do not invalidate the
bounded CI evidence above.

| Decision or acceptance | Where | Status / options |
|---|---|---|
| Package hosting | `packaging/debian/control`, spec | PPA / self-hosted apt / GitHub Releases; COPR / dist-git / release tarballs |
| Idle and lock | `packaging/systemd/realm-session.target` | SPEC 0005 OQ-1: river 0.4 speaks `ext-session-lock-v1`, so the locker must too — gtklock (recommended), waylock (Zig), a new-enough swaylock, or `realm-ward` in M6. The idle defaults are a user-visible security decision and are not guessed |
| Physical browser ScreenCast acceptance | SPEC 0005 A15 | The NixOS reference VM has consumed a real xdpw/PipeWire frame from an emulated River output. A real browser picker and useful stream on physical hardware remain unverified |
| How river is pinned in Nix | `packaging/nix/support.nix` | Pin via the nixpkgs input (current); or a tag-pinned input needing a second, human-produced `zigDeps` hash |
