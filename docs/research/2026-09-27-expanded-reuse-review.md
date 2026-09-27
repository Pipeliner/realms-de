# Expanded reuse review

2026-09-27; main inspected at `5198cbd`. Research, not a runtime change or
accepted replacement decision. Extends [the initial review](2026-09-27-reuse-first-mvp.md).
Three independent reviews covered lifecycle, desktop utilities, and packaging;
the main review checked theme implementations and cross-cutting compatibility.
No tools were installed and no packages were built locally.

## Highest-value changes to the shortlist

| Priority | Candidate | Work it could remove | Decision boundary |
|---|---|---|---|
| First investigation | Upstream Yazi APT packages | Private Yazi compilation on Ubuntu | Exact package/config/dependency proof in Noble CI; accepted source-policy amendment first |
| First code reuse | Existing NixOS VM tests and desktop modules | Re-discovery of PAM, lifecycle and portal test setup | Borrow focused fixtures/patterns, retain Realm-specific assertions |
| Focused experiment | app2unit | Bespoke desktop-entry-to-systemd launch plumbing | Verify installed fuzzel version, configuration propagation and logout behavior |
| Reuse selectively | Stylix GTK/Qt/btop modules; upstream theme templates | Missing configuration roles and integration knowledge | Keep Realm palette and user-file ownership; do not adopt global configuration writes |
| Conditional daily-use gaps | Existing notifications/audio/network/authentication clients | Entire commodity UIs | Add only for an observed/accepted user journey; do not create new launch gates |
| Existing bounded evaluation | UWSM and Waybar | Outer lifecycle or commodity bar modules | Must replace responsibilities, not add a second supervisor/bar |

These priorities are engineering judgments, not measured time savings. Current
upstream documentation is not proof of compatibility with Realm's pinned or
distro packages. Source reuse requires exact revision, file-level licensing,
attribution and tests; using a separately packaged executable is a different
integration from copying its source into Realm's MIT/Apache-2.0 crates.

## Session and application lifecycle

[UWSM](https://github.com/Vladimir-csp/uwsm) supplies an outer systemd session
lifecycle, readiness finalization and application launch support. Its documented
MIT license and generic compositor mode make it a candidate, not a River-ready
drop-in. Replace existing lifecycle ownership if adopted; do not run both.
Native Ubuntu 24.04/Fedora 44 package availability was not established here.

The concrete reusable [NixOS UWSM module](https://raw.githubusercontent.com/NixOS/nixpkgs/master/nixos/modules/programs/wayland/uwsm.nix)
contains desktop-entry registration and service integration, including
`restartIfChanged=false` and `enableDefaultPath=false`. These prevent rebuild
and environment surprises worth checking in Realm. It also selects dbus-broker;
that is a host integration decision, not harmless glue. Nixpkgs is MIT-licensed;
check the exact pinned file before copying.

[app2unit](https://github.com/Vladimir-csp/app2unit) is a narrower reuse candidate:
launch desktop entries/commands in user units without adopting all of UWSM.
Its fuzzel integration documents a modern metadata path and an older-version
compatibility mode. Test the actual target version, inherited theme selection,
launch failures and logout cleanup. The repository identifies GPL-3.0; prefer
the standalone tool to copying its implementation into differently licensed code.

[sway-systemd](https://github.com/alebastr/sway-systemd) offers MIT-licensed target
and shutdown/autostart patterns. Its Sway IPC and keyboard configuration assumptions
make wholesale adoption unsuitable. Reuse specific lifecycle tests/patterns,
not Sway commands or automatic keyboard policy.

**Verified incompatibility:** [Home Manager's River module](https://raw.githubusercontent.com/nix-community/home-manager/master/modules/services/window-managers/river.nix)
defaults to `river-classic` and generates legacy `riverctl` configuration.
It is not Realm's River 0.4 integration. Its independent session target would
also duplicate lifecycle ownership. Module names alone are insufficient evidence.

[systemd's desktop integration document](https://systemd.io/DESKTOP_ENVIRONMENTS/)
explicitly assumes one graphical session per user because activation state is
shared. The owner's single-session choice avoids needing same-user Realm
multi-session support. Concurrent other desktops under the same UID are also
not isolated merely by giving Realm a distinct target. Preserve other desktops'
persistent files and do not promise simultaneous shared-bus isolation.
Reuse systemd's XDG autostart generator if autostart is required, rather than
writing a desktop-file scanner; it has ordering limits for essential services.

## Theme engines versus reusable target knowledge

The immediate problem is consumer configuration and session selection, not
absence of a template renderer. Realm already has eight target templates.

- [Stylix GTK module](https://raw.githubusercontent.com/nix-community/stylix/master/modules/gtk/hm.nix):
  concrete GTK3/GTK4 CSS, font/cursor and Flatpak integration reference. Its
  global Flatpak overrides and managed user files conflict with blindly adopting
  it as a session-local installer. Reuse target knowledge, not those side effects.
- [Stylix Qt module](https://raw.githubusercontent.com/nix-community/stylix/master/modules/qt/hm.nix):
  concrete Qt5/Qt6 palette/dialog/font settings. Its recommended qtct path uses
  Kvantum, beyond Realm's current MVP cut. It even conditions auto-enablement on
  NixOS due to a documented host-systemd issue; don't assume cross-distro safety.
- [Stylix btop module](https://raw.githubusercontent.com/nix-community/stylix/master/modules/btop/hm.nix):
  a compact complete role mapping useful for comparing Realm's generated config.
  It does not solve package version or actual consumer loading.
- [Tinted Builder](https://github.com/tinted-theming/tinted-builder-rust) and
  [Tinty](https://github.com/tinted-theming/tinty): mature-looking template
  ecosystem candidates, but Base16/Base24 roles need an explicit mapping from
  Realm's semantic palette. Do not replace a functioning engine just to gain a
  second configuration convention. Exact source/template licenses were not
  successfully retrieved in this pass; no source import is cleared by this record.
- [Matugen](https://github.com/InioX/matugen) supports standalone templating and
  imported custom color data, not just wallpaper generation. Upstream states
  GPL-2.0-or-later for the program. Its [template catalogue](https://github.com/InioX/matugen-themes)
  is a useful coverage checklist; example live-reload hooks and user-config
  writes contradict Realm's next-login contract. Template licensing must be
  checked separately from the program before copying.
- [Wallust templates](https://explosion-mental.codeberg.page/wallust/templates/)
  are another reference set. No demonstrated benefit justifies adding another
  palette generator to this MVP. License/import clearance remains unverified.

Stylix source-license files could not be retrieved through the selected web
paths in this pass. Treat its code as a research reference until the actual
file/revision license is verified, not automatically as copyable MIT material.

## A concrete packaging opportunity

[Yazi's official installation guide](https://yazi-rs.github.io/docs/installation/)
now advertises an official stable/nightly APT repository with amd64/arm64 binary
builds. This changes the earlier assumption that Ubuntu necessarily needs our
private source build. It does **not** establish Noble runtime compatibility,
offline reproducibility or compatibility with Realm's pinned Yazi config.
Investigate a pinned stable package in CI and compare the work removed before
amending SPEC 0024 or deleting the existing source route.

[Starship's guide](https://starship.rs/guide/) still identifies Ubuntu package
availability from 25.04 and a Fedora COPR route. Neither is evidence of an
official Noble/Fedora 44 archive package. Do not generalize the Yazi discovery
to Starship or replace retained builds with an unpinned install script.

## Commodity desktop utilities: reuse programs, not new Realm UIs

These candidates address possible daily-use gaps; they are not automatically
new MVP gates. The desktop review found no configured notification/audio/network/
Bluetooth/clipboard client integration, and enabling the polkit daemon alone
does not supply an authentication dialog.

| Function | Existing program and source | Integration boundary / caveat |
|---|---|---|
| Notifications | [mako](https://github.com/emersion/mako), MIT | Small generated config; one D-Bus name owner; correct activation environment and session teardown; notifications must not steal focus |
| Audio | [wpctl](https://pipewire.pages.freedesktop.org/wireplumber/daemon/getting_started.html) and [pavucontrol](https://github.com/pulseaudio/pavucontrol) | Reuse key actions plus complete mixer UI; verify WirePlumber and Pulse compatibility, not merely PipeWire portal presence |
| Network | [nmtui](https://networkmanager.pages.freedesktop.org/NetworkManager/NetworkManager/nmtui.html), [nmcli](https://networkmanager.dev/docs/api/latest/nmcli.html), [nm-applet](https://github.com/GNOME/network-manager-applet) | Terminal UI can avoid adding a tray; GUI applet supplies credentials interaction; do not replace host network management just to add a menu |
| Bluetooth | [Blueman](https://github.com/blueman-project/blueman) | Existing BlueZ pairing/manager UI; conditional hardware support; check agent and session lifecycle rather than designing pairing widgets |
| Screenshot/clipboard | [grim](https://github.com/emersion/grim), [slurp](https://github.com/emersion/slurp), [wl-clipboard](https://github.com/bugaevc/wl-clipboard) | Reuse region capture/copy; handle cancellation; ordinary copy/paste needs no history daemon; Sway-focused-window recipes do not work unchanged |
| Permission dialogs | [lxqt-policykit](https://github.com/lxqt/lxqt-policykit) or the target distro's existing agent | Standalone cross-desktop authentication client; exactly one agent and a real prompt test; do not assume daemon enablement means working dialogs |

Grim/slurp identify MIT licenses; wl-clipboard identifies GPL-3.0-or-later.
Blueman has GPLv2/GPLv3-and-later components. Exact source headers for pavucontrol,
NetworkManager clients and lxqt-policykit were not fully verified; this review
recommends packaged execution, not unreviewed source copying.

River's split model still requires Realm's layer-shell implementation for
Waybar/mako/region overlays. Current [River source](https://github.com/riverwm/river/blob/main/river/Server.zig)
exposes capture/data-control globals, but that is not evidence for our exact
packaged revision. Test focus/workarea and capture on the actual CI session.
Keep [xdpw](https://github.com/emersion/xdg-desktop-portal-wlr) plus GTK routing;
a screenshot tool cannot substitute for browser portal acceptance.

The additional value of [Waybar's tray](https://github.com/Alexays/Waybar/wiki/Module:-Tray)
is reusing upstream network/Bluetooth applets, not just replacing meter drawing.
Keep Realm-specific state on its custom JSON boundary. Two important rejections:

- [wlr-which-key](https://github.com/MaxVerevkin/wlr-which-key) owns command/key
  dispatch; it is not a drop-in for Realm's passive, non-focus-taking grimoire.
  GPL-3.0 source also requires deliberate license handling.
- [SwayNotificationCenter](https://github.com/ErikReider/SwayNotificationCenter)
  could supply a control center, but its documented GTK-theme support limits
  increase verification cost. Prefer small mako integration unless persistent
  history/control-center behavior is a demonstrated requirement.

## Reusable packaging and test implementation patterns

[Fedora Sway configuration](https://gitlab.com/fedora/sigs/sway/sway-config-fedora)
is a useful integration catalogue, not a replacement desktop. The official
[Fedora 44 file inventory](https://packages.fedoraproject.org/pkgs/sway-config-fedora/sway-config-fedora/fedora-44.html)
lists `95-autostart-policykit-agent.conf`, `95-xdg-desktop-autostart.conf`,
`95-xdg-user-dirs.conf`, `60-bindings-media.conf`, `60-bindings-volume.conf`,
`/usr/libexec/sway/volume-helper` and `/usr/bin/start-sway`. Package metadata
reports MIT. Exact bodies were inaccessible here: retrieve a pinned source and
license before porting only the needed behavior; do not copy Sway commands.

Use existing Debian [dh_installsystemduser](https://manpages.debian.org/bookworm/debhelper/dh_installsystemduser.1.en.html)
for package lifecycle instead of writing maintainer-script generation. Realm
already uses debhelper: a new package generator is not a solution to the native
dependency closure or real-build timeout.

[Noble autopkgtest-virt-qemu](https://manpages.ubuntu.com/manpages/noble/man1/autopkgtest-virt-qemu.1.html)
offers overlays, serial access, reboot and shutdown handling. A bounded future
comparison could reuse `packaging/native-vm/guest-probe.sh` inside that driver;
do not throw away our acceptance probes. Fedora equivalence is unproven and a
harness migration is not needed just to get another framework.

Keep extending the existing [NixOS testing library](https://github.com/NixOS/nixpkgs/blob/master/nixos/doc/manual/development/writing-nixos-tests.section.md).
The upstream test registry includes `nixos/tests/sway.nix`; its body was not
retrieved, so no specific upstream lock assertion is claimed verified here.
Do not confuse NixOS VM evidence with native distro proof.
[Mock](https://github.com/rpm-software-management/mock) is a potential replacement
for bespoke RPM build-root mechanics, not for graphical acceptance. Exact
borrowed-file licenses for those test/build helpers need checking before import.

## Next experiments, attached to existing tasks

1. **#134/#75:** inspect the official stable Yazi APT package in existing Noble
   CI. Compare dependencies/config compatibility and retained-artifact policy.
   Amend source specifications only if this removes more work than it adds.
2. **#117/#135/#240/#241:** compare target configs with Stylix/template sources;
   retain Realm's working renderer and implement login-bound selection. Trial
   app2unit only if it replaces launch ownership code under that contract.
3. **#68:** inspect UWSM module patterns for rebuild/PATH/lifecycle correctness.
   Wholesale UWSM remains optional and requires one outer owner, not duplication.
4. **#43/#50/#51/#78:** include tray/applet reuse in the bounded bar comparison;
   identify real audio/network/notification gaps during the shared journey and
   use existing clients if accepted. Do not add a separate settings-suite project.
5. **#79/#69/#102:** finish existing locker/portal integration using upstream
   clients. Focus on installed runtime proof, not another library search.

## Adoption discipline

For each candidate, answer: which existing task/code disappears; exact target
version and license; configuration/ownership conflict; minimum CI experiment;
rollback. Reject candidates that add permanent parallel machinery. Preserve
Realm's accepted window semantics and use River's protocol boundary; tinyrwm
could not be fetched in this pass and is not declared a battle-tested replacement.
No new broad security-review gate is proposed; licensing, functionality and
dependency compatibility are ordinary adoption checks.
