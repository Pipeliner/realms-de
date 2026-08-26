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

    systemd.user.services.helm-daemon = {
      Unit = {
        Description = "helm session daemon and river window manager";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        # PRE-ALPHA: this binary does not exist yet (M2). The unit is here so
        # the session shape is complete and testable.
        ExecStart = "${cfg.package}/bin/helm-sessiond";
        # Under river 0.4 this process *is* the window manager: while it is
        # down, river manages nothing. Restart fast — see the comment in
        # packaging/systemd/helm-daemon.service for why this is on-failure and
        # not always.
        Restart = "on-failure";
        RestartSec = 1;
      };
      Install.WantedBy = [ "helm-session.target" ];
    };

    systemd.user.services.helm-bar = {
      Unit = {
        Description = "helm bar";
        PartOf = [ "graphical-session.target" ];
        After = [
          "graphical-session.target"
          "helm-daemon.service"
        ];
        Wants = [ "helm-daemon.service" ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        # PRE-ALPHA: lands in M2, like helm-sessiond above.
        ExecStart = "${cfg.package}/bin/helm-bar";
        # A crashed bar restarts itself and never takes the session with it
        # (docs/PITFALLS.md).
        Restart = "on-failure";
        RestartSec = 1;
      };
      Install.WantedBy = lib.optionals cfg.startBar [ "helm-session.target" ];
    };
  };
}
