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
  realm,
  desktopAdmissionVmTest,
  nixosModule,
  sourceRevision,
  vmControlHelper,
}:
{
  # The session wrapper is the file most likely to break a login, and the only
  # shell in the repo. Keep it clean.
  shellcheck =
    pkgs.runCommand "realm-shellcheck" { nativeBuildInputs = [ pkgs.shellcheck ]; }
      ''
        shellcheck --shell=bash ${src + "/packaging/session/realm-session"}
        shellcheck --shell=sh \
          ${src + "/packaging/check-font-policy.sh"} \
          ${src + "/packaging/font-policy-test.sh"} \
          ${src + "/packaging/nix/check-root-flake-ci.sh"} \
          ${src + "/packaging/nix/test-root-flake-ci.sh"} \
          ${src + "/docs/check-readme-truth-snapshot.sh"} \
          ${src + "/docs/test-readme-truth-snapshot.sh"} \
          ${src + "/docs/check-contribution-templates.sh"} \
          ${src + "/docs/test-contribution-templates.sh"} \
          ${src + "/docs/test-gh-body-file.sh"} \
          ${src + "/docs/check-github-body-safety.sh"} \
          ${src + "/docs/test-github-body-safety.sh"} \
          ${src + "/packaging/debian/toolchain-path.sh"} \
          ${src + "/packaging/debian/test-toolchain-path.sh"}
        touch $out
      '';

  package = realm;

  # Keep the package output contract explicit. Unlike a source-text check,
  # this runs against the real derivation and fails if postInstall cannot place
  # either workspace binary in the package output.
  packaged-binaries = pkgs.runCommand "realm-packaged-binaries" { } ''
    test -x ${realm}/bin/realmctl
    test -x ${realm}/bin/realm-sdd
    touch $out
  '';

  # The local agent-SDD validator reads Git objects at runtime.  Its package
  # wrapper must supply Git without adding it to the desktop session wrapper.
  realm-sdd-git-runtime =
    pkgs.runCommand "realm-sdd-git-runtime" {
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
schema = "realm-agent-sdd/checkpoint/v1"
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
{"id":"ev-001","ts":"2026-08-29T12:00:00Z","kind":"command","summary":"Ran focused check","command":"cargo test -p realm-core","exit_code":0,"git_head":"$parent","purpose":"reproduce result"}
EOF
        git add .agent/work/120
        git commit --no-gpg-sign --message record

        ${pkgs.coreutils}/bin/env -i PATH=/nonexistent \
          ${realm}/bin/realm-sdd gate --issue 120 --from probe --to spike > "$TMPDIR/actual.json"
        printf '%s\n' '{"issue":120,"from":"probe","to":"spike","outcome":"pass","obligations":[{"code":"accepted_refs","status":"met"},{"code":"clean_workspace","status":"met"},{"code":"decision_provenance","status":"met"},{"code":"fresh_checkpoint","status":"met"},{"code":"fresh_evidence","status":"met"},{"code":"git_objects","status":"met"},{"code":"hygiene","status":"met"},{"code":"issue_directory","status":"met"},{"code":"schema","status":"met"},{"code":"transition","status":"met"},{"code":"transition_evidence","status":"met"},{"code":"transition_fields","status":"met"}]}' > "$TMPDIR/expected.json"
        cmp --silent "$TMPDIR/expected.json" "$TMPDIR/actual.json"
        ! grep -F '${pkgs.git}/bin' ${realm}/bin/realm-session
        touch $out
      '';

  # `pkgs.testers.nixosTest`, not the old top-level `nixosTest` alias, which
  # nixpkgs now refuses.
  session-boots = pkgs.testers.nixosTest {
    name = "realm-session-boots";
    enableOCR = true;

    nodes.machine =
      { config, pkgs, ... }:
      {
        imports = [ nixosModule ];
        programs.realm.enable = true;
        services.displayManager.ly.enable = true;
        services.displayManager.defaultSession = "realm";
        services.displayManager.autoLogin = {
          enable = true;
          user = "alice";
        };
        virtualisation.memorySize = 2048;
        virtualisation.resolution = {
          x = 1920;
          y = 1080;
        };
        users.users.alice = {
          isNormalUser = true;
        };
        environment.systemPackages = [
          vmControlHelper
          pkgs.foot
        ];
      };

    testScript =
      { nodes, ... }:
      let
        desktops = nodes.machine.config.services.displayManager.sessionData.desktops;
      in
      ''
      import hashlib
      import json
      import shlex
      from pathlib import Path

      def as_alice(*argv):
          return shlex.join([
              "sudo",
              "-u",
              "alice",
              "env",
              "XDG_RUNTIME_DIR=/run/user/1000",
              *argv,
          ])

      def control(*argv):
          output = machine.succeed(as_alice("realm-vm-control", *argv))
          return output, json.loads(output)

      def wait_for_state(predicate, description):
          observed_raw = None
          observed = None

          def matches(last_try):
              nonlocal observed_raw
              nonlocal observed
              try:
                  observed_raw, response = control("state")
                  observed = response
                  if predicate(response):
                      return True
              except Exception as error:
                  observed = {"error": str(error)}
              if last_try:
                  machine.log(f"last state while waiting for {description}: {observed!r}")
              return False

          retry(matches)
          return observed_raw, observed

      def write_artifact(name, content):
          (Path(machine.out_dir) / name).write_text(content, encoding="utf-8")

      machine.wait_for_unit("multi-user.target")

      # Task 3's descriptor admission needs a positive proof that is impossible
      # on the host test filesystem. The Nix store itself is group-writable in
      # NixOS, which the admission policy deliberately refuses, so root copies
      # a known native ELF to a test-only root-owned immutable-location fixture.
      machine.succeed("install -d -m 0755 /opt/realm-desktop-exec-test")
      machine.succeed(
        "install -m 0555 ${pkgs.coreutils}/bin/true /opt/realm-desktop-exec-test/true"
      )
      machine.succeed(
        "su -s /bin/sh alice -c "
        + "'REALM_DESKTOP_EXEC_TEST_ELF=/opt/realm-desktop-exec-test/true "
        + "${desktopAdmissionVmTest}/libexec/realm-theme-desktop-exec-tests "
        + "--exact generation::desktop_exec::tests::nixos_vm_static_preflight_admits_root_owned_elf_from_non_root_user "
        + "--ignored --nocapture'"
      )

      # The wayland-session entry materialised by NixOS display-manager session data.
      machine.succeed("test -f ${desktops}/share/wayland-sessions/realm.desktop")
      machine.succeed(
        "grep -q '^DesktopNames=realm$' ${desktops}/share/wayland-sessions/realm.desktop"
      )
      machine.succeed(
        "grep -Eq '^Exec=/nix/store/[^/]+-realm-session-launch$' "
        + "${desktops}/share/wayland-sessions/realm.desktop"
      )

      # The session wrapper is installed, wrapped, and its preflight agrees that
      # the compositor and the D-Bus tooling are present.
      machine.succeed("realm-session --version")
      machine.succeed("realm-session --check")

      # river 0.4.x, the compositor realm drives. `-version` (one dash) is
      # river's own spelling. Anything below 0.4 does not implement
      # river-window-management-v1 and the flake would have refused to evaluate;
      # this asserts the package that actually landed.
      machine.succeed("river -version")

      # The user units that carry the session.
      machine.succeed("test -f /etc/systemd/user/realm-session.target")
      machine.succeed("test -f /etc/systemd/user/realm-bar.service")
      machine.succeed("test -f /etc/systemd/user/realm-wm.service")
      machine.succeed("test -f /etc/systemd/user/realm-session-abort.service")

      # The .wants symlinks. Without these, starting realm-session.target starts
      # nothing at all and exits 0 — the failure SPEC 0005 §4 exists to prevent.
      # If this assertion fails, NixOS did not propagate the package's *.wants
      # directory: fix it by declaring the wants in the module rather than by
      # deleting the assertion.
      machine.succeed("test -e /etc/systemd/user/realm-session.target.wants/realm-wm.service")
      machine.succeed("test -e /etc/systemd/user/realm-session.target.wants/realm-bar.service")

      # The restart policy the supervision design depends on (SPEC 0005 §2).
      machine.succeed(
          "grep -q '^RestartPreventExitStatus=69 78$' /etc/systemd/user/realm-wm.service"
      )
      machine.succeed("grep -q '^Slice=app.slice$' /etc/systemd/user/realm-bar.service")

      # Portal policy: a named backend per interface, so behaviour does not
      # depend on what happens to be installed. ScreenCast must not resolve to
      # gtk, which implements none on wlroots compositors. NixOS writes
      # xdg.portal.config.realm to this path; the deb and rpm get the same
      # policy from configs/portal/realm-portals.conf.
      machine.succeed("grep -q '^default=gtk$' /etc/xdg/xdg-desktop-portal/realm-portals.conf")
      machine.succeed(
          "grep -q '^org.freedesktop.impl.portal.ScreenCast=wlr$' "
          "/etc/xdg/xdg-desktop-portal/realm-portals.conf"
      )

      # The palette every themed surface is generated from.
      machine.succeed("test -f /etc/realm/palette.toml")

      # A real graphical login must reach the readiness boundary of the
      # installed daemon. Type=notify makes `active` mean River's management
      # and layer-shell globals, the control listener, and the combined loop
      # are live rather than merely that exec(2) succeeded.
      machine.wait_for_unit("realm-session.target", user="alice")
      machine.wait_for_unit("realm-wm.service", user="alice")
      machine.wait_for_unit("realm-bar.service", user="alice")
      machine.succeed(
          "test \"$(systemctl --user --machine=alice@ show realm-wm.service -p Type --value)\" = notify"
      )

      wm_pid = machine.succeed("pgrep -u alice -xo realm-wm").strip()
      bar_pid = machine.succeed("pgrep -u alice -xo realm-bar").strip()
      river_pid = machine.succeed("pgrep -u alice -xo river").strip()
      machine.succeed(f"test \"$(readlink /proc/{wm_pid}/exe)\" = ${realm}/bin/realm-wm")
      machine.succeed(f"test \"$(readlink /proc/{bar_pid}/exe)\" = ${realm}/bin/realm-bar")

      # Check the user-manager publication against the installed daemon that
      # inherited it. A client started without this value cannot map a surface.
      imported_wayland = machine.succeed(
          "systemctl --user --machine=alice@ show-environment | sed -n 's/^WAYLAND_DISPLAY=//p'"
      ).strip()
      daemon_wayland = machine.succeed(
          f"tr '\\0' '\\n' < /proc/{wm_pid}/environ | sed -n 's/^WAYLAND_DISPLAY=//p'"
      ).strip()
      assert imported_wayland == daemon_wayland and imported_wayland

      # Use the library client itself. The initial state proves Hello and
      # GetState reached the production server; no realmctl command is added.
      initial_raw, initial = control("state")
      assert initial["reply"] == "state", initial
      assert initial["data"]["whichkey"] is True, initial
      write_artifact("control-get-state.json", initial_raw)

      # Three ordinary Wayland application windows must enter compositor-backed
      # state before the tiled-desktop framebuffer capture is accepted.
      for number in range(1, 4):
          command = (
              f"printf 'Realm VM sample {number}\\nordinary Wayland application\\n'; "
              "exec ${pkgs.coreutils}/bin/sleep infinity"
          )
          _raw, response = control(
              "spawn",
              "${pkgs.foot}/bin/foot",
              f"--title=realm-vm-{number}",
              "${pkgs.bash}/bin/bash",
              "-lc",
              command,
          )
          assert response == {"reply": "ok"}, response

      tiled_raw, tiled = wait_for_state(
          lambda response: sum(cell["windows"] for cell in response["data"]["orbits"]) == 3,
          "three managed demo windows",
      )
      assert tiled["data"]["whichkey"] is True, tiled
      machine.wait_for_text("Realm VM sample")
      machine.wait_for_text("which-key")
      write_artifact("control-tiled-state.json", tiled_raw)
      machine.screenshot("realm-tiled-desktop")

      # Drive the real River keybinding path. State snapshots pair each
      # framebuffer capture with the UI state it is intended to show.
      machine.send_key("meta_l-w")
      _hidden_raw, _hidden = wait_for_state(
          lambda response: response["data"]["whichkey"] is False,
          "which-key hidden",
      )
      machine.send_key("meta_l-w")
      _shown_raw, _shown = wait_for_state(
          lambda response: response["data"]["whichkey"] is True,
          "which-key visible",
      )
      machine.send_key("meta_l-shift-0x35")
      grimoire_raw, grimoire = wait_for_state(
          lambda response: response["data"]["grimoire"] is True,
          "grimoire visible",
      )
      assert sum(cell["windows"] for cell in grimoire["data"]["orbits"]) == 3, grimoire
      machine.wait_for_text("GRIMOIRE")
      write_artifact("control-grimoire-state.json", grimoire_raw)
      machine.screenshot("realm-grimoire")

      # Hashes and environment metadata keep later asset copying traceable to
      # these exact compositor framebuffer captures.
      captures = []
      for filename, state_file in [
          ("realm-tiled-desktop.png", "control-tiled-state.json"),
          ("realm-grimoire.png", "control-grimoire-state.json"),
      ]:
          payload = (Path(machine.out_dir) / filename).read_bytes()
          captures.append({
              "file": filename,
              "sha256": hashlib.sha256(payload).hexdigest(),
              "state_file": state_file,
          })
      provenance = {
          "schema": "realm-vm-capture-provenance/v1",
          "source_revision": "${sourceRevision}",
          "realm_package": "${realm}",
          "environment": "NixOS QEMU framebuffer, River 0.4.8, 1920x1080",
          "nixos_version": machine.succeed("nixos-version").strip(),
          "kernel": machine.succeed("uname -srmo").strip(),
          "river_version": machine.succeed("river -version").strip(),
          "clean_demo_session": True,
          "captures": captures,
      }
      assert provenance["source_revision"] != "unknown", provenance
      write_artifact(
          "capture-provenance.json",
          json.dumps(provenance, indent=2, sort_keys=True) + "\n",
      )

      # Quit is deliberately last. The helper only exits zero for the exact
      # Response::Ok frame, so this output proves the requester's response
      # drained before realm-wm asked River to end the session.
      quit_raw, quit_response = control("quit")
      assert quit_response == {"reply": "ok"}, quit_response
      write_artifact("control-quit.json", quit_raw)
      machine.wait_until_succeeds(f"test ! -d /proc/{river_pid}")
    '';
  };
}
