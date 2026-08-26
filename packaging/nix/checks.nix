# `nix flake check` outputs.
#
# `shellcheck` and `package` build on any Linux builder; `session-boots` is a
# NixOS VM test and needs /dev/kvm. CI (.github/workflows/distro.yml) falls back
# to `nix flake check --no-build` plus the two buildable checks when KVM is
# absent, so those two must stay independently buildable.
{
  pkgs,
  lib,
  src,
  helm,
  nixosModule,
}:
{
  # The session wrapper is the file most likely to break a login, and the only
  # shell in the repo. Keep it clean.
  shellcheck =
    pkgs.runCommand "helm-shellcheck" { nativeBuildInputs = [ pkgs.shellcheck ]; }
      ''
        shellcheck --shell=bash ${src + "/packaging/session/helm-session"}
        touch $out
      '';

  package = helm;

  # `pkgs.testers.nixosTest`, not the old top-level `nixosTest` alias, which
  # nixpkgs now refuses.
  session-boots = pkgs.testers.nixosTest {
    name = "helm-session-boots";

    nodes.machine =
      { ... }:
      {
        imports = [ nixosModule ];
        programs.helm.enable = true;
        # Keep the VM small: no display manager, no graphical login. This test
        # asserts the *contract* is installed, not that a desktop renders — see
        # the M2 block below for that.
        virtualisation.memorySize = 2048;
      };

    testScript = ''
      # Everything asserted here is true of helm 0.1.0 today.
      machine.wait_for_unit("multi-user.target")

      # The wayland-session entry a display manager would offer.
      machine.succeed("test -f /run/current-system/sw/share/wayland-sessions/helm.desktop")
      machine.succeed("grep -q '^DesktopNames=helm$' /run/current-system/sw/share/wayland-sessions/helm.desktop")

      # The session wrapper is installed, wrapped, and its preflight agrees that
      # the compositor and the D-Bus tooling are present.
      machine.succeed("helm-session --version")
      machine.succeed("helm-session --check")

      # river 0.4.x, the compositor helm drives. `-version` (one dash) is
      # river's own spelling. Anything below 0.4 does not implement
      # river-window-management-v1 and the flake would have refused to evaluate;
      # this asserts the package that actually landed.
      machine.succeed("river -version")

      # The user units that carry the session.
      machine.succeed("test -f /etc/systemd/user/helm-session.target")
      machine.succeed("test -f /etc/systemd/user/helm-bar.service")
      machine.succeed("test -f /etc/systemd/user/helm-sessiond.service")
      machine.succeed("test -f /etc/systemd/user/helm-session-abort.service")

      # The .wants symlinks. Without these, starting helm-session.target starts
      # nothing at all and exits 0 — the failure SPEC 0005 §4 exists to prevent.
      # If this assertion fails, NixOS did not propagate the package's *.wants
      # directory: fix it by declaring the wants in the module rather than by
      # deleting the assertion.
      machine.succeed("test -e /etc/systemd/user/helm-session.target.wants/helm-sessiond.service")
      machine.succeed("test -e /etc/systemd/user/helm-session.target.wants/helm-bar.service")

      # The restart policy the supervision design depends on (SPEC 0005 §2).
      machine.succeed(
          "grep -q '^RestartPreventExitStatus=69 78$' /etc/systemd/user/helm-sessiond.service"
      )
      machine.succeed("grep -q '^Slice=app.slice$' /etc/systemd/user/helm-bar.service")

      # Portal policy: a named backend per interface, so behaviour does not
      # depend on what happens to be installed. ScreenCast must not resolve to
      # gtk, which implements none on wlroots compositors. NixOS writes
      # xdg.portal.config.helm to this path; the deb and rpm get the same
      # policy from configs/portal/helm-portals.conf.
      machine.succeed("grep -q '^default=gtk$' /etc/xdg/xdg-desktop-portal/helm-portals.conf")
      machine.succeed(
          "grep -q '^org.freedesktop.impl.portal.ScreenCast=wlr$' "
          "/etc/xdg/xdg-desktop-portal/helm-portals.conf"
      )

      # The palette every themed surface is generated from.
      machine.succeed("test -f /etc/helm/palette.toml")

      # ── PENDING M2 — the assertion this test exists for ────────────────────
      # helm-bar and helm-sessiond do not exist yet, so this block does not run.
      # When M2 lands, delete `pending_m2` and the guard; the body is the real
      # test: log a user in, wait for the window manager to attach to river and
      # the bar to map a layer surface, and assert the environment handshake
      # actually happened.
      pending_m2 = False
      if pending_m2:
          machine.succeed("loginctl enable-linger alice")
          machine.wait_for_unit("helm-session.target", user="alice")
          # Under river 0.4 the compositor is inert until helm-wm attaches as
          # its window manager, so this is the liveness check that matters.
          machine.wait_until_succeeds("pgrep -u alice helm-wm")
          machine.wait_until_succeeds("pgrep -u alice helm-bar")
          # The failure this guards: a portal activated without WAYLAND_DISPLAY
          # hangs for 25 s and then fails.
          machine.succeed(
              "su - alice -c 'systemctl --user show-environment' | grep -q '^WAYLAND_DISPLAY='"
          )
          machine.screenshot("helm-bar")
    '';
  };
}
