# realm — Fedora 44 pre-alpha spec (ADR 0015 / SPEC 0009).
#
# PRE-ALPHA (0.1.0). The Cargo workspace builds realmctl, realm-wm and realm-bar;
# retained tool bundles build private Yazi, ya and Starship executables. This
# package also installs the session contract — the
# wayland-session entry, the
# session wrapper that performs the ADR 0011 systemd/D-Bus environment
# handshake, the systemd user units and the palette. %install requires every
# Realm runtime binary; absence is a package-build failure.
#
# BINARY NAMES ARE SETTLED (SPEC 0006 / SPEC 0025). Realm's CLI installs as
# `realmctl`, and the window manager and session daemon install as `realm-wm`
# (the crate is realm-session, but that name already belongs to the session entry
# script the display manager runs).
#
# Source0 is a packaging kit, not a second Realm workspace archive. It contains
# this packaging metadata, the staging/linkage helpers, and the retained
# three selected bundle authorities. %%prep validates and unpacks each canonical
# inner source.tar.gz; no checkout or archive-generation command is part of this
# package path.
#
# Fedora 44's official repositories resolved river-0.4.8-1.fc44 during the
# 2026-08-29 review. That dated package observation justifies the native
# protocol-generation floor below; it is not evidence that a Realm graphical
# session has run successfully with that package.

%global realm_summary Keyboard-first, gapless-tiling Wayland desktop environment

Name:           realm
Version:        0.1.0
Release:        1%{?dist}
Summary:        %{realm_summary}

# Dual-licensed, recipient's choice — the Rust ecosystem convention.
License:        MIT OR Apache-2.0
URL:            https://github.com/pipeliner/realms-de
Source0:        %{name}-%{version}.tar.gz

%global realm_bundle %{_builddir}/%{name}-%{version}/packaging/tool-sources/bundles/realm-workspace
%global realm_stage %{_builddir}/%{name}-%{version}/.realm-workspace
%global realm_source %{realm_stage}/source
%global realm_cargo_home %{realm_stage}/.cargo
%global realm_target_dir %{_builddir}/%{name}-%{version}/.cargo-target
%global tool_stager %{_builddir}/%{name}-%{version}/packaging/tool-sources/stage-tool-bundle.py
%global tool_runtime %{_builddir}/%{name}-%{version}/packaging/tool-sources/test-tool-runtime.py
%global yazi_bundle %{_builddir}/%{name}-%{version}/packaging/tool-sources/bundles/yazi-25.4.8
%global yazi_stage %{_builddir}/%{name}-%{version}/.yazi-25.4.8
%global yazi_source %{yazi_stage}/source
%global yazi_cargo_home %{yazi_stage}/.cargo
%global yazi_target_dir %{_builddir}/%{name}-%{version}/.yazi-target
%global starship_bundle %{_builddir}/%{name}-%{version}/packaging/tool-sources/bundles/starship-1.23.0
%global starship_stage %{_builddir}/%{name}-%{version}/.starship-1.23.0
%global starship_source %{starship_stage}/source
%global starship_cargo_home %{starship_stage}/.cargo
%global starship_target_dir %{_builddir}/%{name}-%{version}/.starship-target

# realm's MSRV is 1.89 (Cargo.toml). The BuildRequires below is the mechanical
# check: dnf refuses the build rather than failing halfway through cargo if the
# shipped Rust compiler is older.
BuildRequires:  rust >= 1.89
BuildRequires:  cargo
BuildRequires:  dejavu-sans-fonts
BuildRequires:  dejavu-sans-mono-fonts
BuildRequires:  systemd-rpm-macros
BuildRequires:  make
BuildRequires:  python3
BuildRequires:  zstd

# Fedora's native compositor candidate. The lower bound is the
# river-window-management-v1 generation boundary, not a tested-session claim.
Requires:       river >= 0.4.0
# The two halves of the session handshake. Without dbus the activation
# environment cannot be updated and portals hang; without systemd the user
# units never start.
Requires:       dbus-common
Requires:       systemd
Requires:       /usr/bin/xdg-settings
Requires:       /usr/bin/gtk-launch
# A portal backend, or "Open File" silently does nothing in Firefox
# (docs/PITFALLS.md, "No portal backend installed").
# Named backends, not a disjunction: a solver may satisfy `gtk or wlr or gnome`
# with a backend that implements no ScreenCast, and the user meets that as screen
# sharing that silently produces nothing (SPEC 0005 §5). gtk answers FileChooser
# and Settings; wlr answers ScreenCast and Screenshot, which is what
# configs/portal/realm-portals.conf routes to it.
Requires:       xdg-desktop-portal
Requires:       xdg-desktop-portal-gtk
Requires:       xdg-desktop-portal-wlr
# xdg-desktop-portal-wlr's default `simple` chooser shells out to slurp for
# output selection.
Recommends:     slurp
# The tools realm reuses rather than rewrites (ADR 0007).
Requires:       foot
Requires:       fuzzel
Requires:       btop
Requires:       zsh
# The glyph contract (ADR 0012): symbol coverage is optional and guarded by
# the startup probe; neither font package is a hard runtime dependency.
Recommends:     google-noto-sans-symbols2-fonts
Recommends:     ibm-plex-mono-fonts
Recommends:     gsettings-desktop-schemas
Recommends:     xorg-x11-server-Xwayland
ExclusiveArch:  %{rust_arches}

%description
realm is a Wayland desktop built around one idea: an ordered ledger of windows is
the only state that matters, and every layout is a pure projection of it. Undo
is exact, focus never moves a rectangle, and nothing animates.

It reuses proven tools rather than rewriting them — yazi, btop, zsh with
starship, fuzzel and foot — and themes all of them from a single palette file.
The compositor is river 0.4, driven by realm's own window manager over
river-window-management-v1.

THIS PACKAGE IS PRE-ALPHA AND DOES NOT INSTALL A WORKING DESKTOP. Its recipe
requires realmctl, realm-wm and realm-bar plus the session contract. CI installs
the exact RPM into an empty Fedora 44 root with normal dependency resolution
and runs the installed realmctl/palette and River version probes. Graphical
login, portal, River session compatibility and SELinux runtime behaviour on
Fedora remain unverified.

%prep
%autosetup
python3 packaging/tool-sources/check-native-source-kit.py rpm \
    %{_builddir}/%{name}-%{version}
rm -rf %{realm_stage} %{realm_target_dir} \
    %{yazi_stage} %{yazi_target_dir} %{starship_stage} %{starship_target_dir}
mkdir -p %{realm_target_dir} %{yazi_target_dir} %{starship_target_dir}
python3 packaging/tool-sources/stage-realm-workspace.py \
    %{realm_bundle} %{realm_stage}
python3 %{tool_stager} %{yazi_bundle} %{yazi_stage}
python3 %{tool_stager} %{starship_bundle} %{starship_stage}
sh packaging/fedora/normalize-source-modes.sh \
    %{realm_stage} %{yazi_stage} %{starship_stage}

%build
cd %{realm_source}
CARGO_HOME=%{realm_cargo_home} CARGO_TARGET_DIR=%{realm_target_dir} \
    cargo build --release --frozen --offline --locked --workspace
cd %{yazi_source}
CARGO_HOME=%{yazi_cargo_home} CARGO_TARGET_DIR=%{yazi_target_dir} \
CFLAGS="$CFLAGS -std=gnu17" \
SOURCE_DATE_EPOCH=1744112829 \
VERGEN_GIT_SHA=99ea3b74c4260a724b43af812df0f68ef59395b7 \
VERGEN_GIT_COMMIT_DATE=2025-04-08 VERGEN_BUILD_DATE=2025-04-08 \
    cargo build --release --frozen --offline --locked \
        --package yazi-fm --package yazi-cli
cd %{starship_source}
CARGO_HOME=%{starship_cargo_home} CARGO_TARGET_DIR=%{starship_target_dir} \
    cargo build --release --frozen --offline --locked --bin starship

%install
cd %{realm_source}
install -Dpm0755 packaging/session/realm-session %{buildroot}%{_bindir}/realm-session
install -Dpm0755 packaging/session/realm-browser %{buildroot}%{_bindir}/realm-browser
install -Dpm0644 packaging/session/realm.desktop %{buildroot}%{_datadir}/wayland-sessions/realm.desktop
install -Dpm0644 packaging/systemd/realm-session.target %{buildroot}%{_userunitdir}/realm-session.target
install -Dpm0644 packaging/systemd/realm-wm.service %{buildroot}%{_userunitdir}/realm-wm.service
install -Dpm0644 packaging/systemd/realm-bar.service %{buildroot}%{_userunitdir}/realm-bar.service
install -Dpm0644 packaging/systemd/realm-session-abort.service %{buildroot}%{_userunitdir}/realm-session-abort.service
install -Dpm0644 palette.toml %{buildroot}%{_datadir}/realm/palette.toml
# The portal backend policy (SPEC 0005 §5).
install -Dpm0644 configs/portal/realm-portals.conf %{buildroot}%{_datadir}/xdg-desktop-portal/realm-portals.conf

# The .wants symlinks, shipped rather than left to [Install] processing.
# [Install] only takes effect when something runs it — `systemctl --user
# enable`, dh_installsystemduser, or an rpm preset — and rpm has no preset
# mechanism for *user* units that fires for a session target. Without these
# symlinks, `systemctl --user start realm-session.target` starts nothing at all
# and exits 0, which is the hardest kind of failure to diagnose (SPEC 0005 §4).
install -dm0755 %{buildroot}%{_userunitdir}/realm-session.target.wants
ln -sf ../realm-wm.service %{buildroot}%{_userunitdir}/realm-session.target.wants/realm-wm.service
ln -sf ../realm-bar.service %{buildroot}%{_userunitdir}/realm-session.target.wants/realm-bar.service

# Install the complete runtime payload and record it in the RPM file list. A
# missing output makes install fail rather than producing a partial package.
echo "%{_bindir}/realm-session" >%{_builddir}/realm-binaries.list
for bin in realmctl realm-wm realm-bar; do
    install -Dpm0755 "%{realm_target_dir}/release/${bin}" "%{buildroot}%{_bindir}/${bin}"
    echo "%{_bindir}/${bin}" >>%{_builddir}/realm-binaries.list
done
echo "%dir %{_prefix}/lib/realm" >>%{_builddir}/realm-binaries.list
echo "%dir %{_prefix}/lib/realm/bin" >>%{_builddir}/realm-binaries.list
for bin in yazi ya; do
    install -Dpm0755 "%{yazi_target_dir}/release/${bin}" \
        "%{buildroot}%{_prefix}/lib/realm/bin/${bin}"
    echo "%{_prefix}/lib/realm/bin/${bin}" >>%{_builddir}/realm-binaries.list
done
install -Dpm0755 "%{starship_target_dir}/release/starship" \
    "%{buildroot}%{_prefix}/lib/realm/bin/starship"
echo "%{_prefix}/lib/realm/bin/starship" >>%{_builddir}/realm-binaries.list

%check
# realm-core's tests include the palette lint, so a palette that fails its WCAG
# floors fails the package build. That is deliberate (ADR 0005).
# realm-agent-sdd is not packaged here and its gate fixtures require live Git
# worktree state, which the canonical source archive deliberately omits.
python3 %{_builddir}/%{name}-%{version}/packaging/tool-sources/stage-realm-workspace.py \
    %{realm_bundle} %{realm_stage}
cd %{realm_source}
CARGO_HOME=%{realm_cargo_home} CARGO_TARGET_DIR=%{realm_target_dir} \
    cargo test --release --frozen --offline --locked --workspace \
        --exclude realm-agent-sdd
./packaging/tool-sources/test-tool-configs.sh
python3 %{tool_runtime} \
    %{realm_target_dir}/release/realmctl \
    %{yazi_target_dir}/release/yazi %{yazi_target_dir}/release/ya \
    %{starship_target_dir}/release/starship %{realm_source}

# No %%systemd_user_post/%%systemd_user_preun. Those macros enable units named
# in %%{_userunitdir} for *new* user sessions via presets, and realm's units must
# not be preset-enabled: realm-session.target has no [Install] section on purpose
# (enabling it would start it at boot for a lingering user, with no display, and
# every unit under it would be silently condition-skipped). The .wants symlinks
# shipped in %%install are what make the target start anything, and they need no
# scriptlet.

# -f: the session entry and mandatory three-binary runtime payload recorded
# during %%install.
%files -f %{_builddir}/realm-binaries.list
%{_bindir}/realm-browser
%license .realm-workspace/source/LICENSE-MIT .realm-workspace/source/LICENSE-APACHE
%doc packaging/package-docs/INSTALL.md .realm-workspace/source/docs/PITFALLS.md
%{_datadir}/wayland-sessions/realm.desktop
%{_userunitdir}/realm-session.target
%{_userunitdir}/realm-wm.service
%{_userunitdir}/realm-bar.service
%{_userunitdir}/realm-session-abort.service
%dir %{_userunitdir}/realm-session.target.wants
%{_userunitdir}/realm-session.target.wants/realm-wm.service
%{_userunitdir}/realm-session.target.wants/realm-bar.service
%dir %{_datadir}/xdg-desktop-portal
%{_datadir}/xdg-desktop-portal/realm-portals.conf
%dir %{_datadir}/realm
%{_datadir}/realm/palette.toml
# ── SELinux ───────────────────────────────────────────────────────────────────
# realm must be SELinux-clean and require no custom labels. Why that claim is
# plausible, stated so it can be argued with rather than trusted:
#
#   * Every file this package installs lands in a standard location whose
#     default label is already correct in the targeted policy: %%{_bindir} is
#     bin_t, %%{_userunitdir} is systemd_unit_file_t, %%{_datadir} is usr_t. No
#     restorecon rule, no semanage fcontext, no policy module.
#   * Everything realm runs, runs in the user's own session domain (unconfined_t
#     on a default Fedora desktop, or user_t under a confined login). A
#     compositor, a window manager, a bar and a CLI are ordinary user programs:
#     they open a Wayland socket, talk to the session bus, and write under
#     $XDG_RUNTIME_DIR and $XDG_CONFIG_HOME, all of which the user's domain may
#     already do.
#   * realm's control socket is $XDG_RUNTIME_DIR/realm/ctl.sock — inside
#     /run/user/UID, which is user_runtime_t and owned by the same user. No
#     system service, no privileged helper, no setuid binary, no port.
#   * realm asks for no capability: no CAP_SYS_ADMIN, no device access beyond
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
* Sat Aug 29 2026 realm contributors <vadim.evard@gmail.com> - 0.1.0-1
- Correct the pre-alpha baseline to Fedora 44 and use Fedora's native
  river >= 0.4.0 candidate; runtime/session compatibility remains unverified.

* Wed Aug 26 2026 realm contributors <vadim.evard@gmail.com> - 0.1.0-1
- Initial packaging skeleton: session entry, systemd user units, portal policy
  and palette. No realm binaries yet (M1-M2).
- Depends on a vendored river 0.4.x; realm is river's window manager.
- Ships the realm-session.target.wants symlinks explicitly: [Install] alone does
  not create them for user units on rpm (SPEC 0005 section 4).
