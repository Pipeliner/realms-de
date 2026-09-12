# The realm derivation.
#
# PRE-ALPHA (0.1.0): `realmctl theme {apply,lint,diff}` and the workspace's
# metadata-only local validator are installed. This derivation also
# installs the session wrapper (wrapped so it can find river, systemctl,
# dbus-update-activation-environment and gsettings), the wayland-session entry,
# the systemd user units and the palette. realm-wm remains a pending M1–M2
# binary.
{
  pkgs,
  lib,
  support,
  src,
}:
(support.rustPlatformFor pkgs).buildRustPackage {
  pname = "realm";
  inherit (support) version;

  src = lib.cleanSourceWith {
    inherit src;
    # target/ and result/ are large and irrelevant; keeping them out keeps the
    # store path stable across local builds.
    filter =
      path: _type:
      !(builtins.elem (baseNameOf path) [
        "target"
        "result"
        ".direnv"
      ]);
  };

  cargoLock.lockFile = src + "/Cargo.lock";

  # This virtual workspace has no root package. Build every member explicitly
  # so postInstall can copy realmctl from Cargo's target-qualified output while
  # retaining the realm-sdd binary that is wrapped below.
  cargoBuildFlags = [ "--workspace" ];

  # The complete workspace test suite includes realm-sdd integration fixtures
  # and invokes Git to construct and inspect fixture repositories.  Git is a
  # check-time tool here, not an implicit runtime-closure decision for the
  # installed realm-sdd binary (tracked separately from this package gate).
  nativeBuildInputs = [
    pkgs.makeWrapper
    pkgs.git
  ];

  # realm-core's tests include the palette lint, so a palette that fails its
  # WCAG floors fails the build. That is the intended behaviour (ADR 0005).
  doCheck = true;

  postInstall = ''
    install -Dm755 target/${pkgs.stdenv.targetPlatform.rust.cargoShortTarget}/release/realmctl \
      $out/bin/realmctl
    install -Dm755 target/${pkgs.stdenv.targetPlatform.rust.cargoShortTarget}/release/realm-bar \
      $out/bin/realm-bar
    install -Dm755 ${src + "/packaging/session/realm-session"} $out/bin/realm-session

    # The desktop entry must point at the store path, not /usr/bin.
    install -Dm644 ${src + "/packaging/session/realm.desktop"} \
      $out/share/wayland-sessions/realm.desktop
    substituteInPlace $out/share/wayland-sessions/realm.desktop \
      --replace-fail /usr/bin/realm-session $out/bin/realm-session

    for unit in ${src + "/packaging/systemd"}/*; do
      install -Dm644 "$unit" $out/lib/systemd/user/"$(basename "$unit")"
    done

    # The .wants symlinks, shipped rather than left to [Install] processing.
    # Without them `systemctl --user start realm-session.target` starts nothing
    # at all and exits 0 (SPEC 0005 §4). NixOS's systemd.packages handling
    # propagates a package's *.wants directories; the VM test asserts the
    # resulting symlink exists rather than trusting that.
    mkdir -p $out/lib/systemd/user/realm-session.target.wants
    ln -s ../realm-wm.service \
      $out/lib/systemd/user/realm-session.target.wants/realm-wm.service
    ln -s ../realm-bar.service \
      $out/lib/systemd/user/realm-session.target.wants/realm-bar.service

    # The portal backend policy. On NixOS the module's xdg.portal.config says
    # the same thing; this copy is what makes the package correct on
    # nix-on-non-NixOS, where /usr/share is not ours to write.
    install -Dm644 ${src + "/configs/portal/realm-portals.conf"} \
      $out/share/xdg-desktop-portal/realm-portals.conf
    # The units name /usr/bin paths that do not exist on NixOS. Rewritten here
    # rather than in the unit files so the deb and the rpm keep working
    # unchanged.
    substituteInPlace $out/lib/systemd/user/*.service \
      --replace-quiet /usr/bin/ $out/bin/

    install -Dm644 ${src + "/palette.toml"} $out/share/realm/palette.toml

    wrapProgram $out/bin/realm-session \
      --prefix PATH : ${lib.makeBinPath (support.wrapperRuntime pkgs)}

    # SPEC 0010: realm-sdd reads local Git objects. Keep Git out of the desktop
    # wrapper path while making the installed validator independent of caller
    # PATH inheritance.
    wrapProgram $out/bin/realm-sdd \
      --prefix PATH : ${lib.makeBinPath [ pkgs.git ]}
  '';

  meta = {
    description = "Keyboard-first, gapless-tiling Wayland desktop environment";
    homepage = "https://github.com/pipeliner/realms-de";
    license = with lib.licenses; [
      mit
      asl20
    ];
    platforms = lib.platforms.linux;
    mainProgram = "realm-session";
  };
}
