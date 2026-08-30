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
        shellcheck --shell=sh \
          ${src + "/packaging/check-font-policy.sh"} \
          ${src + "/packaging/font-policy-test.sh"} \
          ${src + "/packaging/nix/check-root-flake-ci.sh"} \
          ${src + "/packaging/nix/test-root-flake-ci.sh"} \
          ${src + "/docs/check-readme-truth-snapshot.sh"} \
          ${src + "/docs/test-readme-truth-snapshot.sh"}
        touch $out
      '';

  package = helm;

  # The local agent-SDD validator reads Git objects at runtime.  Its package
  # wrapper must supply Git without adding it to the desktop session wrapper.
  helm-sdd-git-runtime =
    pkgs.runCommand "helm-sdd-git-runtime" {
      nativeBuildInputs = [
        pkgs.coreutils
        pkgs.git
        pkgs.gnugrep
      ];
    }
      ''
        repo="$TMPDIR/repo"
        mkdir "$repo"
        cd "$repo"

        git init --initial-branch=main
        git config user.email test@example.invalid
        git config user.name test
        printf 'fixture\n' > README
        git add README
        git commit --no-gpg-sign --message initial
        parent="$(git rev-parse HEAD)"

        mkdir -p .agent/work/120
        cat > .agent/work/120/checkpoint.toml <<EOF
schema = "helm-agent-sdd/checkpoint/v1"
issue = 120
reason = "handoff"
created_at = "2026-08-29T12:00:00Z"
current_maturity = "probe"
requested_maturity = "spike"
git_head = "$parent"
question = "Can the focused check reproduce the result?"
success_condition = "One reproducible command exists."
limitations = []
affected_specs = []

[goal]
statement = "State one durable task goal."

[acceptance]
criteria = ["A concrete condition exists."]

[workspace]
branch = "main"
base = "$parent"
dirty = false

[next_actions]
items = ["Rerun the focused check."]
EOF
        cat > .agent/work/120/evidence.jsonl <<EOF
{"id":"ev-001","ts":"2026-08-29T12:00:00Z","kind":"command","summary":"Ran focused check","command":"cargo test -p helm-core","exit_code":0,"git_head":"$parent","purpose":"reproduce result"}
EOF
        git add .agent/work/120
        git commit --no-gpg-sign --message record

        ${pkgs.coreutils}/bin/env -i PATH=/nonexistent \
          ${helm}/bin/helm-sdd gate --issue 120 --from probe --to spike > "$TMPDIR/actual.json"
        printf '%s\n' '{"issue":120,"from":"probe","to":"spike","outcome":"pass","obligations":[{"code":"accepted_refs","status":"met"},{"code":"clean_workspace","status":"met"},{"code":"decision_provenance","status":"met"},{"code":"fresh_checkpoint","status":"met"},{"code":"fresh_evidence","status":"met"},{"code":"git_objects","status":"met"},{"code":"hygiene","status":"met"},{"code":"issue_directory","status":"met"},{"code":"schema","status":"met"},{"code":"transition","status":"met"},{"code":"transition_evidence","status":"met"},{"code":"transition_fields","status":"met"}]}' > "$TMPDIR/expected.json"
        cmp --silent "$TMPDIR/expected.json" "$TMPDIR/actual.json"
        ! grep -F '${pkgs.git}/bin' ${helm}/bin/helm-session
        touch $out
      '';

  # `pkgs.testers.nixosTest`, not the old top-level `nixosTest` alias, which
  # nixpkgs now refuses.
  session-boots = pkgs.testers.nixosTest {
    name = "helm-session-boots";

    nodes.machine =
      { config, ... }:
      {
        imports = [ nixosModule ];
        programs.helm.enable = true;
        services.displayManager.ly.enable = true;
        virtualisation.memorySize = 2048;
      };

    testScript =
      { nodes, ... }:
      let
        desktops = nodes.machine.config.services.displayManager.sessionData.desktops;
      in
      ''
      # Everything asserted here is true of helm 0.1.0 today.
      machine.wait_for_unit("multi-user.target")

      # The wayland-session entry materialised by NixOS display-manager session data.
      machine.succeed("test -f ${desktops}/share/wayland-sessions/helm.desktop")
      machine.succeed(
        "grep -q '^DesktopNames=helm$' ${desktops}/share/wayland-sessions/helm.desktop"
      )
      machine.succeed(
        "grep -Eq '^Exec=/nix/store/[^/]+-helm-session-launch$' "
        + "${desktops}/share/wayland-sessions/helm.desktop"
      )

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
      machine.succeed("test -f /etc/systemd/user/helm-wm.service")
      machine.succeed("test -f /etc/systemd/user/helm-session-abort.service")

      # The .wants symlinks. Without these, starting helm-session.target starts
      # nothing at all and exits 0 — the failure SPEC 0005 §4 exists to prevent.
      # If this assertion fails, NixOS did not propagate the package's *.wants
      # directory: fix it by declaring the wants in the module rather than by
      # deleting the assertion.
      machine.succeed("test -e /etc/systemd/user/helm-session.target.wants/helm-wm.service")
      machine.succeed("test -e /etc/systemd/user/helm-session.target.wants/helm-bar.service")

      # The restart policy the supervision design depends on (SPEC 0005 §2).
      machine.succeed(
          "grep -q '^RestartPreventExitStatus=69 78$' /etc/systemd/user/helm-wm.service"
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
      # helm-bar and helm-wm do not exist yet, so this block does not run.
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
