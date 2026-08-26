# Installing helm

> **helm 0.1.0 is pre-alpha and does not install a working desktop.** The
> binaries that make helm a desktop — the window manager, the bar, the CLI — are
> not written yet (M1–M2 in [docs/MVP.md](MVP.md)). What you can install today is
> the *session contract*: the login entry, the wrapper that performs the
> systemd/D-Bus environment handshake, the systemd user units and the palette.
> Every section below has a **What this actually installs today** block that
> says exactly what you get and what you do not.

helm targets three platforms as first-class, tested in CI
([ARCHITECTURE.md §5](ARCHITECTURE.md)):

| Platform | Delivery | State today |
|---|---|---|
| NixOS / Nix | flake: `packages.default`, `nixosModules.helm`, `homeManagerModules.helm` | Reference build. Evaluates; VM test asserts the contract, not the desktop |
| Ubuntu 24.04 LTS + | `.deb` from `packaging/debian/` | Builds; three runtime dependencies are not in the Ubuntu archive (below) |
| Fedora 41 + | `.rpm` from `packaging/fedora/helm.spec` | Builds; the vendored river is not written yet |

Anything else is best-effort. The flake is the definition; the deb and the rpm
follow from the same tree.

---

## The compositor: a vendored river 0.4.x

helm does not ship its own compositor yet. It runs on **river 0.4.x** and *is*
river's window manager, driving it over `river-window-management-v1` (ADR 0013).
Two consequences you will meet immediately:

- **river 0.4 does no window management on its own.** Until `helm-sessiond`
  attaches, river places nothing. Today `helm-sessiond` does not exist, so the
  helm session is a compositor with nothing driving it: a background, a cursor,
  and no way to open a window. That is the expected state of 0.1.0, not a bug
  to report.
- **`river-window-management-v1` is classified *unstable* in the Wayland
  registry.** river's maintainer states that window managers will not be broken,
  but the classification is what it is. That is why helm pins a tested river
  (0.4.8) rather than accepting whatever a distribution ships, and why the Nix
  build refuses a river outside 0.4.x instead of failing at runtime.

Distributions have not caught up: Ubuntu 24.04 ships no river at all, and
Fedora 41-era river is likely 0.3.x or river-classic, neither of which
implements the protocol. So helm packages a pinned river alongside itself. On
Nix that pin is the `nixpkgs` input; on Ubuntu and Fedora it is a `helm-river`
package that **has not been built yet** — see the `NEEDS-HUMAN` blocks in
`packaging/debian/rules` and `packaging/fedora/helm.spec`.

---

## NixOS and Nix

The flake lives at the repository root; the parts it imports live in
`packaging/nix/`.

```sh
git clone https://github.com/pipeliner/realms-de && cd realms-de

nix develop          # rust toolchain, river, yazi, btop, starship, fuzzel, foot
nix build            # the helm package
nix flake check      # shellcheck + package; the VM test needs /dev/kvm
```

> **No `flake.lock` is committed yet.** Without it the build is not reproducible,
> which is the whole claim ADR 0010 makes for the flake. Run `nix flake lock`
> once and commit the result. It could not be generated in the container this
> packaging was written in: `nix flake lock` resolves `github:` inputs through
> `api.github.com`, which that container's egress policy refuses.

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
pulls in a portal backend (`xdg-desktop-portal-gtk`), enables dconf, polkit and
XWayland, and installs the fonts the glyph inventory needs. It deliberately does
**not** set `XDG_CURRENT_DESKTOP` globally — that variable describes one
session, and a machine that also offers GNOME must not have every session
claiming to be helm. The session wrapper exports it per login.

For the user half — palette, generated configs, user units — add
`helm.homeManagerModules.helm` to your home-manager configuration and set
`programs.helm.enable = true` there too.

### What this actually installs today

- `bin/helm-session` — the session wrapper, with river, `systemctl`,
  `dbus-update-activation-environment` and `gsettings` on its PATH.
- `share/wayland-sessions/helm.desktop` — the login entry. river's own package
  also provides a `river` entry; only the helm one applies helm's environment
  contract.
- `lib/systemd/user/helm-session.target`, `helm-daemon.service`,
  `helm-bar.service` — installed, and the two services fail to start because
  their binaries do not exist.
- `share/helm/palette.toml`, `/etc/helm/palette.toml`.
- river 0.4.x, yazi, btop, starship, zsh, fuzzel and foot as runtime packages.

Not installed, because they are not written: `helm`, `helm-sessiond`,
`helm-bar`, any generated theme.

`checks.session-boots` boots a NixOS VM and asserts the login entry, the
wrapper, the units, the palette and river's presence. The assertion it exists
for — the bar appears — is written but gated off, marked `PENDING M2` in
`packaging/nix/checks.nix`.

---

## Ubuntu 24.04 LTS and newer

There is no apt repository yet (`NEEDS-HUMAN` in `packaging/debian/control`:
PPA, self-hosted apt, or GitHub Releases). Build it yourself:

```sh
sudo apt install devscripts debhelper rustc-1.82 cargo-1.82
git clone https://github.com/pipeliner/realms-de && cd realms-de

ln -s packaging/debian debian        # Debian tooling insists on ./debian
dpkg-buildpackage -us -uc -b
sudo apt install ../helm_0.1.0_*.deb
```

Ubuntu 24.04 ships rustc 1.75, which is below helm's MSRV of 1.82.
`noble-updates` carries `rustc-1.82`/`cargo-1.82`; `debian/rules` puts a
versioned toolchain on PATH and fails with a clear message rather than building
with the wrong compiler.

**Three runtime dependencies are not in the Ubuntu 24.04 archive** (checked
against noble's package lists):

| Missing | Effect | What to do |
|---|---|---|
| `river` (any version) | No compositor — helm cannot start | Build the vendored `helm-river`, when it exists |
| `yazi` | charon (files) is missing | `cargo install --locked yazi-fm yazi-cli`, or a newer Ubuntu |
| `starship` | thoth's prompt falls back to plain zsh | `cargo install --locked starship` |

`fonts-ibm-plex` is in *multiverse*, so it is a Recommends rather than a
Depends: a hard dependency would make helm uninstallable on a box with only
main and universe enabled. Without it the bar renders in the system monospace
font — the glyph probe degrades rather than drawing tofu (ADR 0012), but it is
not the design's typeface. `sudo add-apt-repository multiverse && sudo apt
install fonts-ibm-plex` fixes it.

### What this actually installs today

`/usr/bin/helm-session`, `/usr/share/wayland-sessions/helm.desktop`, the three
user units under `/usr/lib/systemd/user/`, and `/usr/share/helm/palette.toml`.
No `/usr/bin/helm`, no `helm-bar`, no `helm-sessiond` — `debian/rules` installs
those automatically once they build, and says so in the build log while they do
not.

---

## Fedora 41 and newer

No repository yet (`NEEDS-HUMAN` in the spec: COPR, dist-git, or release
tarballs). Build it yourself:

```sh
sudo dnf install rpm-build rust cargo systemd-rpm-macros
git clone https://github.com/pipeliner/realms-de && cd realms-de

git archive --format=tar.gz --prefix=helm-0.1.0/ -o ~/rpmbuild/SOURCES/helm-0.1.0.tar.gz HEAD
rpmbuild -bb packaging/fedora/helm.spec
sudo dnf install ~/rpmbuild/RPMS/*/helm-0.1.0-*.rpm
```

**Name collision, unresolved:** Fedora ships a `helm` package for the Kubernetes
package manager, which owns `/usr/bin/helm`. helm's CLI is spelled `helm ctl
doctor` and wants the same path. This is a `NEEDS-HUMAN` decision recorded at
the top of the spec, with options; it does not block the pre-alpha because no
CLI binary is built yet, and it must be settled before one is.

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

The same set as the deb: wrapper, session entry, three user units, palette. The
`%install` loop picks up `helm`, `helm-sessiond` and `helm-bar` once they build.

---

## Why there is a wrapper script at all

`helm-session` is not boilerplate. It implements the ordering contract in
ADR 0011, and the order is the entire point:

1. Export `XDG_CURRENT_DESKTOP=helm`, `XDG_SESSION_TYPE=wayland`,
   `XDG_SESSION_DESKTOP=helm`, `XCURSOR_THEME`, `XCURSOR_SIZE` — **before** the
   compositor starts, because portal backend selection and cursor loading both
   read them at client start-up.
2. Start river.
3. **Wait** for the Wayland socket to actually appear. Importing early imports
   nothing, silently, and everything downstream inherits the hole.
4. Publish the environment to **both** `systemctl --user import-environment`
   *and* `dbus-update-activation-environment --systemd`. Two launchers, two
   environments; a service D-Bus activates later gets whatever was in the
   activation environment at that moment.
5. Mirror the cursor theme into `gsettings` (GTK reads that, not the
   environment).
6. Only now start `helm-session.target` — the window manager, then the bar.

On exit it stops the target and clears both environments, so the next session
does not inherit a `WAYLAND_DISPLAY` pointing at a dead socket.

Missing systemd, missing D-Bus tooling and missing helm binaries are all logged
and survived rather than fatal, so a broken piece degrades the desktop instead
of replacing it with a black screen. `HELM_STRICT=1` makes missing helm binaries
fatal instead. The wrapper logs to `${XDG_STATE_HOME:-~/.local/state}/helm/session.log`.

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
`ibm-plex-mono-fonts` + `google-noto-sans-symbols2-fonts` (Fedora); the Nix
module installs IBM Plex and the symbols-only Nerd Font for you.

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

### I log in and get an empty screen with a cursor

Expected in 0.1.0. river 0.4 does no window management by itself, and
`helm-sessiond` — the window manager that drives it — does not exist yet. The
session log will say so:

```sh
tail -f ~/.local/state/helm/session.log
```

Look for `helm-sessiond is not installed`. Once M2 lands, the same symptom means
the window manager failed to attach, and `systemctl --user status
helm-daemon.service` is the next stop.

### The bar never appears (M2 onwards)

Under river the bar is a layer-shell client served by *helm's own window
manager*, so a missing bar usually means the window manager is not serving
layer-shell — not that the bar is broken. Check `helm-daemon.service` before
`helm-bar.service`. A crashed bar restarts on its own and never takes the
session down; a crashed window manager leaves river unmanaged, which is the
sharper failure.

```sh
systemctl --user status helm-daemon.service helm-bar.service
journalctl --user -u helm-bar.service -b
```

### Nothing is themed, though `helm ctl theme apply` works by hand

User units started before the import and inherited an empty environment. Compare
the three views: the compositor's `/proc/<pid>/environ`, `systemctl --user
show-environment`, and what D-Bus hands an activated service. Any disagreement
is the bug.

---

## Open decisions (`NEEDS-HUMAN`)

These are recorded in the packaging files themselves, each with its options.
None of them block building from source today.

| Decision | Where | Options |
|---|---|---|
| How a pinned river is vendored for deb/rpm | `packaging/debian/rules`, `packaging/fedora/helm.spec` | Build from the upstream tag with zig 0.16 + wlroots 0.20; package an upstream binary; require a PPA/COPR; raise the minimum distro version |
| Package hosting | `packaging/debian/control`, spec | PPA / self-hosted apt / GitHub Releases; COPR / dist-git / release tarballs |
| Maintainer identity | `packaging/debian/control`, `changelog`, spec | List address, `maintainers@` alias, or a named person. The current address uses the reserved `.invalid` domain so it cannot deliver anywhere |
| `helm` binary name on Fedora | `packaging/fedora/helm.spec` | `helm-ctl`; rename the package to `helm-de`; confirm no collision with Kubernetes helm |
| Idle and lock | `packaging/systemd/helm-session.target` | swayidle + swaylock (packaged on both targets); gtklock; a helm-native locker in M6 |
| ScreenCast portal backend | `packaging/nix/nixos-module.nix` | Add `xdg-desktop-portal-wlr` and route ScreenCast to it; wait for a river-aware backend |
| `helm-daemon` restart policy | `packaging/systemd/helm-daemon.service` | Keep `on-failure`; or `always` plus an explicit success exit status for a deliberate quit, once the daemon exists |
| How river is pinned in Nix | `packaging/nix/support.nix` | Pin via the nixpkgs input (current); or a tag-pinned input needing a second, human-produced `zigDeps` hash |
