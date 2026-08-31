# programs.helm — the home-manager module.
#
# User-level half of the desktop: the palette, the generated configs for the
# tools helm reuses, and the user units. The system half (session entry,
# portals, fonts) is packaging/nix/nixos-module.nix.
{ self, support }:
{
  config,
  pkgs,
  lib,
  ...
}:
let
  cfg = config.programs.helm;

  shipped = name: self + "/configs/${name}";

  # configs/ is filled in by M1's theming pipeline. Link only what is actually
  # there, so a fresh clone evaluates instead of failing on a missing path.
  linkIfPresent =
    target: name:
    lib.optionalAttrs (builtins.pathExists (shipped name)) { ${target}.source = shipped name; };
in
{
  options.programs.helm = {
    enable = lib.mkEnableOption "helm user configuration";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "helm.packages.\${system}.default";
      description = "The helm package whose units and wrapper this user runs.";
    };

    paletteFile = lib.mkOption {
      type = lib.types.path;
      default = "${cfg.package}/share/helm/palette.toml";
      defaultText = lib.literalExpression "\${cfg.package}/share/helm/palette.toml";
      description = ''
        This user's palette, linked to ~/.config/helm/palette.toml.
        `helm ctl theme apply` (M1) renders every themed file from it.
      '';
    };

    startBar = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run helm-bar as a user unit under helm-session.target. The unit is
        installed either way; this only controls whether the session target
        wants it.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ] ++ support.reusedTools pkgs;

    xdg.configFile =
      {
        "helm/palette.toml".source = cfg.paletteFile;
        # A user-level portal policy overrides the system one, and on
        # nix-on-non-NixOS it may be the only one that exists (SPEC 0005 §5).
        "xdg-desktop-portal/helm-portals.conf".source =
          "${cfg.package}/share/xdg-desktop-portal/helm-portals.conf";
      }
      # M1 replaces these static links with helm-theme's generated output; the
      # destination paths do not change.
      // linkIfPresent "yazi" "yazi"
      // linkIfPresent "btop/themes/helm.theme" "btop/helm.theme"
      // linkIfPresent "starship.toml" "starship/starship.toml"
      // linkIfPresent "foot/foot.ini" "foot/foot.ini"
      // linkIfPresent "fuzzel/fuzzel.ini" "fuzzel/fuzzel.ini";

    # These mirror packaging/systemd/*. home-manager writes its own copies into
    # ~/.config/systemd/user, which take precedence over the package's units —
    # keep the two in step.
    systemd.user.targets.helm-session = {
      Unit = {
        Description = "helm desktop session";
        BindsTo = [ "graphical-session.target" ];
        Wants = [ "graphical-session-pre.target" ];
        After = [ "graphical-session-pre.target" ];
      };
    };

    systemd.user.services.helm-wm = {
      Unit = {
        Description = "helm window manager and session daemon";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
        StartLimitIntervalSec = 30;
        StartLimitBurst = 5;
        OnFailure = [ "helm-session-abort.service" ];
      };
      Service = {
        Environment = "PATH=/usr/lib/helm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin";
        # PRE-ALPHA: this binary does not exist yet (M2). The unit is here so
        # the session shape is complete and testable.
        ExecStart = "${cfg.package}/bin/helm-wm";
        # always, not on-failure: under river 0.4 this process *is* the window
        # manager, and quit goes through river's exit_session rather than
        # through this exiting — so a clean exit is not a normal path and must
        # never leave river unmanaged. 69 = river answered `unavailable`
        # (another window manager holds the global), 78 = protocol mismatch;
        # restarting cannot help with either and looping buries the message.
        Restart = "always";
        RestartSec = 1;
        RestartPreventExitStatus = "69 78";
        Slice = "session.slice";
        TimeoutStopSec = 10;
      };
      Install.WantedBy = [ "helm-session.target" ];
    };

    # The abort path: fires once the window manager has exhausted its restart
    # limit, and signals the session entry to return the user to the display
    # manager rather than leaving them on an inert compositor (SPEC 0005 §2).
    systemd.user.services.helm-session-abort = {
      Unit.Description = "helm session abort (window manager did not attach)";
      Service = {
        Type = "oneshot";
        ExecStart = "${cfg.package}/bin/helm-session --abort";
      };
    };

    systemd.user.services.helm-bar = {
      Unit = {
        Description = "helm bar";
        PartOf = [ "graphical-session.target" ];
        After = [
          "graphical-session.target"
          "helm-wm.service"
        ];
        Wants = [ "helm-wm.service" ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
        StartLimitIntervalSec = 30;
        StartLimitBurst = 5;
      };
      Service = {
        Environment = "PATH=/usr/lib/helm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin";
        # PRE-ALPHA: lands in M2, like helm-wm above.
        ExecStart = "${cfg.package}/bin/helm-bar";
        # A crashed bar restarts itself and never takes the session with it
        # (docs/PITFALLS.md). app.slice, not session.slice: under memory
        # pressure the restartable thing should be reaped before the window
        # manager.
        Restart = "on-failure";
        RestartSec = 1;
        Slice = "app.slice";
        TimeoutStopSec = 5;
      };
      Install.WantedBy = lib.optionals cfg.startBar [ "helm-session.target" ];
    };
  };
}
