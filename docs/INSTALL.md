# Installing helm

> **helm 0.1.0 is pre-alpha and does not install a working desktop.** The
> binaries that make helm a desktop — `helm-wm` (the window manager) and
> `helm-bar` — are not written yet (M1–M2 in [docs/MVP.md](MVP.md)). `helmctl`
> is installed with `theme apply`, `theme lint`, and `theme diff`. What you can
> install today is the *session contract*: the login entry, the entry script that
> performs the systemd/D-Bus environment handshake, the systemd user units, the
> portal policy and the palette.
>
> **Selecting the helm session today ends in `FATAL WM-ABORT` and returns you to
> the display manager.** That is deliberate, not a crash: river 0.4 does no
> window management of its own, so without `helm-wm` there is no layer shell and
> helm could not draw an error onto the screen it was occupying. A returned login
> beats a black screen ([SPEC 0005](specs/0005-session-startup.md) §2). Set
> `HELM_ALLOW_NO_WM=1` to stay in bare river instead.
>
> Every section below has a **What this actually installs today** block that says
> exactly what you get and what you do not.
>
> **Candidate SPEC 0012 is not implemented by today's package.** The current
> entry starts river without a persistent activation claim, the WM/bar units
> are only `PartOf=graphical-session.target`, and current exit has no durable
> admission-freeze or launch-record/lease reconciliation. Those differences are
> expected red implementation evidence while SPEC 0012 remains Draft; current
> packages must not be described as satisfying its restart/logout contract.

helm intends to support three M3 platforms, but today's evidence differs by
target ([ARCHITECTURE.md §5](ARCHITECTURE.md)):

| Platform | Delivery | State today |
|---|---|---|
| NixOS / Nix | flake: `packages.default`, `nixosModules.helm`, `homeManagerModules.helm` | Reference build. Evaluates; VM test asserts the contract, not the desktop |
| Ubuntu 24.04 LTS + | `.deb` from `packaging/debian/` | Builds; three runtime dependencies are not in the Ubuntu archive (below) |
| Fedora 44 (pre-alpha) | RPM from the retained-only source kit | Builds in a pinned Fedora 44 image; package installation and the graphical session are unverified |

Anything else is best-effort. The flake is the definition; the deb and the rpm
follow from the same tree.

---

## The compositor: River 0.4 protocol, target-specific sources

helm does not ship its own compositor yet. It runs on **river 0.4.x** and *is*
river's window manager, driving it over `river-window-management-v1` (ADR 0013).
Two consequences you will meet immediately:

- **river 0.4 does no window management on its own.** Until `helm-wm` attaches,
  river places nothing — and serves no layer shell, so a bar would map nothing
  even if it were running. Today `helm-wm` does not exist, so a login applies the
  environment contract, finds no window manager, logs `FATAL WM-ABORT` and hands
  you back to the display manager. That is the expected state of 0.1.0, not a bug
  to report.
- **`river-window-management-v1` is declared *stable* as of river 0.4.0**, with
  a forward-compatibility pledge to 1.0.0. The residual risk is not a protocol
  classification but trust in a single maintainer of a pre-1.0 project. helm
  treats a version bump as an event to re-verify, not a surprise to absorb.
  Fedora 44 has an official `river >= 0.4.0` package candidate; package
  availability alone does not prove Helm runtime compatibility.



## NixOS and Nix

The flake lives at the repository root; the parts it imports live in
`packaging/nix/`.

```sh
git clone https://github.com/pipeliner/realms-de && cd realms-de

nix develop          # rust toolchain, river, yazi, btop, starship, fuzzel, foot
nix build            # the helm package
nix flake check      # shellcheck + package; the VM test needs /dev/kvm
```

> **`flake.lock` is committed.** It pins the Nix inputs used by this reference
> build. Refresh it only deliberately with `nix flake update` or `nix flake
> lock`, review the revision/hash diff, and run `nix flake check` before
> committing the update.

### As a NixOS module

```nix
{
  inputs.helm.url = "github:pipeliner/realms-de";

  outputs = { nixpkgs, helm, ... }: {
    nixosConfigurations.yourhost = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        helm.nixosModules.helm
        {
          programs.helm.enable = true;
          # programs.helm.compositor = pkgs.river;   # default: pinned river 0.4.x
          # programs.helm.paletteFile = ./my-palette.toml;
          # programs.helm.cursorTheme = "Adwaita";
          # programs.helm.cursorSize  = 24;
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
claiming to be helm. The session wrapper exports it per login.

For the user half — palette, generated configs, user units — add
`helm.homeManagerModules.helm` to your home-manager configuration and set
`programs.helm.enable = true` there too.

### What this actually installs today

- `bin/helmctl` — the theme CLI: `theme apply`, `theme lint`, and `theme diff`.
- `bin/helm-session` — the session entry, with river, `systemctl`,
  `dbus-update-activation-environment`, `dbus-run-session` and `gsettings` on
  its PATH.
- `share/wayland-sessions/helm.desktop` — the login entry. river's own package
  also provides a `river` entry; only the helm one applies helm's environment
  contract.
- `lib/systemd/user/helm-session.target`, `helm-wm.service`,
  `helm-bar.service`, `helm-session-abort.service` — installed together with the
  `helm-session.target.wants/` symlinks that make starting the target actually
  start something. The two client services fail because their binaries do not
  exist.
- `share/xdg-desktop-portal/helm-portals.conf`, and the equivalent as
  `xdg.portal.config.helm` — a named backend per interface.
- `share/helm/palette.toml`, `/etc/helm/palette.toml`.
- river 0.4.x, yazi, btop, starship, zsh, fuzzel, foot and slurp as runtime
  packages.

Not installed, because they are not written: `helm-wm`, `helm-bar`, any
generated theme.

`checks.session-boots` boots a NixOS VM and asserts the login entry, the
wrapper, the units, the palette and river's presence. The assertion it exists
for — the bar appears — is written but gated off, marked `PENDING M2` in
`packaging/nix/checks.nix`.

---

## Ubuntu 24.04 LTS and newer

There is no apt repository yet (`NEEDS-HUMAN` in `packaging/debian/control`:
PPA, self-hosted apt, or GitHub Releases). Build it yourself:

```sh
sudo apt install devscripts debhelper rustc-1.85 cargo-1.85 pkg-config python3 zstd
git clone https://github.com/pipeliner/realms-de && cd realms-de

packaging/tool-sources/build-native-source-kits.sh "$PWD/native-kits"
cd native-kits/helm-debian-0.1.0
dpkg-buildpackage -us -uc -b
sudo apt install ../helm_0.1.0_*.deb
```

The producer copies only Debian metadata, the staging/linkage helpers, and the
retained Helm workspace bundle into the package source directory. The checkout
is intake context for running that producer; it is not the package build input.
`debian/rules` rejects a full checkout before Cargo rather than treating it as
a second workspace authority.

Ubuntu 24.04's default rustc is 1.75, which is below helm's MSRV of 1.85. The
archive carries versioned toolchain packages — `rustc-1.85`/`cargo-1.85` are
there (1.85.1) — and `debian/rules` puts the newest one it finds on PATH,
failing with a clear message rather than building with the wrong compiler.
For a source-workspace build where those versioned packages are unavailable,
install the declared floor or newer with `rustup` instead; the MSRV comes from
the locked dependency graph, not Ubuntu's glibc.

**Three runtime dependencies are not in the Ubuntu 24.04 archive** (checked
against noble's package lists):

| Missing | Effect | What to do |
|---|---|---|
| `river` (any version) | No compositor — helm cannot start | Ubuntu only: provide a River `>= 0.4.0` package; its source is unresolved |
| `yazi` | charon (files) is missing | `cargo install --locked yazi-fm yazi-cli`, or a newer Ubuntu |
| `starship` | thoth's prompt falls back to plain zsh | `cargo install --locked starship` |

`fonts-ibm-plex` is in *multiverse*, so it is a Recommends rather than a
Depends: a hard dependency would make helm uninstallable on a box with only
main and universe enabled. Without it the bar renders in the system monospace
font — the glyph probe degrades rather than drawing tofu (ADR 0012), but it is
not the design's typeface. `sudo add-apt-repository multiverse && sudo apt
install fonts-ibm-plex` fixes it.

### What this actually installs today

`/usr/bin/helm-session`, `/usr/share/wayland-sessions/helm.desktop`, the four
user units under `/usr/lib/systemd/user/` **plus the
`helm-session.target.wants/` symlinks**, `/usr/share/xdg-desktop-portal/helm-portals.conf`
and `/usr/share/helm/palette.toml`, plus `/usr/bin/helmctl` with `theme apply`,
`theme lint`, and `theme diff`. `debian/rules` will also install `helm-wm` and
`helm-bar` automatically when they build.

---

## Fedora 44 (pre-alpha)

There is no Helm Fedora repository. The tracked RPM is pre-alpha and does not
install a working desktop. Fedora 44 has one pinned Cargo-smoke lane and one
pinned retained-source RPM-build lane; the latter builds the RPM from the
retained-only source kit without clean-installing it. Package installation and
a graphical session remain unverified. To investigate the package locally:

```sh
sudo dnf install rpm-build rust cargo systemd-rpm-macros make python3 zstd
git clone https://github.com/pipeliner/realms-de && cd realms-de

packaging/tool-sources/build-native-source-kits.sh "$PWD/native-kits"
rpmbuild -bb \
  --define "_sourcedir $PWD/native-kits" \
  "$PWD/native-kits/helm.spec"
sudo dnf install ~/rpmbuild/RPMS/*/helm-0.1.0-*.rpm
```

The resulting RPM `Source0` contains only Fedora metadata, the shared helpers,
and the same retained Helm bundle. `%prep` rejects a checkout-shaped Source0
and stages Cargo exclusively from the canonical inner `source.tar.gz`.

Fedora 44's official package listing reported `rust` and `cargo` 1.97.1 on
2026-08-29, above the current Rust 1.85 MSRV. Fedora repositories float, so
check `rustc --version` when building from source. On an older Fedora release,
use `rustup` to install Rust 1.85 or newer for a source-workspace build; do not
lower `rust-version` to match a distro toolchain, because the locked dependency
graph would still fail to parse. RPM builds remain governed by the toolchain
requirement in `packaging/fedora/helm.spec`.

**Name collision, settled:** Fedora ships a `helm` package for the Kubernetes
package manager, which owns `/usr/bin/helm` — and rpm refuses an install that
would collide rather than warning. helm's CLI therefore installs as `helmctl`,
and the window manager as `helm-wm`. The `%files` list is scoped so it can never
claim `/usr/bin/helm`.

### SELinux

helm is intended to be SELinux-clean and to require no custom labels or policy
module. The reasoning, so it can be checked rather than trusted: everything
installs into standard locations whose default labels in the targeted policy are
already correct (`/usr/bin` → `bin_t`, `/usr/lib/systemd/user` →
`systemd_unit_file_t`, `/usr/share` → `usr_t`); everything runs in the user's own
session domain; helm's control socket lives in `$XDG_RUNTIME_DIR/helm/ctl.sock`
inside `/run/user/$UID`; and nothing is setuid, has file capabilities, listens on
a port or runs as a system service.

**This has not been verified on a Fedora box in enforcing mode** — there is none
in the container this packaging was written in. The M3 Fedora CI job should run
`ausearch -m AVC -ts recent` after a session start and assert it comes back
empty. That is the guard that would fail if the claim ever stopped being true.

### What this actually installs today

The same set as the deb: `helmctl` with its three theme commands, session
entry, login entry, four user units and their `.wants` symlinks, portal policy,
and palette. The `%install` loop will also pick up `helm-wm` and `helm-bar`
when they build.

---

## Why there is a wrapper script at all

`helm-session` is not boilerplate. It implements the ordering contract in
ADR 0011, and the order is the entire point:

The numbered list below describes the currently shipped SPEC 0005 path. Draft
SPEC 0012 requires a crash-safe persistent session claim before step 2 and
mode-specific systemd/direct lifecycle teardown, but neither is shipped or
implementation authority yet.

1. Export `XDG_CURRENT_DESKTOP=helm`, `XDG_SESSION_TYPE=wayland`,
   `XDG_SESSION_DESKTOP=helm`, `XCURSOR_THEME`, `XCURSOR_SIZE` — **before** the
   compositor starts, because portal backend selection and cursor loading both
   read them at client start-up.
2. Start river — in the background, never `exec`: the entry outlives the
   compositor because it owns teardown.
3. **Wait** for the Wayland socket to actually appear, then for it to answer a
   round trip (`helmctl wait-display`, once that exists; until then the entry
   logs `DEGRADED NO-DISPLAY-PROBE` and settles for file existence). Importing
   early imports nothing, silently, and everything downstream inherits the hole.
   XWayland's `DISPLAY` is discovered the same way, by diffing
   `/tmp/.X11-unix/X*` across the compositor start.
4. Publish the environment to **both** `systemctl --user import-environment`
   *and* `dbus-update-activation-environment --systemd`. Two launchers, two
   environments; a service D-Bus activates later gets whatever was in the
   activation environment at that moment.
5. Mirror the cursor theme into `gsettings` (GTK reads that, not the
   environment).
6. Only now start `helm-session.target` — the window manager, then the bar —
   and then **verify each unit individually**. `systemctl start` exiting 0 is
   not a success signal: a unit whose `ConditionEnvironment=` is unmet is
   *skipped*, not failed, so the target activates, `systemctl --user --failed`
   is empty, and the desktop is inert. If the window manager is not `active`,
   the entry logs `FATAL WM-ABORT` and returns you to the display manager.

On exit the current pre-alpha script attempts to stop the target and clear both
environments so the next session does not inherit a `WAYLAND_DISPLAY` pointing
at a dead socket. This is not proof of candidate SPEC 0012 conformance: the
shipped units do not yet propagate Helm-target stop and the script does not yet
freeze launch admission or preserve/reconcile durable launch records and
leases.

Every degradation emits exactly one line with a stable code, so it can be
grepped, quoted in a bug report and looked up here:

```
<timestamp> helm-session: DEGRADED <CODE>: <what you are losing>
```

| Code | Means | You lose |
|---|---|---|
| `NO-SESSION-BUS` | No session bus, and `dbus-run-session` could not supply one (the entry re-execs under it once) | Portals, file dialogs, screen sharing |
| `NO-DBUS-ACTIVATION` | `dbus-update-activation-environment` missing | D-Bus-activated services never see the display: file dialogs hang ~25 s |
| `NO-SYSTEMD-USER` | No reachable `systemd --user` | Supervision; clients run under a bounded respawn loop instead |
| `NO-DISPLAY-PROBE` | `helmctl wait-display` not installed | Certainty that the compositor is dispatching, not merely bound |
| `NO-XWAYLAND` | No new X11 socket appeared | X11 applications; `DISPLAY` stays unset rather than being imported empty |
| `NO-GSETTINGS` | `gsettings` or its schemas missing | GTK apps use their own cursor |
| `NO-CURSOR-THEME` | The named theme is under no icon directory | The pointer is the default arrow |

Fatal conditions are `FATAL <CODE>` and exit: `NO-RUNTIME-DIR`, `NO-COMPOSITOR`,
`NO-SOCKET`, `WM-ABORT`. `HELM_STRICT=1` additionally makes a missing helm
binary fatal. The entry logs to
`${XDG_STATE_HOME:-~/.local/state}/helm/session.log`.

`PATH` is deliberately **not** imported into the user manager: it would replace
`PATH` for every user unit for the manager's lifetime, surviving logout on a
lingering user. `HELM_IMPORT_PATH=1` opts in, which is what a Nix profile the
user manager does not know about needs.

Preflight, without logging in:

```sh
helm-session --check
```

That is the stand-in for `helm ctl doctor` until the CLI exists.

---

## Troubleshooting

Each entry names the symptom, what is actually happening, how to check it by
hand today, and the `doctor` check that will diagnose it once `helm-ctl` lands
(M1–M2). The full failure register is [docs/PITFALLS.md](PITFALLS.md).

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
compositor's own environment has `WAYLAND_DISPLAY` and systemd or D-Bus does
not, the import ran too early or was skipped — that is the whole diagnosis,
every time.

*`helm ctl doctor` will check this in three separate places — the process,
systemd, and D-Bus — because they fail separately.*

### Screen sharing offers no sources, or produces nothing

Either `XDG_CURRENT_DESKTOP` was not `helm` when the portal started, so
`portals.conf` matching failed, or the chosen backend implements no ScreenCast.
`xdg-desktop-portal-gtk` — the default this packaging installs — does not
implement ScreenCast for wlroots-based compositors, and river 0.4 is
wlroots-based.

Installing `xdg-desktop-portal-wlr` alongside it is the likely fix and is
**unverified under river 0.4**; it is a `NEEDS-HUMAN` marked in
`packaging/nix/nixos-module.nix`. Screen sharing under helm should be treated as
untested until someone runs it on hardware.

### Tofu boxes instead of runes, or `𓂃` renders as a rectangle

The font stack does not cover helm's glyph inventory. helm's answer is a startup
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
glyph probe and ASCII fallbacks keep Helm legible when they are absent.

*`helm ctl doctor` will print the glyph coverage summary —
`helm-core::glyphs::Probe::summary()` already produces exactly that line.*

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

*`helm ctl doctor` will check the theme resolves *and* that gsettings agrees.*

### I select helm and land straight back at the login screen

Expected in 0.1.0, and the log says exactly why:

```sh
tail -20 ~/.local/state/helm/session.log
```

Look for `FATAL WM-ABORT: river is running but helm is not managing it`. river
0.4 does no window management by itself; `helm-wm` does, and it does not exist
until M2. The entry refuses to leave you on an inert compositor because, with no
layer shell, helm cannot draw an explanation onto it.

`HELM_ALLOW_NO_WM=1` keeps river running without a window manager — useful for
poking at the compositor, not a desktop. Once M2 lands, the same abort means the
window manager really failed, and `systemctl --user status helm-wm.service`
is the next stop; `helm-session-abort.service` is what turned its failure into a
returned login.

If the window manager exits **69**, another window-management client already
holds river's global — a leftover `helm-wm`, or one started by hand. river
answers `unavailable` to the second, so the unit deliberately does not restart:
`pgrep -a helm-wm` finds the holder.

### The bar never appears (M2 onwards)

Under river the bar is a layer-shell client served by *helm's own window
manager*, so a missing bar usually means the window manager is not serving
layer-shell — not that the bar is broken. Check `helm-wm.service` before
`helm-bar.service`. A crashed bar restarts on its own and never takes the
session down; a crashed window manager leaves river unmanaged, which is the
sharper failure.

```sh
systemctl --user status helm-wm.service helm-bar.service
systemctl --user list-dependencies helm-session.target
journalctl --user -u helm-bar.service -b
```

If `list-dependencies` shows the target with nothing under it, the package's
`helm-session.target.wants/` symlinks are missing — starting the target then
starts nothing at all and still exits 0.

### Running apps keep the old theme after `helm ctl theme apply`

This is expected: apply selects a sealed generation for future launches and
never rethemes an existing process. Test a newly launched program started
through a verified Helm launch profile. If that future launch still uses the
old generation, check the generation selection/profile integration before
debugging toolkit reload behavior.

User units started before the import and inherited an empty environment. Compare
the three views: the compositor's `/proc/<pid>/environ`, `systemctl --user
show-environment`, and what D-Bus hands an activated service. Any disagreement
is a separate session-startup bug; theme apply does not repair it.

---

## Open decisions (`NEEDS-HUMAN`)

These are recorded in the packaging files themselves, each with its options.
None of them block building from source today.

| Decision | Where | Options |
|---|---|---|
| How a River 0.4-compatible source is provided for the deb | `packaging/debian/rules` | Build from an upstream tag; package an upstream binary; require a PPA; raise the minimum Ubuntu version |
| Package hosting | `packaging/debian/control`, spec | PPA / self-hosted apt / GitHub Releases; COPR / dist-git / release tarballs |
| Maintainer identity | `packaging/debian/control`, `changelog`, spec, `SECURITY.md` | `vadim.evard@gmail.com` is the reachable package and private-security fallback contact |
| `helm` binary name on Fedora | `packaging/fedora/helm.spec` | `helm-ctl`; rename the package to `helm-de`; confirm no collision with Kubernetes helm |
| Idle and lock | `packaging/systemd/helm-session.target` | SPEC 0005 OQ-1: river 0.4 speaks `ext-session-lock-v1`, so the locker must too — gtklock (recommended), waylock (Zig), a new-enough swaylock, or `helm-ward` in M6. The idle defaults are a user-visible security decision and are not guessed |
| ScreenCast actually working under river | `configs/portal/helm-portals.conf` | SPEC 0005 OQ-2: whether river 0.4.8 exports `wlr-screencopy-unstable-v1` and whether xdpw works with external window management. Routed to `wlr`; **unverified**, and screen sharing stays unproven until someone tests it on hardware |
| How river is pinned in Nix | `packaging/nix/support.nix` | Pin via the nixpkgs input (current); or a tag-pinned input needing a second, human-produced `zigDeps` hash |
