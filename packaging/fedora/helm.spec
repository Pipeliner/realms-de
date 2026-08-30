# helm — Fedora 44 pre-alpha spec (ADR 0015 / SPEC 0009).
#
# PRE-ALPHA (0.1.0). The Cargo workspace builds helmctl for the implemented
# theme commands. This package also installs the session contract — the
# wayland-session entry, the
# session wrapper that performs the ADR 0011 systemd/D-Bus environment
# handshake, the systemd user units and the palette. %install picks up
# helm-wm and helm-bar automatically once they build.
#
# BINARY NAMES ARE SETTLED (ARCHITECTURE.md, commit 4ddcc26). Fedora ships
# Kubernetes Helm as %%{_bindir}/helm, and two packages owning one path cannot
# coexist — rpm refuses the install rather than warning. So helm's CLI installs
# as `helmctl`, and the window manager and session daemon installs as `helm-wm`
# (the crate is helm-session, but that name already belongs to the session entry
# script the display manager runs). The %%files glob below is scoped so it can
# never claim %%{_bindir}/helm.
#
# NEEDS-HUMAN (Source0 / hosting): no download URL is invented here. Options:
# a GitHub release tarball once tags exist, a Fedora COPR building from git, or
# dist-git after a formal review. Until then, build from a local tarball:
#   git archive --format=tar.gz --prefix=helm-0.1.0/ -o ~/rpmbuild/SOURCES/helm-0.1.0.tar.gz HEAD
#
# Fedora 44's official repositories resolved river-0.4.8-1.fc44 during the
# 2026-08-29 review. That dated package observation justifies the native
# protocol-generation floor below; it is not evidence that a Helm graphical
# session has run successfully with that package.

%global helm_summary Keyboard-first, gapless-tiling Wayland desktop environment

Name:           helm
Version:        0.1.0
Release:        1%{?dist}
Summary:        %{helm_summary}

# Dual-licensed, recipient's choice — the Rust ecosystem convention.
License:        MIT OR Apache-2.0
URL:            https://github.com/pipeliner/realms-de
Source0:        %{name}-%{version}.tar.gz

# helm's MSRV is 1.85 (Cargo.toml). The BuildRequires below is the mechanical
# check: dnf refuses the build rather than failing halfway through cargo if the
# shipped Rust compiler is older.
BuildRequires:  rust >= 1.85
BuildRequires:  cargo
BuildRequires:  systemd-rpm-macros
BuildRequires:  make
# NOTE ON THE RUST MACROS: a package destined for Fedora proper would use
# rust2rpm's %%cargo_prep/%%cargo_build/%%cargo_install with vendored sources,
# because Fedora builders have no network. This spec uses plain cargo with
# --locked so it can be built today from a git checkout; switching to the
# macros is mechanical and does not change anything below %%files.

# Fedora's native compositor candidate. The lower bound is the
# river-window-management-v1 generation boundary, not a tested-session claim.
Requires:       river >= 0.4.0
# The two halves of the session handshake. Without dbus the activation
# environment cannot be updated and portals hang; without systemd the user
# units never start.
Requires:       dbus-common
Requires:       systemd
# A portal backend, or "Open File" silently does nothing in Firefox
# (docs/PITFALLS.md, "No portal backend installed").
# Named backends, not a disjunction: a solver may satisfy `gtk or wlr or gnome`
# with a backend that implements no ScreenCast, and the user meets that as screen
# sharing that silently produces nothing (SPEC 0005 §5). gtk answers FileChooser
# and Settings; wlr answers ScreenCast and Screenshot, which is what
# configs/portal/helm-portals.conf routes to it.
Requires:       xdg-desktop-portal
Requires:       xdg-desktop-portal-gtk
Requires:       xdg-desktop-portal-wlr
# xdg-desktop-portal-wlr's default `simple` chooser shells out to slurp for
# output selection.
Recommends:     slurp
# The tools helm reuses rather than rewrites (ADR 0007).
Requires:       foot
Requires:       fuzzel
Requires:       btop
Requires:       zsh
# The glyph contract (ADR 0012): runes, planetary symbols and one hieroglyph.
Requires:       google-noto-sans-symbols2-fonts
Recommends:     ibm-plex-mono-fonts
Recommends:     gsettings-desktop-schemas
Recommends:     xorg-x11-server-Xwayland
# charon and thoth. Recommends rather than Requires: their absence degrades two
# panes, it does not break the session, and their availability in Fedora was
# not verifiable from the build container.
Recommends:     yazi
Recommends:     starship

ExclusiveArch:  %{rust_arches}

%description
helm is a Wayland desktop built around one idea: an ordered ledger of windows is
the only state that matters, and every layout is a pure projection of it. Undo
is exact, focus never moves a rectangle, and nothing animates.

It reuses proven tools rather than rewriting them — yazi, btop, zsh with
starship, fuzzel and foot — and themes all of them from a single palette file.
The compositor is river 0.4, driven by helm's own window manager over
river-window-management-v1.

THIS PACKAGE IS PRE-ALPHA AND DOES NOT INSTALL A WORKING DESKTOP. It installs
helmctl with `theme apply`, `theme lint`, and `theme diff`, plus the session
contract: the wayland-session entry, the session wrapper that performs the
systemd and D-Bus environment handshake, the systemd user units and the
palette. Logging into the session gives you river with no window management
attached because helm-wm and helm-bar are not written yet.

%prep
%autosetup

%build
# --locked: build what Cargo.lock says, never silently resolve something else.
cargo build --release --locked --workspace

%install
install -Dpm0755 packaging/session/helm-session %{buildroot}%{_bindir}/helm-session
install -Dpm0644 packaging/session/helm.desktop %{buildroot}%{_datadir}/wayland-sessions/helm.desktop
install -Dpm0644 packaging/systemd/helm-session.target %{buildroot}%{_userunitdir}/helm-session.target
install -Dpm0644 packaging/systemd/helm-wm.service %{buildroot}%{_userunitdir}/helm-wm.service
install -Dpm0644 packaging/systemd/helm-bar.service %{buildroot}%{_userunitdir}/helm-bar.service
install -Dpm0644 packaging/systemd/helm-session-abort.service %{buildroot}%{_userunitdir}/helm-session-abort.service
install -Dpm0644 palette.toml %{buildroot}%{_datadir}/helm/palette.toml
# The portal backend policy (SPEC 0005 §5).
install -Dpm0644 configs/portal/helm-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/helm-portals.conf

# The .wants symlinks, shipped rather than left to [Install] processing.
# [Install] only takes effect when something runs it — `systemctl --user
# enable`, dh_installsystemduser, or an rpm preset — and rpm has no preset
# mechanism for *user* units that fires for a session target. Without these
# symlinks, `systemctl --user start helm-session.target` starts nothing at all
# and exits 0, which is the hardest kind of failure to diagnose (SPEC 0005 §4).
install -dm0755 %{buildroot}%{_userunitdir}/helm-session.target.wants
ln -sf ../helm-wm.service %{buildroot}%{_userunitdir}/helm-session.target.wants/helm-wm.service
ln -sf ../helm-bar.service %{buildroot}%{_userunitdir}/helm-session.target.wants/helm-bar.service

# Install whichever helm binaries this revision actually built, and record them
# in a generated file list. This revision builds helmctl.
#
# A generated list, rather than globs in %%files: rpmbuild treats a %%files glob
# that matches nothing as a hard error ("File not found: .../helm-wm*"), so
# globbing for binaries that do not exist yet fails the build. Verified the hard
# way — that is exactly how this spec first failed. The list means helm-wm and
# helm-bar need no spec change, and it can never accidentally claim
# %%{_bindir}/helm, which on
# Fedora belongs to Kubernetes Helm.
# The list starts with the session entry, which always exists — rpm rejects an
# *empty* -f list as firmly as it rejects a glob that matches nothing.
echo "%{_bindir}/helm-session" >%{_builddir}/helm-binaries.list
for bin in helmctl helm-wm helm-bar; do
    if [ -x "target/release/${bin}" ]; then
        install -Dpm0755 "target/release/${bin}" "%{buildroot}%{_bindir}/${bin}"
        echo "%{_bindir}/${bin}" >>%{_builddir}/helm-binaries.list
    fi
done

%check
# helm-core's tests include the palette lint, so a palette that fails its WCAG
# floors fails the package build. That is deliberate (ADR 0005).
cargo test --release --locked --workspace

# No %%systemd_user_post/%%systemd_user_preun. Those macros enable units named
# in %%{_userunitdir} for *new* user sessions via presets, and helm's units must
# not be preset-enabled: helm-session.target has no [Install] section on purpose
# (enabling it would start it at boot for a lingering user, with no display, and
# every unit under it would be silently condition-skipped). The .wants symlinks
# shipped in %%install are what make the target start anything, and they need no
# scriptlet.

# -f: every %%{_bindir} entry this revision actually installed, recorded during
# %%install — the session entry and helmctl today, plus helm-wm and helm-bar
# once they build, with no spec change.
%files -f %{_builddir}/helm-binaries.list
%license LICENSE-MIT LICENSE-APACHE
%doc docs/INSTALL.md docs/PITFALLS.md
%{_datadir}/wayland-sessions/helm.desktop
%{_userunitdir}/helm-session.target
%{_userunitdir}/helm-wm.service
%{_userunitdir}/helm-bar.service
%{_userunitdir}/helm-session-abort.service
%dir %{_userunitdir}/helm-session.target.wants
%{_userunitdir}/helm-session.target.wants/helm-wm.service
%{_userunitdir}/helm-session.target.wants/helm-bar.service
%dir %{_datadir}/xdg-desktop-portal
%{_datadir}/xdg-desktop-portal/helm-portals.conf
%dir %{_datadir}/helm
%{_datadir}/helm/palette.toml
# ── SELinux ───────────────────────────────────────────────────────────────────
# helm must be SELinux-clean and require no custom labels. Why that claim is
# plausible, stated so it can be argued with rather than trusted:
#
#   * Every file this package installs lands in a standard location whose
#     default label is already correct in the targeted policy: %%{_bindir} is
#     bin_t, %%{_userunitdir} is systemd_unit_file_t, %%{_datadir} is usr_t. No
#     restorecon rule, no semanage fcontext, no policy module.
#   * Everything helm runs, runs in the user's own session domain (unconfined_t
#     on a default Fedora desktop, or user_t under a confined login). A
#     compositor, a window manager, a bar and a CLI are ordinary user programs:
#     they open a Wayland socket, talk to the session bus, and write under
#     $XDG_RUNTIME_DIR and $XDG_CONFIG_HOME, all of which the user's domain may
#     already do.
#   * helm's control socket is $XDG_RUNTIME_DIR/helm/ctl.sock — inside
#     /run/user/UID, which is user_runtime_t and owned by the same user. No
#     system service, no privileged helper, no setuid binary, no port.
#   * helm asks for no capability: no CAP_SYS_ADMIN, no device access beyond
#     what the compositor (river, already packaged and labelled) requests.
#
# What would break the claim, and must be re-checked if it ever becomes true:
# a system-level unit, a setuid or file-capability binary, a socket outside
# /run/user, or reading another user's files.
#
# NOT VERIFIED: this reasoning has not been checked on a Fedora box in enforcing
# mode, because there is none in the build container and `restorecon`/`semanage`
# are unavailable there. The M3 CI job on Fedora should run `ausearch -m AVC` (or
# `ausearch -m AVC -ts recent`) after a session start and assert it is empty —
# that is the guard that would fail if this claim regressed.

%changelog
* Sat Aug 29 2026 helm contributors <vadim.evard@gmail.com> - 0.1.0-1
- Correct the pre-alpha baseline to Fedora 44 and use Fedora's native
  river >= 0.4.0 candidate; runtime/session compatibility remains unverified.

* Wed Aug 26 2026 helm contributors <vadim.evard@gmail.com> - 0.1.0-1
- Initial packaging skeleton: session entry, systemd user units, portal policy
  and palette. No helm binaries yet (M1-M2).
- Depends on a vendored river 0.4.x; helm is river's window manager.
- Ships the helm-session.target.wants symlinks explicitly: [Install] alone does
  not create them for user units on rpm (SPEC 0005 section 4).
