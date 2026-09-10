# programs.realm — the NixOS module.
#
# Installs the session, registers the wayland-session entry, pulls in a portal
# backend, and makes sure both halves of the session-environment handshake
# (systemd --user and D-Bus activation) exist for the wrapper to use.
{ self, support }:
{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.programs.realm;

  # The compositor is chosen per host, so the launch command is built here
  # rather than baked into the package.
  launcher = pkgs.writeShellScript "realm-session-launch" ''
    export REALM_COMPOSITOR=${lib.getExe cfg.compositor}
    export REALM_CURSOR_THEME=${lib.escapeShellArg cfg.cursorTheme}
    export REALM_CURSOR_SIZE=${toString cfg.cursorSize}
    exec ${cfg.package}/bin/realm-session "$@"
  '';

  # `providedSessions` is what services.displayManager.sessionPackages checks;
  # without it NixOS refuses the package.
  sessionPackage =
    pkgs.runCommand "realm-session-entry" { passthru.providedSessions = [ "realm" ]; }
      ''
        install -Dm644 ${cfg.package}/share/wayland-sessions/realm.desktop \
          $out/share/wayland-sessions/realm.desktop
        substituteInPlace $out/share/wayland-sessions/realm.desktop \
          --replace-fail ${cfg.package}/bin/realm-session ${launcher}
      '';
in
{
  options.programs.realm = {
    enable = lib.mkEnableOption "the realm desktop environment";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "realm.packages.\${system}.default";
      description = "The realm package to install.";
    };

    compositor = lib.mkOption {
      type = lib.types.package;
      default = support.riverFor pkgs;
      defaultText = lib.literalExpression "pkgs.river (0.4.x, version-guarded)";
      description = ''
        The Wayland compositor realm runs on: river 0.4.x, driven by
        realm-session over river-window-management-v1 (ADR 0013).

        Point this elsewhere and you own the consequences — a compositor that
        does not implement river-window-management-v1 leaves realm with no way
        to place windows. The seam is real (realm-session talks to a WmBackend,
        not to river), but there is exactly one backend today.
      '';
    };

    paletteFile = lib.mkOption {
      type = lib.types.path;
      default = "${cfg.package}/share/realm/palette.toml";
      defaultText = lib.literalExpression "\${cfg.package}/share/realm/palette.toml";
      description = ''
        System-wide palette, installed to /etc/realm/palette.toml. Every themed
        surface is generated from this one file (ADR 0005); a user's own
        ~/.config/realm/palette.toml takes precedence.
      '';
    };

    cursorTheme = lib.mkOption {
      type = lib.types.str;
      default = "Adwaita";
      description = ''
        Cursor theme. Set in the session environment *and* in gsettings by the
        session wrapper, because a cursor theme set in only one of the two
        changes size as the pointer crosses windows (docs/PITFALLS.md, "Cursor
        theme unset").
      '';
    };

    cursorSize = lib.mkOption {
      type = lib.types.int;
      default = 24;
      description = "Cursor size in logical pixels.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      cfg.package
      cfg.compositor
      sessionPackage
    ]
    ++ [ pkgs.slurp ] # xdg-desktop-portal-wlr's default output chooser
    ++ support.reusedTools pkgs;

    # Registers realm in the display manager's session list. river's own package
    # also provides a "river" session entry; both will be offered, and only the
    # realm one applies realm's environment contract.
    services.displayManager.sessionPackages = [ sessionPackage ];

    # Picks up lib/systemd/user/*.{target,service} from the package, and the
    # realm-session.target.wants/ symlinks with them — [Install] alone would not
    # create those, and without them starting the session target starts nothing
    # and reports success (SPEC 0005 §4). checks.session-boots asserts the
    # symlink exists on the built system rather than trusting this.
    systemd.packages = [ cfg.package ];
    systemd.user.services.realm-wm.wantedBy = [ "realm-session.target" ];
    systemd.user.services.realm-bar.wantedBy = [ "realm-session.target" ];

    environment.etc."realm/palette.toml".source = cfg.paletteFile;

    # ── the session-environment handshake (ADR 0011) ────────────────────────
    #
    # The wrapper does the real work: it waits for WAYLAND_DISPLAY to exist,
    # then imports it into systemd --user *and* the D-Bus activation
    # environment before any client starts. What the module can add is making
    # sure both channels exist at all — a session bus, a systemd user manager,
    # and dconf so the gsettings half of the cursor contract has somewhere to
    # write.
    #
    # XDG_CURRENT_DESKTOP / XDG_SESSION_TYPE are deliberately NOT set in
    # environment.sessionVariables: they describe *this* session, and a machine
    # that also offers GNOME must not have every session claiming to be realm.
    # The wrapper exports them per session instead.
    services.dbus.enable = true;
    programs.dconf.enable = true;
    security.polkit.enable = true;
    hardware.graphics.enable = lib.mkDefault true;

    # Portals: a browser with no portal backend has a silently broken "Open
    # File" (docs/PITFALLS.md). A backend is named per interface rather than
    # left to whatever is installed, which is the same policy the deb and the
    # rpm get from configs/portal/realm-portals.conf — keep the two in step.
    #
    # gtk implements FileChooser and Settings but NOT ScreenCast on
    # wlroots-based compositors, so screen sharing is routed to wlr explicitly;
    # left to the default it offers no sources and reports no error.
    #
    # UNVERIFIED (SPEC 0005 OQ-2): whether river 0.4.8 still exports
    # wlr-screencopy-unstable-v1, and whether xdg-desktop-portal-wlr works when
    # window management lives outside the compositor. Screen sharing under realm
    # stays marked unverified in docs/INSTALL.md until that is tested on
    # hardware. slurp is xdpw's default output chooser.
    xdg.portal = {
      enable = true;
      extraPortals = [
        pkgs.xdg-desktop-portal-gtk
        pkgs.xdg-desktop-portal-wlr
      ];
      config.realm = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "wlr" ];
        "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
      };
      xdgOpenUsePortal = true;
    };

    # The glyph contract (ADR 0012). IBM Plex Mono is the design's face; the
    # IBM Plex Mono is Realm's only first-party font selection. ADR 0012 keeps
    # Symbola and Nerd Fonts optional: the startup probe falls back to ASCII
    # rather than making a third-party symbol font a mandatory system change.
    fonts.packages = [
      pkgs.ibm-plex
    ];

    # XWayland: old apps should look wrong-ish, not broken.
    programs.xwayland.enable = lib.mkDefault true;
  };
}
