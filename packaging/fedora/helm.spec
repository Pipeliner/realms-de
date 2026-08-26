# helm — Fedora 41+ spec (ARCHITECTURE.md §5).
#
# PRE-ALPHA (0.1.0). The Cargo workspace contains one crate, helm-core, and it
# is a library: this package installs NO helm binaries, because none exist yet.
# What it installs is the session contract — the wayland-session entry, the
# session wrapper that performs the ADR 0011 systemd/D-Bus environment
# handshake, the systemd user units and the palette. %install picks up
# helm-sessiond, helm-bar and helm automatically once M1-M2 build them.
#
# NEEDS-HUMAN (package name): Fedora ships a `helm` package for the Kubernetes
# package manager, which owns %{_bindir}/helm. helm's CLI is spelled
# `helm ctl doctor`, which wants that same path. This has NOT been verified
# against a live Fedora repo from the build container. Options, in rough order
# of preference: (a) ship the CLI as %{_bindir}/helm-ctl and provide `helm ctl`
# only through the session's own PATH; (b) name the package helm-de and the
# binary helm-de-ctl; (c) confirm the collision is not real and keep `helm`.
# Whoever owns distro submission decides — do not let a conflicting file land.
#
# NEEDS-HUMAN (Source0 / hosting): no download URL is invented here. Options:
# a GitHub release tarball once tags exist, a Fedora COPR building from git, or
# dist-git after a formal review. Until then, build from a local tarball:
#   git archive --format=tar.gz --prefix=helm-0.1.0/ -o ~/rpmbuild/SOURCES/helm-0.1.0.tar.gz HEAD
#
# NEEDS-HUMAN (vendored river): river 0.4.x is the compositor helm drives over
# river-window-management-v1, and it is NOT built by this spec. Fedora's own
# river package version was not verifiable from the build container; F41-era
# Fedora is likely to carry river 0.3.x or river-classic, neither of which
# implements the protocol. Options: (a) a separate helm-river spec building the
# upstream tag (https://codeberg.org/river/river) with zig 0.16 and wlroots
# 0.20 — this puts zig into the packaging pipeline, which is accepted, but no
# zig build invocation is written here because none has been verified;
# (b) require Fedora's river once it reaches 0.4.x; (c) ship a COPR of river
# 0.4.x. The Requires below accepts either name so the decision is not blocked
# by this file.

%global helm_summary Keyboard-first, gapless-tiling Wayland desktop environment

Name:           helm
Version:        0.1.0
Release:        1%{?dist}
Summary:        %{helm_summary}

# Dual-licensed, recipient's choice — the Rust ecosystem convention.
License:        MIT OR Apache-2.0
URL:            https://github.com/pipeliner/realms-de
Source0:        %{name}-%{version}.tar.gz

# helm's MSRV is 1.82 (Cargo.toml). Fedora 41 ships rust 1.82+, so plain
# `rust`/`cargo` suffice.
BuildRequires:  rust >= 1.82
BuildRequires:  cargo
BuildRequires:  systemd-rpm-macros
BuildRequires:  make
# NOTE ON THE RUST MACROS: a package destined for Fedora proper would use
# rust2rpm's %%cargo_prep/%%cargo_build/%%cargo_install with vendored sources,
# because Fedora builders have no network. This spec uses plain cargo with
# --locked so it can be built today from a git checkout; switching to the
# macros is mechanical and does not change anything below %%files.

# The compositor. Either the vendored, pinned build or a distro river new
# enough to speak river-window-management-v1 — 0.3.x and river-classic do not.
Requires:       (helm-river >= 0.4.0 or river >= 0.4.0)
# The two halves of the session handshake. Without dbus the activation
# environment cannot be updated and portals hang; without systemd the user
# units never start.
Requires:       dbus-common
Requires:       systemd
# A portal backend, or "Open File" silently does nothing in Firefox
# (docs/PITFALLS.md, "No portal backend installed").
Requires:       xdg-desktop-portal
Requires:       (xdg-desktop-portal-gtk or xdg-desktop-portal-wlr or xdg-desktop-portal-gnome)
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

THIS PACKAGE IS PRE-ALPHA AND DOES NOT INSTALL A WORKING DESKTOP. helm's
binaries are not written yet. What it installs is the session contract: the
wayland-session entry, the session wrapper that performs the systemd and D-Bus
environment handshake, the systemd user units and the palette. Logging into the
session gives you river with no window management attached.

%prep
%autosetup

%build
# --locked: build what Cargo.lock says, never silently resolve something else.
cargo build --release --locked --workspace

%install
install -Dpm0755 packaging/session/helm-session %{buildroot}%{_bindir}/helm-session
install -Dpm0644 packaging/session/helm.desktop %{buildroot}%{_datadir}/wayland-sessions/helm.desktop
install -Dpm0644 packaging/systemd/helm-session.target %{buildroot}%{_userunitdir}/helm-session.target
install -Dpm0644 packaging/systemd/helm-daemon.service %{buildroot}%{_userunitdir}/helm-daemon.service
install -Dpm0644 packaging/systemd/helm-bar.service %{buildroot}%{_userunitdir}/helm-bar.service
install -Dpm0644 palette.toml %{buildroot}%{_datadir}/helm/palette.toml

# Install whichever helm binaries this revision actually built. Today that is
# none of them; the loop means M1-M2 need no spec change.
for bin in helm helm-sessiond helm-bar; do
    if [ -x "target/release/${bin}" ]; then
        install -Dpm0755 "target/release/${bin}" "%{buildroot}%{_bindir}/${bin}"
    fi
done

%check
# helm-core's tests include the palette lint, so a palette that fails its WCAG
# floors fails the package build. That is deliberate (ADR 0005).
cargo test --release --locked --workspace

# The user units are wanted by helm-session.target, which the session wrapper
# starts; there is nothing to enable at the system level and no service to
# restart on upgrade. %%systemd_user_post/%%systemd_user_preun are therefore
# deliberately absent — add them only if a unit ever gains
# WantedBy=default.target.

%files
%license LICENSE-MIT LICENSE-APACHE
%doc docs/INSTALL.md docs/PITFALLS.md
%{_bindir}/helm-session
%{_datadir}/wayland-sessions/helm.desktop
%{_userunitdir}/helm-session.target
%{_userunitdir}/helm-daemon.service
%{_userunitdir}/helm-bar.service
%dir %{_datadir}/helm
%{_datadir}/helm/palette.toml
# Binaries land here once they exist (M1-M2). Listed as globs so the spec does
# not need editing, and %%files does not fail while they are absent:
%{_bindir}/helm*
# (the glob above covers helm-session too; it is listed explicitly for clarity)

# ── SELinux ───────────────────────────────────────────────────────────────────
# helm must be SELinux-clean and require no custom labels. Why that claim is
# plausible, stated so it can be argued with rather than trusted:
#
#   * Every file this package installs lands in a standard location whose
#     default label is already correct in the targeted policy: %{_bindir} is
#     bin_t, %{_userunitdir} is systemd_unit_file_t, %{_datadir} is usr_t. No
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
* Wed Aug 26 2026 helm contributors <helm-maintainers@helm.invalid> - 0.1.0-1
- Initial packaging skeleton: session entry, session wrapper, systemd user
  units and palette. No helm binaries yet (M1-M2).
- Depends on a vendored river 0.4.x; helm is river's window manager.
