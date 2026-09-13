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
        shellcheck --shell=bash \
          ${src + "/packaging/session/realm-session"} \
          ${src + "/packaging/session/test-runtime-dir-mode.sh"} \
          ${src + "/packaging/session/test-portal-warmup.sh"}
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
        bash ${src + "/packaging/session/test-runtime-dir-mode.sh"}
        bash ${src + "/packaging/session/test-portal-warmup.sh"}
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

  # The installed command owns the narrow command path needed by its two
  # expressly permitted external probes. A systemd transient service supplies
  # no interactive-shell path, so exercise the same condition directly.
  realmctl-command-runtime = pkgs.runCommand "realmctl-command-runtime" {
    nativeBuildInputs = [ pkgs.jq ];
  } ''
    mkdir -m 700 "$TMPDIR/runtime" "$TMPDIR/home"
    set +e
    env -i \
      HOME="$TMPDIR/home" \
      XDG_RUNTIME_DIR="$TMPDIR/runtime" \
      XDG_DATA_DIRS=${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name} \
      PATH=/not-an-installed-command-path \
      ${realm}/bin/realmctl --json --palette ${src + "/palette.toml"} doctor \
      >"$TMPDIR/report.json"
    status=$?
    set -e
    echo "doctor command-path fixture exit: $status"
    cat "$TMPDIR/report.json"
    case "$status" in
      0|1) ;;
      *) exit 1 ;;
    esac
    ! grep -F 'error:not found' "$TMPDIR/report.json"
    jq -e '.checks | map(select(.id == "tools/floors")) |
      length == 1 and .[0].group == "tools" and .[0].status == "skip"' \
      "$TMPDIR/report.json"
    touch $out
  '';

  # Keep the bare-session cursor inputs independently buildable. Pinned
  # nixpkgs' GLib setup hook moves schemas below share/gsettings-schemas/$name,
  # and its desktop-manager modules publish that package-named directory in
  # XDG_DATA_DIRS. Assert both package layouts before the slower VM exercises
  # the live gsettings/cursor agreement through `realmctl doctor`.
  desktop-session-data = pkgs.runCommand "realm-desktop-session-data" { } ''
    test -f ${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}/glib-2.0/schemas/gschemas.compiled
    test -d ${pkgs.adwaita-icon-theme}/share/icons/Adwaita/cursors
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
      import datetime as dt
      import hashlib
      import json
      import shlex
      from pathlib import Path
      from test_driver.errors import RequestedAssertionFailed

      STARTUP_TIMEOUT = dt.timedelta(seconds=120)
      STATE_TIMEOUT = dt.timedelta(seconds=60)
      OCR_TIMEOUT = dt.timedelta(seconds=120)
      EXIT_TIMEOUT = dt.timedelta(seconds=30)
      DIAGNOSTIC_TIMEOUT = dt.timedelta(seconds=10)

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
              except RequestedAssertionFailed as error:
                  observed = {"control_error": str(error)}
              else:
                  observed = response
                  # Keep schema and predicate failures outside the transient
                  # client retry path: malformed production state must fail now.
                  if predicate(response):
                      return True
              if last_try:
                  machine.log(f"last state while waiting for {description}: {observed!r}")
              return False

          retry(matches, timeout=STATE_TIMEOUT)
          return observed_raw, observed

      def wait_for_single_user_process(name):
          quoted = shlex.quote(name)
          machine.wait_until_succeeds(
              f'test "$(pgrep -u alice -x -c {quoted})" -eq 1',
              timeout=STATE_TIMEOUT,
          )
          return machine.succeed(f"pgrep -u alice -x -o {quoted}").strip()

      def assert_fixed_consumer(pid, executable, expected_args, generation):
          actual_executable = machine.succeed(f"readlink /proc/{pid}/exe").strip()
          assert actual_executable == executable, (actual_executable, executable)
          actual_args = machine.succeed(
              f"tr '\\0' '\\n' < /proc/{pid}/cmdline"
          ).splitlines()
          assert actual_args == expected_args, (actual_args, expected_args)

          leases = machine.succeed(
              f"grep -R -l -x 'pid {pid}' /home/alice/.config/realm/generated/leases"
          ).splitlines()
          assert len(leases) == 1, leases
          lease = machine.succeed(f"cat {shlex.quote(leases[0])}")
          assert f"generation {generation}\n" in lease, lease
          assert f"pid {pid}\n" in lease, lease

      def write_artifact(name, content):
          (Path(machine.out_dir) / name).write_text(content, encoding="utf-8")

      def log_startup_diagnostics():
          commands = [
              (
                  "Realm user unit status",
                  "systemctl --user --machine=alice@ --no-pager --full status "
                  "realm-session.target realm-wm.service realm-bar.service",
              ),
              (
                  "Realm user unit journal",
                  "journalctl -b --no-pager -n 200 _UID=1000 "
                  "_SYSTEMD_USER_UNIT=realm-session.target "
                  "_SYSTEMD_USER_UNIT=realm-wm.service "
                  "_SYSTEMD_USER_UNIT=realm-bar.service",
              ),
              (
                  "display manager journal",
                  "journalctl -b --no-pager -n 120 -u display-manager.service",
              ),
              (
                  "session log and user processes",
                  "if test -f /home/alice/.local/state/realm/session.log; then "
                  "tail -n 120 /home/alice/.local/state/realm/session.log; "
                  "else echo 'realm session log absent'; fi; ps -fu alice",
              ),
          ]
          for label, command in commands:
              try:
                  status, output = machine.execute(command, timeout=DIAGNOSTIC_TIMEOUT)
                  machine.log(f"{label} (exit {status}):\n{output}")
              except Exception as error:
                  machine.log(f"{label} unavailable: {error}")

      machine.wait_for_unit("multi-user.target", timeout=STARTUP_TIMEOUT)

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
      machine.succeed(
          "grep -q '^org.freedesktop.impl.portal.Settings=gtk$' "
          "/etc/xdg/xdg-desktop-portal/realm-portals.conf"
      )
      machine.succeed(
          "grep -q '^org.freedesktop.impl.portal.Inhibit=none$' "
          "/etc/xdg/xdg-desktop-portal/realm-portals.conf"
      )

      # The palette every themed surface is generated from.
      machine.succeed("test -f /etc/realm/palette.toml")

      # A real graphical login must reach the readiness boundary of the
      # installed daemon. Type=notify makes `active` mean River's management
      # and layer-shell globals, the control listener, and the combined loop
      # are live rather than merely that exec(2) succeeded.
      try:
          # Login and the wrapper run asynchronously after multi-user.target.
          # wait_for_unit rejects an inactive target before its start job exists.
          machine.wait_until_succeeds(
              "systemctl --user --machine=alice@ is-active --quiet realm-session.target",
              timeout=STARTUP_TIMEOUT,
          )
          machine.wait_for_unit(
              "realm-wm.service", user="alice", timeout=STARTUP_TIMEOUT
          )
          machine.wait_for_unit(
              "realm-bar.service", user="alice", timeout=STARTUP_TIMEOUT
          )
      except Exception:
          log_startup_diagnostics()
          raise
      machine.succeed(
          "test \"$(systemctl --user --machine=alice@ show realm-wm.service -p Type --value)\" = notify"
      )

      wm_pid = machine.succeed("pgrep -u alice -xo realm-wm").strip()
      bar_pid = machine.succeed("pgrep -u alice -xo realm-bar").strip()
      river_pid = machine.succeed("pgrep -u alice -xo river").strip()
      machine.succeed(f"test \"$(readlink /proc/{wm_pid}/exe)\" = ${realm}/bin/realm-wm")
      machine.succeed(f"test \"$(readlink /proc/{bar_pid}/exe)\" = ${realm}/bin/realm-bar")
      # The package unit and NixOS-generated drop-in jointly determine PATH.
      # Inspect the running daemon so a later drop-in cannot silently replace
      # a correct-looking package unit with NixOS's minimal default PATH.
      machine.succeed("grep -F -q '${pkgs.foot}/bin' /etc/systemd/user/realm-wm.service")
      machine.succeed("grep -F -q '${pkgs.fuzzel}/bin' /etc/systemd/user/realm-wm.service")
      daemon_path = machine.succeed(
          f"tr '\\0' '\\n' < /proc/{wm_pid}/environ | sed -n 's/^PATH=//p'"
      ).strip().split(":")
      assert '${realm}/bin' in daemon_path, daemon_path
      assert '${pkgs.foot}/bin' in daemon_path, daemon_path
      assert '${pkgs.fuzzel}/bin' in daemon_path, daemon_path

      # Check the user-manager publication against the installed daemon that
      # inherited it. A client started without this value cannot map a surface.
      imported_wayland = machine.succeed(
          "systemctl --user --machine=alice@ show-environment | sed -n 's/^WAYLAND_DISPLAY=//p'"
      ).strip()
      daemon_wayland = machine.succeed(
          f"tr '\\0' '\\n' < /proc/{wm_pid}/environ | sed -n 's/^WAYLAND_DISPLAY=//p'"
      ).strip()
      assert imported_wayland == daemon_wayland and imported_wayland

      # Run the installed diagnostic inside the user manager so it receives
      # the same published graphical-session environment as supervised units.
      # This is the healthy-session acceptance path for the complete fixed
      # check set; skips remain explicit data rather than omitted checks.
      # The session entry enqueues cold portal activation after its environment
      # imports. Waiting here verifies that production path; the fixture does
      # not manually start the service before doctor.
      machine.wait_for_unit(
          "xdg-desktop-portal.service", user="alice", timeout=STARTUP_TIMEOUT
      )
      doctor_raw = machine.succeed(
          "systemd-run --user --machine=alice@ --wait --pipe --quiet --collect "
          "${realm}/bin/realmctl --json doctor"
      )
      doctor = json.loads(doctor_raw)
      assert len(doctor["checks"]) == 32, doctor
      assert [check["id"] for check in doctor["checks"]] == [
          "session/socket", "session/protocol-version", "session/degraded",
          "wm/attached", "wm/layer-shell", "wm/capabilities",
          "wm/protocol-version", "env/identity",
          "env/wayland-display/process", "env/wayland-display/systemd",
          "env/wayland-display/dbus", "env/desktop/systemd",
          "env/desktop/dbus", "env/agree", "env/stale", "env/cursor",
          "env/xwayland", "env/list-matches-entry", "units/target",
          "units/wm", "units/bar", "units/restart-policy",
          "units/idle-lock", "portal/answers", "portal/config",
          "portal/filechooser", "portal/screencast", "palette/lint",
          "theme/outputs", "fonts/glyphs", "fonts/attribution",
          "tools/floors",
      ]
      assert all(check["status"] != "fail" for check in doctor["checks"]), doctor
      doctor_by_id = {check["id"]: check for check in doctor["checks"]}
      assert {
          check["id"] for check in doctor["checks"] if check["status"] == "skip"
      } == {"units/idle-lock", "portal/filechooser", "tools/floors"}, doctor
      for check_id in [
          "session/socket", "session/protocol-version", "wm/attached",
          "wm/layer-shell", "portal/answers", "portal/config",
          "portal/screencast",
      ]:
          assert doctor_by_id[check_id]["status"] == "ok", doctor
      write_artifact("realmctl-doctor.json", doctor_raw)

      # Use the library client itself. The initial state proves Hello and
      # GetState reached the production server independently of realmctl.
      initial_raw, initial = control("state")
      assert initial["reply"] == "state", initial
      assert initial["data"]["whichkey"] is True, initial
      write_artifact("control-get-state.json", initial_raw)

      modules_raw, modules = wait_for_state(
          lambda response: [
              module["id"] for module in response["data"]["modules"]
          ] == ["net", "cpu", "mem", "clock"],
          "ordered batteryless MVP system modules",
      )
      write_artifact("control-modules-state.json", modules_raw)

      # Exercise the installed fixed-consumer route through River's real
      # default bindings. A PATH grep alone cannot prove that Session emits the
      # typed effect, the worker starts the executor, or the executor selects N.
      generation = machine.succeed(
          "cat /home/alice/.config/realm/generated/current"
      ).strip()
      generation_root = (
          f"/home/alice/.config/realm/generated/generations/{generation}"
      )
      machine.succeed(
          "sudo -u alice ${pkgs.foot}/bin/foot --check-config "
          f"--config={generation_root}/foot/foot.ini"
      )

      machine.send_key("meta_l-ret")
      terminal_pid = wait_for_single_user_process("foot")
      _terminal_raw, terminal_state = wait_for_state(
          lambda response: sum(
              cell["windows"] for cell in response["data"]["orbits"]
          ) == 1,
          "default-binding terminal window",
      )
      assert terminal_state["reply"] == "state", terminal_state
      assert_fixed_consumer(
          terminal_pid,
          "${pkgs.foot}/bin/foot",
          [
              "foot",
              f"--config={generation_root}/foot/foot.ini",
              "--override=key-bindings.spawn-terminal=none",
          ],
          generation,
      )
      machine.succeed(f"kill -TERM {terminal_pid}")
      machine.wait_until_succeeds(
          f"test ! -d /proc/{terminal_pid}", timeout=STATE_TIMEOUT
      )
      wait_for_state(
          lambda response: sum(
              cell["windows"] for cell in response["data"]["orbits"]
          ) == 0,
          "default-binding terminal close",
      )

      machine.succeed(
          "install -d -o alice -g users -m 0755 "
          "/home/alice/.local/share/applications"
      )
      machine.succeed(
          "printf '%s\\n' '[Desktop Entry]' 'Type=Application' "
          "'Name=Realm Launch Ready' 'Exec=${pkgs.coreutils}/bin/true' "
          "> /home/alice/.local/share/applications/realm-launch-ready.desktop && "
          "chown alice:users "
          "/home/alice/.local/share/applications/realm-launch-ready.desktop && "
          "chmod 0644 "
          "/home/alice/.local/share/applications/realm-launch-ready.desktop"
      )
      machine.send_key("meta_l-d")
      launcher_pid = wait_for_single_user_process("fuzzel")
      assert_fixed_consumer(
          launcher_pid,
          "${pkgs.fuzzel}/bin/fuzzel",
          ["fuzzel", f"--config={generation_root}/fuzzel/fuzzel.ini"],
          generation,
      )
      machine.wait_for_text("Realm Launch Ready", timeout=OCR_TIMEOUT)
      machine.send_key("esc")
      machine.wait_until_succeeds(
          f"test ! -d /proc/{launcher_pid}", timeout=STATE_TIMEOUT
      )

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
      machine.wait_for_text("Realm VM sample", timeout=OCR_TIMEOUT)
      # The strip has no heading. Its distinctive launcher label proves it is
      # rendered; exact OCR of the small full-spellbook prompt is unreliable.
      # Neither the sample applications nor the bar title contains this label.
      machine.wait_for_text("hecate", timeout=OCR_TIMEOUT)
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
      machine.wait_for_text("GRIMOIRE", timeout=OCR_TIMEOUT)
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
          # The requested VM mode need not be the compositor's actual mode.
          # Read the PNG IHDR rather than claiming the configured resolution.
          assert payload[:8] == bytes([137, 80, 78, 71, 13, 10, 26, 10])
          assert payload[12:16] == b"IHDR" and len(payload) >= 24
          width = int.from_bytes(payload[16:20], "big")
          height = int.from_bytes(payload[20:24], "big")
          assert width > 0 and height > 0
          captures.append({
              "file": filename,
              "width": width,
              "height": height,
              "sha256": hashlib.sha256(payload).hexdigest(),
              "state_file": state_file,
          })
      provenance = {
          "schema": "realm-vm-capture-provenance/v1",
          "source_revision": "${sourceRevision}",
          "realm_package": "${realm}",
          "environment": "NixOS QEMU framebuffer, River 0.4.8; dimensions recorded per capture",
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
      machine.wait_until_succeeds(
          f"test ! -d /proc/{river_pid}", timeout=EXIT_TIMEOUT
      )
    '';
  };
}
