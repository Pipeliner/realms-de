# programs.helm — the NixOS module.
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
  cfg = config.programs.helm;

  # The compositor is chosen per host, so the launch command is built here
  # rather than baked into the package.
  launcher = pkgs.writeShellScript "helm-session-launch" ''
    export HELM_COMPOSITOR=${lib.getExe cfg.compositor}
    export HELM_CURSOR_THEME=${lib.escapeShellArg cfg.cursorTheme}
    export HELM_CURSOR_SIZE=${toString cfg.cursorSize}
    exec ${cfg.package}/bin/helm-session "$@"
  '';

  # `providedSessions` is what services.displayManager.sessionPackages checks;
  # without it NixOS refuses the package.
  sessionPackage =
    pkgs.runCommand "helm-session-entry" { passthru.providedSessions = [ "helm" ]; }
      ''
        install -Dm644 ${cfg.package}/share/wayland-sessions/helm.desktop \
          $out/share/wayland-sessions/helm.desktop
        substituteInPlace $out/share/wayland-sessions/helm.desktop \
          --replace-fail ${cfg.package}/bin/helm-session ${launcher}
      '';
in
{
  options.programs.helm = {
    enable = lib.mkEnableOption "the helm desktop environment";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "helm.packages.\${system}.default";
      description = "The helm package to install.";
    };

    compositor = lib.mkOption {
      type = lib.types.package;
      default = support.riverFor pkgs;
      defaultText = lib.literalExpression "pkgs.river (0.4.x, version-guarded)";
      description = ''
        The Wayland compositor helm runs on: river 0.4.x, driven by
        helm-session over river-window-management-v1 (ADR 0013).

        Point this elsewhere and you own the consequences — a compositor that
        does not implement river-window-management-v1 leaves helm with no way
        to place windows. The seam is real (helm-session talks to a WmBackend,
        not to river), but there is exactly one backend today.
      '';
    };

    paletteFile = lib.mkOption {
      type = lib.types.path;
      default = "${cfg.package}/share/helm/palette.toml";
      defaultText = lib.literalExpression "\${cfg.package}/share/helm/palette.toml";
      description = ''
        System-wide palette, installed to /etc/helm/palette.toml. Every themed
        surface is generated from this one file (ADR 0005); a user's own
        ~/.config/helm/palette.toml takes precedence.
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

    # Registers helm in the display manager's session list. river's own package
    # also provides a "river" session entry; both will be offered, and only the
    # helm one applies helm's environment contract.
    services.displayManager.sessionPackages = [ sessionPackage ];

    # Picks up lib/systemd/user/*.{target,service} from the package, and the
    # helm-session.target.wants/ symlinks with them — [Install] alone would not
    # create those, and without them starting the session target starts nothing
    # and reports success (SPEC 0005 §4). checks.session-boots asserts the
    # symlink exists on the built system rather than trusting this.
    systemd.packages = [ cfg.package ];
    systemd.user.services.helm-wm.wantedBy = [ "helm-session.target" ];
    systemd.user.services.helm-bar.wantedBy = [ "helm-session.target" ];

    environment.etc."helm/palette.toml".source = cfg.paletteFile;

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
    # that also offers GNOME must not have every session claiming to be helm.
    # The wrapper exports them per session instead.
    services.dbus.enable = true;
    programs.dconf.enable = true;
    security.polkit.enable = true;
    hardware.graphics.enable = lib.mkDefault true;

    # Portals: a browser with no portal backend has a silently broken "Open
    # File" (docs/PITFALLS.md). A backend is named per interface rather than
    # left to whatever is installed, which is the same policy the deb and the
    # rpm get from configs/portal/helm-portals.conf — keep the two in step.
    #
    # gtk implements FileChooser and Settings but NOT ScreenCast on
    # wlroots-based compositors, so screen sharing is routed to wlr explicitly;
    # left to the default it offers no sources and reports no error.
    #
    # UNVERIFIED (SPEC 0005 OQ-2): whether river 0.4.8 still exports
    # wlr-screencopy-unstable-v1, and whether xdg-desktop-portal-wlr works when
    # window management lives outside the compositor. Screen sharing under helm
    # stays marked unverified in docs/INSTALL.md until that is tested on
    # hardware. slurp is xdpw's default output chooser.
    xdg.portal = {
      enable = true;
      extraPortals = [
        pkgs.xdg-desktop-portal-gtk
        pkgs.xdg-desktop-portal-wlr
      ];
      config.helm = {
        default = [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = [ "wlr" ];
        "org.freedesktop.impl.portal.Screenshot" = [ "wlr" ];
      };
      xdgOpenUsePortal = true;
    };

    # The glyph contract (ADR 0012). IBM Plex Mono is the design's face; the
    # symbols-only Nerd Font supplies runes and instrument glyphs. Without
    # these the bar is a row of tofu on first boot.
    #
    # On nixpkgs older than 24.11 the second attribute is
    # `(nerdfonts.override { fonts = [ "NerdFontsSymbolsOnly" ]; })`.
    fonts.packages = [
      pkgs.ibm-plex
      pkgs.nerd-fonts.symbols-only
    ];

    # XWayland: old apps should look wrong-ish, not broken.
    programs.xwayland.enable = lib.mkDefault true;
  };
}
