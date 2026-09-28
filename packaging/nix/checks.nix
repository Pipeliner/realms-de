# `nix flake check` outputs.
#
# `shellcheck` and `package` build on any Linux builder; `session-boots` is a
# NixOS VM test and needs /dev/kvm. CI (.github/workflows/distro.yml) falls back
# to `nix flake check --no-build` plus selected buildable checks when KVM is
# absent, so those checks must stay independently buildable.
{
  pkgs,
  lib,
  src,
  realm,
  desktopAdmissionVmTest,
  nixosModule,
  sourceRevision,
  support,
  vmControlHelper,
  portalVmHelper,
  waybarComparison ? false,
  waybarFixture ? null,
}:
let
  xwaylandWindowObservation = pkgs.writers.writePython3Bin
    "realm-xwayland-window-observation"
    { }
    (builtins.readFile ./xwayland_window_observation.py);
  realmYazi = lib.findFirst (
    package: lib.getName package == "yazi"
  ) null (support.reusedTools pkgs);
in
assert realmYazi != null;
assert realmYazi.version == "25.4.8";
assert !waybarComparison || waybarFixture != null;
{
  # The session wrapper is the file most likely to break a login, and the only
  # shell in the repo. Keep it clean.
  shellcheck =
    pkgs.runCommand "realm-shellcheck" {
      nativeBuildInputs = [ pkgs.shellcheck ];
      REALM_SUSPEND_FIXTURE = builtins.toJSON (import ./suspend_fixture.nix);
    }
      ''
        shellcheck --shell=bash \
          ${src + "/packaging/session/realm-session"} \
          ${src + "/packaging/session/test-runtime-dir-mode.sh"} \
          ${src + "/packaging/session/test-portal-warmup.sh"}
        shellcheck --shell=sh \
          ${src + "/packaging/session/realm-idle"} \
          ${src + "/packaging/session/realm-backlight"} \
          ${src + "/packaging/session/realm-browser"} \
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
        ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_portal_vm_helper.py"}
        REALM_NODE=${pkgs.nodejs}/bin/node ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_browser_screencast.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_window_controls.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_toolkit_focus.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_relogin.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/native-vm/test_relogin_roundtrip.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/native-vm/test_window_roundtrip.py"}
        ${pkgs.python3}/bin/python3 ${src + "/packaging/nix/test_suspend_builder.py"}
        bash ${src + "/packaging/session/test-portal-warmup.sh"}
        touch $out
      '';

  package = realm;

  # Execute the packaged helper's exact GI import path before spending time on
  # the graphical VM. This catches a typelib placed in a non-default output;
  # importing the Python source on the host cannot prove that closure.
  portal-helper-imports = pkgs.runCommand "realm-portal-helper-imports" { } ''
    ${portalVmHelper}/bin/realm-portal-vm --check-imports
    touch $out
  '';

  # Keep the package output contract explicit. Unlike a source-text check,
  # this runs against the real derivation and fails if postInstall cannot place
  # either workspace binary in the package output.
  packaged-binaries = pkgs.runCommand "realm-packaged-binaries" { } ''
    test -x ${realm}/bin/realmctl
    test -x ${realm}/bin/realm-browser
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

  xwayland-window-observation = pkgs.runCommand
    "realm-xwayland-window-observation-tests"
    {
      nativeBuildInputs = [
        pkgs.coreutils
        pkgs.python3
      ];
    }
    ''
      PYTHONDONTWRITEBYTECODE=1 ${pkgs.python3}/bin/python3 \
        ${src + "/packaging/nix/test_xwayland_window_observation.py"}
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
    name = if waybarComparison then "realm-waybar-comparison" else "realm-session-boots";
    enableOCR = true;

    nodes.machine =
      { config, pkgs, ... }:
      let
        xwaylandProbe = pkgs.writers.writePython3Bin "realm-xwayland-dbus-probe" {
          libraries = [ pkgs.python3Packages.dbus-next ];
        } ''
          import asyncio
          import os
          from pathlib import Path

          from dbus_next.aio import MessageBus
          from dbus_next.constants import RequestNameReply


          async def main():
              display = os.environ.get("DISPLAY")
              runtime_dir = os.environ.get("XDG_RUNTIME_DIR")
              if not display or not runtime_dir:
                  raise SystemExit("D-Bus activation omitted DISPLAY or XDG_RUNTIME_DIR")

              bus = await MessageBus().connect()
              reply = await bus.request_name("org.realm.XWaylandProbe")
              if reply is not RequestNameReply.PRIMARY_OWNER:
                  raise SystemExit("D-Bus probe did not acquire its configured name")

              title = "Realm X11 Probe A17-" + str(os.getpid())
              child = await asyncio.create_subprocess_exec(
                  "${pkgs.xmessage}"
                  "/bin/xmessage",
                  "-title",
                  title,
                  "-center",
                  "Realm X11 Probe",
              )
              marker = Path(runtime_dir) / "realm-xwayland-probe"
              marker.write_text(
                  "display=" + display + "\n"
                  "service_pid=" + str(os.getpid()) + "\n"
                  "child_pid=" + str(child.pid) + "\n"
                  "title=" + title + "\n",
                  encoding="utf-8",
              )
              await child.wait()
              bus.disconnect()


          asyncio.run(main())
        '';
        xwaylandProbeService = pkgs.writeTextFile {
          name = "realm-xwayland-dbus-service";
          destination = "/share/dbus-1/services/org.realm.XWaylandProbe.service";
          text = ''
            [D-BUS Service]
            Name=org.realm.XWaylandProbe
            Exec=${xwaylandProbe}/bin/realm-xwayland-dbus-probe
          '';
        };
      in
      {
        imports = [ nixosModule ];
        programs.realm.enable = true;
        services.displayManager.ly.enable = true;
        services.displayManager.ly.settings.default_input = "password";
        services.displayManager.defaultSession = "realm";
        services.displayManager.autoLogin = {
          enable = true;
          user = "alice";
        };
        # Use the installed interactive slurp chooser, including for Firefox.
        virtualisation.memorySize = 2048;
        # CI hypothesis only: ICH9 TCO reset followed a stalled S3 resume.
        # Removing this virtual device must not waive real suspend acceptance.
        virtualisation.qemu.options = [ "-global ICH9-LPC.enable_tco=off" ];
        # Keep resume/panic evidence visible if QEMU closes both driver sockets.
        boot.kernelParams = [ "no_console_suspend" "initcall_debug" ];
        virtualisation.resolution = {
          x = 1920;
          y = 1080;
        };
        users.users.alice = {
          isNormalUser = true;
          # Public fixture credential, used only inside this disposable VM.
          initialPassword = "realmtest";
        };
        services.dbus.packages = [ xwaylandProbeService ];
        # Synthetic CI guest only: locate the mapped-X11/Realm boundary.
        systemd.user.services.realm-wm.environment.WAYLAND_DEBUG = "client";
        environment.systemPackages = [
          vmControlHelper
          portalVmHelper
          pkgs.foot
          pkgs.zsh
          pkgs.starship
          realmYazi
          pkgs.btop
          pkgs.gtk3.dev
          pkgs.gtk4.dev
          pkgs.strace
          pkgs.firefox
          (pkgs.makeDesktopItem {
            name = "realm-browser-test";
            desktopName = "Realm Browser Test";
            exec = "${pkgs.firefox}/bin/firefox --no-remote about:blank";
            mimeTypes = [ "text/html" "x-scheme-handler/http" "x-scheme-handler/https" ];
          })
        ] ++ lib.optionals waybarComparison [ pkgs.waybar pkgs.python3 pkgs.wlr-randr pkgs.fontconfig ];
        environment.etc."xdg/mimeapps.list".text = ''
          [Default Applications]
          text/html=realm-browser-test.desktop
          x-scheme-handler/http=realm-browser-test.desktop
          x-scheme-handler/https=realm-browser-test.desktop
        '';
      };

    testScript =
      { nodes, ... }:
      let
        desktops = nodes.machine.config.services.displayManager.sessionData.desktops;
      in
      assert nodes.machine.config.services.pipewire.enable;
      ''
      import datetime as dt
      import hashlib
      import importlib
      import json
      import os
      import re
      import shlex
      import sys
      import time
      from pathlib import Path
      from test_driver.errors import RequestedAssertionFailed
      sys.path.insert(0, "${src + "/packaging/native-vm"}")
      window_probe = importlib.import_module("window_roundtrip")
      WINDOW_SNAPSHOT = window_probe.SNAPSHOT
      exercise_controls = window_probe.exercise_controls

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

      def wait_for_state(predicate, description, timeout=STATE_TIMEOUT):
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

          retry(matches, timeout=timeout)
          return observed_raw, observed

      def remaining_timeout(deadline, description):
          remaining = deadline - time.monotonic()
          assert remaining > 0, f"{description} exhausted the shared X11 deadline"
          return dt.timedelta(seconds=remaining)

      def x11_observation(display, title, timeout=DIAGNOSTIC_TIMEOUT):
          status, output = machine.execute(
              as_alice(
                  "${xwaylandWindowObservation}"
                  "/bin/realm-xwayland-window-observation",
                  "--timeout-bin",
                  "${pkgs.coreutils}/bin/timeout",
                  "--xwininfo-bin",
                  "${pkgs.xwininfo}/bin/xwininfo",
                  "--display",
                  display,
                  "--title",
                  title,
                  "--command-timeout",
                  "2s",
              ),
              timeout=timeout,
          )
          if status != 0:
              return {
                  "observer_status": status,
                  "observer_output": output,
                  "tree": {"status": None, "output": ""},
                  "window_ids": [],
                  "stats": [],
                  "viewable": False,
              }
          return json.loads(output)

      def log_x11_protocol_diagnostics():
          try:
              status, output = machine.execute(
                  "timeout --kill-after=1 5 sh -c " + shlex.quote(
                      "journalctl -b --no-pager -o cat -n 200 "
                      "_SYSTEMD_USER_UNIT=realm-wm.service "
                      "--grep='river_window_manager_v1|river_window_v1|protocol|error' "
                      "| tail -c 65536"
                  ),
                  timeout=DIAGNOSTIC_TIMEOUT,
              )
              machine.log(f"X11 Realm protocol journal (exit {status}):\n{output[-65536:]}")
          except Exception as error:
              machine.log(f"X11 Realm protocol journal unavailable: {str(error)[-512:]}")

      def log_x11_diagnostics(observation, child_pid):
          log_x11_protocol_diagnostics()
          tree = observation["tree"]
          machine.log(f"X11 root tree (exit {tree['status']}):\n{tree['output']}")
          for stats in observation["stats"]:
              machine.log(
                  f"X11 window {stats['window_id']} stats "
                  f"(exit {stats['status']}):\n{stats['output']}"
              )
          if "observer_status" in observation:
              machine.log(
                  f"X11 observer (exit {observation['observer_status']}):\n"
                  f"{observation['observer_output']}"
              )
          stderr_status, stderr = machine.execute(
              "journalctl -b --no-pager -o cat "
              f"_PID={shlex.quote(child_pid)}",
              timeout=DIAGNOSTIC_TIMEOUT,
          )
          machine.log(
              f"X11 child journal/stderr (exit {stderr_status}):\n{stderr}"
          )

      def wait_for_x11_mapping(display, title, child_pid, deadline):
          last_observation = None

          def mapped(_last_try):
              nonlocal last_observation
              last_observation = x11_observation(
                  display,
                  title,
                  timeout=remaining_timeout(deadline, "X11 window mapping"),
              )
              return last_observation["viewable"]

          try:
              retry(
                  mapped,
                  timeout=remaining_timeout(deadline, "X11 window mapping"),
              )
          except Exception:
              if last_observation is None:
                  last_observation = x11_observation(display, title)
              log_x11_diagnostics(last_observation, child_pid)
              raise
          return last_observation

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

      def process_environment(pid):
          return dict(
              line.split("=", 1)
              for line in machine.succeed(
                  f"tr '\\0' '\\n' < /proc/{pid}/environ"
              ).splitlines()
              if "=" in line
          )

      def write_artifact(name, content):
          (Path(machine.out_dir) / name).write_text(content, encoding="utf-8")

      def wait_for_managed_window_count(expected, description):
          return wait_for_state(
              lambda response: sum(
                  cell["windows"] for cell in response["data"]["orbits"]
              ) == expected,
              description,
          )

      def toolkit_focus_matches(response, expected_title):
          return (
              sum(cell["windows"] for cell in response["data"]["orbits"]) == 2
              and response["data"]["focused_title"] == expected_title
          )

      def assert_toolkit_diagnostics_clean(name, stderr, trace, required_paths,
                                           diagnostic_pattern):
          # The strict gate below failed in CI without printing the matched
          # line. Emit only bounded offending evidence before rejecting it.
          _status, matched_stderr = machine.execute(
              f"grep -n -E -i -m 10 {shlex.quote(diagnostic_pattern)} "
              f"{shlex.quote(stderr)} | head -c 8192",
              timeout=DIAGNOSTIC_TIMEOUT,
          )
          missing_open = []
          for required_path in required_paths:
              quoted_match = shlex.quote(f'"{required_path}"')
              open_status, _open_output = machine.execute(
                  f"grep -F {quoted_match} {shlex.quote(trace)} "
                  "| grep -E -q '= [0-9]+$'",
                  timeout=DIAGNOSTIC_TIMEOUT,
              )
              if open_status != 0:
                  missing_open.append(required_path)
          if matched_stderr or missing_open:
              if matched_stderr:
                  machine.log(f"{name} matched CSS/theme stderr:\n{matched_stderr}")
              for required_path in required_paths:
                  quoted_match = shlex.quote(f'"{required_path}"')
                  _status, matched_openat = machine.execute(
                      f"grep -F -m 10 {quoted_match} {shlex.quote(trace)} "
                      "| head -c 8192",
                      timeout=DIAGNOSTIC_TIMEOUT,
                  )
                  machine.log(
                      f"{name} openat {required_path}: "
                      + (matched_openat or "<no matching openat line>")
                  )
              _status, css_openat = machine.execute(
                  "grep -E -i -m 20 "
                  + shlex.quote(r"(gtk[.]css|realm/gtk-3)")
                  + f" {shlex.quote(trace)} | head -c 8192",
                  timeout=DIAGNOSTIC_TIMEOUT,
              )
              machine.log(
                  f"{name} relevant GTK openat: "
                  + (css_openat or "<no relevant openat line>")
              )
              _status, loader_openat = machine.execute(
                  "grep -E -i -m 20 "
                  + shlex.quote(r"(gdk-pixbuf|loaders[.]cache|mime)")
                  + f" {shlex.quote(trace)} | head -c 8192",
                  timeout=DIAGNOSTIC_TIMEOUT,
              )
              machine.log(
                  f"{name} loader openat: "
                  + (loader_openat or "<no loader openat line>")
              )
          machine.fail(
              f"grep -E -i -q {shlex.quote(diagnostic_pattern)} "
              f"{shlex.quote(stderr)}"
          )

      def exercise_toolkit(
          command,
          name,
          required_paths,
          expected_title,
          expected_text,
          diagnostic_pattern,
          screenshot=None,
          launcher=False,
      ):
          trace = f"/tmp/realm-{name}.trace"
          stderr = f"/tmp/realm-{name}.stderr"
          done = f"/tmp/realm-{name}.done"
          machine.succeed(
              f"rm -f {shlex.quote(trace)} {shlex.quote(stderr)} "
              f"{shlex.quote(done)}"
          )
          run_script = (
              "${pkgs.strace}/bin/strace -f -qq -e trace=openat "
              f"-o {shlex.quote(trace)} {command} 2> {shlex.quote(stderr)}; "
              "realm_probe_status=$?; printf '%s\\n' \"$realm_probe_status\" "
              f"> {shlex.quote(done)}"
          )
          if launcher:
              script = f"/tmp/realm-{name}-launch"
              environment_file = f"/tmp/realm-{name}.environment"
              desktop = f"/home/alice/.local/share/applications/realm-{name}.desktop"
              title = f"Realm Probe {name}"
              script_text = (
                  "#!/bin/sh\n"
                  + f"${pkgs.coreutils}/bin/env > {shlex.quote(environment_file)}\n"
                  + run_script + "\n"
              )
              desktop_text = (
                  "[Desktop Entry]\nType=Application\n"
                  + f"Name={title}\nExec={script}\n"
              )
              machine.succeed(
                  "install -d -o alice -g users -m 0755 "
                  "/home/alice/.local/share/applications && "
                  + "printf %s " + shlex.quote(script_text)
                  + " > " + shlex.quote(script)
                  + " && chmod 0755 " + shlex.quote(script)
                  + " && printf %s " + shlex.quote(desktop_text)
                  + " > " + shlex.quote(desktop)
                  + " && chown alice:users " + shlex.quote(desktop)
              )
              machine.send_key("meta_l-d")
              launcher_pid = wait_for_single_user_process("fuzzel")
              launcher_environment = process_environment(launcher_pid)
              assert launcher_environment["REALM_GENERATION"] == generation_root
              machine.send_chars(title)
              machine.wait_for_text(title, timeout=OCR_TIMEOUT)
              machine.send_key("ret")
              machine.wait_until_succeeds(
                  f"test -s {shlex.quote(environment_file)}", timeout=STATE_TIMEOUT
              )
              child_environment = dict(
                  line.split("=", 1)
                  for line in machine.succeed(
                      f"cat {shlex.quote(environment_file)}"
                  ).splitlines()
                  if "=" in line
              )
              for key in (
                  "REALM_GENERATION", "ZDOTDIR", "STARSHIP_CONFIG",
                  "YAZI_CONFIG_HOME", "GTK_THEME", "QT_QPA_PLATFORMTHEME",
                  "XDG_DATA_DIRS", "XDG_CONFIG_DIRS",
              ):
                  assert child_environment[key] == zsh_environment[key], (
                      key, child_environment, zsh_environment
                  )
              write_artifact(f"{name}-environment.json",
                             json.dumps(child_environment, indent=2))
          else:
              machine.send_chars(run_script + "\n")
          managed_raw, _managed = wait_for_state(
              lambda response: toolkit_focus_matches(response, expected_title),
              f"focused {name} application window",
          )
          if name.startswith("gtk3-"):
              # Inspect the real demo process, not the launching shell/strace.
              # This is bounded diagnostic evidence; title/window/trace gates
              # remain the acceptance conditions below.
              _status, child_selectors = machine.execute(
                  "for pid in $(pgrep -u alice -f gtk3-widget-factory); do "
                  "exe=$(readlink /proc/$pid/exe 2>/dev/null || true); "
                  "case \"$exe\" in */gtk3-widget-factory|*/.gtk3-widget-factory-wrapped) "
                  "printf 'pid=%s exe=%s\\n' \"$pid\" \"$exe\"; "
                  "tr '\\0' '\\n' < /proc/$pid/environ | "
                  "grep -E '^(GTK_THEME|XDG_DATA_DIRS|GDK_PIXBUF_MODULE_FILE)=' "
                  "| head -c 4096;; esac; done",
                  timeout=DIAGNOSTIC_TIMEOUT,
              )
              machine.log(
                  f"{name} selected GTK child environment: "
                  + (child_selectors[:8192] or "<unavailable>")
              )
          machine.wait_for_text(expected_text, timeout=OCR_TIMEOUT)
          if screenshot is not None:
              write_artifact(f"control-{name}-state.json", managed_raw)
              machine.screenshot(screenshot)
              screenshot_path = Path(machine.out_dir) / f"{screenshot}.png"
              assert screenshot_path.stat().st_size > 0, screenshot_path
          machine.send_key("meta_l-q")
          wait_for_managed_window_count(1, f"{name} application close")
          machine.wait_until_succeeds(
              f"test -s {shlex.quote(done)}", timeout=STATE_TIMEOUT
          )
          assert machine.succeed(f"cat {shlex.quote(done)}").strip() == "0"
          assert_toolkit_diagnostics_clean(
              name, stderr, trace, required_paths, diagnostic_pattern
          )
          matched_trace = []
          for required_path in required_paths:
              quoted_match = shlex.quote(f'"{required_path}"')
              machine.succeed(
                  f"grep -F {quoted_match} {shlex.quote(trace)} "
                  "| grep -E -q '= [0-9]+$'"
              )
              matched_trace.append(
                  machine.succeed(
                      f"grep -F {quoted_match} {shlex.quote(trace)}"
                  )
              )
          write_artifact(f"{name}-openat.log", "".join(matched_trace))
          write_artifact(
              f"{name}-stderr.log",
              machine.succeed(f"cat {shlex.quote(stderr)}"),
          )

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

      def select_portal_output(screenshot):
          machine.wait_until_succeeds("pgrep -u alice -x slurp", timeout=STATE_TIMEOUT)
          machine.screenshot(screenshot)
          qmp = machine.qmp_client
          assert qmp is not None, "portal pointer QMP connection is unavailable"

          def pointer_command(command, arguments=None):
              try:
                  result = qmp.send(command, arguments) if arguments is not None else qmp.send(command)
              except Exception as error:
                  machine.log(f"portal pointer QMP {command} {arguments!r} raised: {error}")
                  raise
              machine.log(f"portal pointer QMP {command} {arguments!r} result: {result!r}")
              assert isinstance(result, dict) and "return" in result, (
                  f"portal pointer QMP {command} failed: {result!r}"
              )
              return result["return"]

          mice = pointer_command("query-mice")
          machine.log(f"portal pointer inventory: {mice!r}")
          assert isinstance(mice, list), f"portal pointer inventory is malformed: {mice!r}"
          active = [mouse for mouse in mice if isinstance(mouse, dict) and mouse.get("current") is True]
          assert len(active) == 1 and active[0].get("absolute") is True, (
              f"portal requires one active absolute pointer: {mice!r}"
          )
          machine.log(f"portal pointer selected: {active[0]!r}; target QMP absolute x=16384 y=16384")
          pointer_command("input-send-event", {"events": [
              {"type": "abs", "data": {"axis": "x", "value": 16384}},
              {"type": "abs", "data": {"axis": "y", "value": 16384}},
          ]})
          machine.screenshot(screenshot + "-pointer")
          deadline = time.monotonic() + STATE_TIMEOUT.total_seconds()
          while machine.execute("pgrep -u alice -x slurp", timeout=DIAGNOSTIC_TIMEOUT)[0] == 0:
              assert time.monotonic() < deadline, "portal output selection timed out"
              pointer_command("input-send-event", {"events": [
                  {"type": "btn", "data": {"button": "left", "down": True}},
              ]})
              pointer_command("input-send-event", {"events": [
                  {"type": "btn", "data": {"button": "left", "down": False}},
              ]})
              time.sleep(0.5)

      def select_portal_file():
          machine.succeed(f"touch {portal_ready_path}.continue")
          machine.wait_until_succeeds(f"test -s {portal_ready_path}.selection", timeout=STATE_TIMEOUT)
          ready = json.loads(machine.succeed(f"cat {portal_ready_path}.selection"))
          assert ready["elapsed_ms"] <= 2000 and ready["handle"].endswith("/realm_select"), ready
          wait_for_state(lambda response: sum(cell["windows"] for cell in response["data"]["orbits"]) == 1,
                         "managed file selection chooser")
          machine.wait_for_text("Realm file selection", timeout=OCR_TIMEOUT)
          machine.screenshot("realm-portal-file-selection")
          machine.send_key("ctrl-l")
          machine.send_chars("/tmp/realmfile")
          machine.send_key("alt-o")
          wait_for_state(lambda response: sum(cell["windows"] for cell in response["data"]["orbits"]) == 0,
                         "file selection chooser closed")

      def log_portal_diagnostics():
          commands = [
              (
                  "portal and PipeWire user unit status",
                  "systemctl --user --machine=alice@ --no-pager --full status "
                  "xdg-desktop-portal.service xdg-desktop-portal-gtk.service "
                  "xdg-desktop-portal-wlr.service pipewire.socket "
                  "pipewire.service wireplumber.service",
              ),
              (
                  "portal and PipeWire user journal",
                  "journalctl -b --no-pager -n 240 _UID=1000 "
                  "_SYSTEMD_USER_UNIT=xdg-desktop-portal.service "
                  "_SYSTEMD_USER_UNIT=xdg-desktop-portal-gtk.service "
                  "_SYSTEMD_USER_UNIT=xdg-desktop-portal-wlr.service "
                  "_SYSTEMD_USER_UNIT=pipewire.service "
                  "_SYSTEMD_USER_UNIT=wireplumber.service",
              ),
          ]
          for label, command in commands:
              try:
                  status, output = machine.execute(command, timeout=DIAGNOSTIC_TIMEOUT)
                  machine.log(f"{label} (exit {status}):\n{output}")
              except Exception as error:
                  machine.log(f"{label} unavailable: {error}")

      machine.wait_for_unit("multi-user.target", timeout=STARTUP_TIMEOUT)
      # Installed package/PAM and fresh-login activation proof.
      machine.succeed("test -x ${realm}/bin/realm-idle -a -x ${realm}/bin/realm-backlight")
      machine.succeed("test -f /etc/pam.d/swaylock")
      machine.succeed("grep -F 'ExecStart=${pkgs.swaylock}/bin/swaylock -f' ${realm}/lib/systemd/user/realm-lock.service")
      machine.succeed("test -e ${realm}/lib/systemd/user/realm-session.target.wants/realm-idle.service")

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
      machine.succeed("test -e /etc/systemd/user/realm-session.target.wants/realm-idle.service")

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
          machine.wait_for_unit(
              "realm-idle.service", user="alice", timeout=STARTUP_TIMEOUT
          )
          idle_login = machine.succeed(as_alice(
              "systemctl", "--user", "show", "realm-idle.service",
              "-p", "ActiveState", "-p", "MainPID", "-p", "ActiveEnterTimestampMonotonic"
          ))
          write_artifact("idle-login.txt", idle_login)
          # Keep the long acceptance journey controlled after proving startup.
          machine.succeed(as_alice("systemctl", "--user", "stop", "realm-idle.service"))
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
      assert '${pkgs.zsh}/bin' in daemon_path, daemon_path
      assert '${pkgs.starship}/bin' in daemon_path, daemon_path
      assert '${realmYazi}/bin' in daemon_path, daemon_path
      assert '${pkgs.btop}/bin' in daemon_path, daemon_path
      wm_environment = process_environment(wm_pid)
      qt6_plugin_root = "${pkgs.qt6Packages.qt6ct}/${pkgs.qt6Packages.qtbase.qtPluginPrefix}"
      assert qt6_plugin_root in wm_environment["QT_PLUGIN_PATH"].split(":"), (
          wm_environment.get("QT_PLUGIN_PATH"), qt6_plugin_root
      )
      machine.succeed(
          f"test -f {shlex.quote(qt6_plugin_root + '/platformthemes/libqt6ct.so')}"
      )
      svg_loader_cache = wm_environment["GDK_PIXBUF_MODULE_FILE"]
      assert svg_loader_cache
      machine.succeed(f"test -f {shlex.quote(svg_loader_cache)}")
      # librsvg 2.62.3 installs libpixbufloader_svg.so (underscore). Require
      # the complete quoted filename in GdkPixbuf's generated cache, not merely
      # an inherited environment variable or an obsolete loader name.
      svg_loader_query = (
          "grep -F -q 'libpixbufloader_svg.so\"' " + shlex.quote(svg_loader_cache)
      )
      svg_loader_status, _ = machine.execute(
          svg_loader_query, timeout=DIAGNOSTIC_TIMEOUT
      )
      if svg_loader_status != 0:
          _, cache_excerpt = machine.execute(
              f"head -c 4096 {shlex.quote(svg_loader_cache)}",
              timeout=DIAGNOSTIC_TIMEOUT,
          )
          machine.log(f"SVG loader cache missing pinned module (bounded):\n{cache_excerpt[:4096]}")
      machine.succeed(svg_loader_query)

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
      # XWayland is useful only if the wrapper learns its assigned DISPLAY and
      # publishes it before either launcher starts. The purpose-built D-Bus
      # service acquires its configured name before recording its activation
      # environment and opening a real X11 window, so a failed activation
      # cannot look like a successful propagation proof.
      imported_display = machine.succeed(
          "systemctl --user --machine=alice@ show-environment | sed -n 's/^DISPLAY=//p'"
      ).strip()
      daemon_display = machine.succeed(
          f"tr '\\0' '\\n' < /proc/{wm_pid}/environ | sed -n 's/^DISPLAY=//p'"
      ).strip()
      assert imported_display == daemon_display and imported_display
      assert imported_display.startswith(":"), imported_display
      machine.succeed(
          f"test -S /tmp/.X11-unix/X{shlex.quote(imported_display[1:])}"
      )
      machine.succeed(
          "grep -F -q 'xwayland display discovered: DISPLAY=' "
          "/home/alice/.local/state/realm/session.log"
      )
      machine.fail(
          "grep -F -q 'DEGRADED NO-XWAYLAND' "
          "/home/alice/.local/state/realm/session.log"
      )

      baseline_raw, baseline = control("state")
      baseline_windows = sum(
          cell["windows"] for cell in baseline["data"]["orbits"]
      )
      x11_deadline = time.monotonic() + STATE_TIMEOUT.total_seconds()
      machine.succeed(
          as_alice(
              "env",
              "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
              "dbus-send",
              "--session",
              "--type=method_call",
              "--dest=org.realm.XWaylandProbe",
              "/org/realm/XWaylandProbe",
              "org.realm.XWaylandProbe.Open",
          )
      )
      marker = "/run/user/1000/realm-xwayland-probe"
      machine.wait_until_succeeds(
          f"test -s {marker}",
          timeout=remaining_timeout(x11_deadline, "D-Bus activation marker"),
      )
      activated_display = machine.succeed(
          f"sed -n 's/^display=//p' {marker}"
      ).strip()
      service_pid = machine.succeed(
          f"sed -n 's/^service_pid=//p' {marker}"
      ).strip()
      x11_pid = machine.succeed(
          f"sed -n 's/^child_pid=//p' {marker}"
      ).strip()
      x11_title = machine.succeed(
          f"sed -n 's/^title=//p' {marker}"
      ).strip()
      assert activated_display == imported_display, (
          activated_display,
          imported_display,
      )
      x11_exe = machine.succeed(f"readlink /proc/{x11_pid}/exe").strip()
      expected_x11_exe = "${pkgs.xmessage}/bin/.xmessage-wrapped"
      assert x11_exe == expected_x11_exe, (x11_exe, expected_x11_exe)
      assert x11_title == f"Realm X11 Probe A17-{service_pid}", x11_title
      x11_mapping = wait_for_x11_mapping(
          activated_display,
          x11_title,
          x11_pid,
          x11_deadline,
      )
      try:
          wait_for_state(
              lambda response: sum(
                  cell["windows"] for cell in response["data"]["orbits"]
              ) == baseline_windows + 1,
              "D-Bus-activated X11 window",
              timeout=remaining_timeout(x11_deadline, "Realm X11 observation"),
          )
      except Exception:
          x11_observation_after_projection = x11_observation(
              activated_display,
              x11_title,
          )
          log_x11_diagnostics(x11_observation_after_projection, x11_pid)
          machine.log(f"X11 mapping-boundary observation:\n{x11_mapping!r}")
          raise
      machine.wait_for_text("Realm X11 Probe", timeout=OCR_TIMEOUT)
      machine.succeed(f"kill -TERM {x11_pid}")
      machine.wait_until_succeeds(
          f"test ! -d /proc/{x11_pid} && test ! -d /proc/{service_pid}",
          timeout=STATE_TIMEOUT,
      )
      wait_for_state(
          lambda response: sum(
              cell["windows"] for cell in response["data"]["orbits"]
          ) == baseline_windows,
          "D-Bus-activated X11 window close",
      )

      # Use the library client itself. The initial state proves Hello and
      # GetState reached the production server independently of realmctl.
      initial_raw, initial = control("state")
      assert initial["reply"] == "state", initial
      assert initial["data"]["whichkey"] is True, initial
      assert sum(cell["windows"] for cell in initial["data"]["orbits"]) == 0, initial
      write_artifact("control-get-state.json", initial_raw)

      # Exercise the actual installed proxy, GTK and wlr backends, and the
      # per-user PipeWire graph. The helper retains one D-Bus connection so the
      # request/session handles remain owned by the same caller. The driver
      # waits for a validated request marker, observes the real GTK chooser as
      # a managed and rendered River window, and cancels it with a real key.
      # Receiving a ScreenCast node id is not enough: the helper maps and hashes
      # one nonempty video buffer from the restricted PipeWire FD.
      portal_ready_path = "/tmp/realm-portal-filechooser-ready.json"
      portal_output_path = "/tmp/realm-portal-roundtrip.json"
      portal_error_path = "/tmp/realm-portal-roundtrip.stderr"
      portal_status_path = "/tmp/realm-portal-roundtrip.status"
      try:
          machine.wait_until_succeeds(
              "systemctl --user --machine=alice@ is-active --quiet pipewire.socket",
              timeout=STATE_TIMEOUT,
          )
          portal_command = shlex.join([
              "sudo",
              "-u",
              "alice",
              "env",
              "XDG_RUNTIME_DIR=/run/user/1000",
              "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
              f"WAYLAND_DISPLAY={imported_wayland}",
              f"REALM_PORTAL_FILECHOOSER_READY={portal_ready_path}",
              "realm-portal-vm",
          ])
          machine.succeed(
              f"(if {portal_command} > {portal_output_path} "
              f"2> {portal_error_path}; then "
              f"realm_portal_status=0; else realm_portal_status=$?; fi; "
              f"printf '%s\\n' \"$realm_portal_status\" > {portal_status_path}) "
              "< /dev/null > /dev/null 2>&1 &"
          )
          machine.wait_until_succeeds(
              f"test -s {portal_ready_path}", timeout=STATE_TIMEOUT
          )
          portal_ready = json.loads(machine.succeed(f"cat {portal_ready_path}"))
          assert portal_ready["elapsed_ms"] <= 2000, portal_ready
          assert portal_ready["handle"].endswith("/realm_file"), portal_ready

          _chooser_raw, chooser_state = wait_for_state(
              lambda response: sum(
                  cell["windows"] for cell in response["data"]["orbits"]
              ) == 1,
              "managed portal file chooser",
          )
          assert chooser_state["reply"] == "state", chooser_state
          machine.wait_for_text("Realm portal VM", timeout=OCR_TIMEOUT)
          machine.send_key("alt-c")
          wait_for_state(
              lambda response: sum(
                  cell["windows"] for cell in response["data"]["orbits"]
              ) == 0,
              "portal file chooser close after explicit Cancel",
          )

          select_portal_file()
          select_portal_output("realm-portal-output-chooser")
          machine.wait_until_succeeds(
              f"test -s {portal_status_path}", timeout=OCR_TIMEOUT
          )
          portal_status = machine.succeed(f"cat {portal_status_path}").strip()
          assert portal_status == "0", (
              portal_status,
              machine.succeed(f"cat {portal_error_path}"),
          )
          portal_raw = machine.succeed(f"cat {portal_output_path}")
          portal = json.loads(portal_raw)
          assert portal["filechooser"]["elapsed_ms"] <= 2000, portal
          assert portal["filechooser"]["completion"] == "response", portal
          assert portal["filechooser"]["response_code"] == 1, portal
          selected = portal["file_selection"]
          assert selected["elapsed_ms"] <= 2000 and selected["completion"] == "response", selected
          assert selected["response_code"] == 0 and selected["uri"] == "file:///tmp/realmfile", selected
          assert selected["bytes"] == 29, selected
          assert selected["sha256"] == hashlib.sha256(b"Realm portal selection proof\n").hexdigest(), selected
          assert portal["settings"]["reply_type"] == "(a{sa{sv}})", portal
          assert portal["screencast"]["node_id"] > 0, portal
          assert portal["screencast"]["buffer_bytes"] > 0, portal
          assert portal["screencast"]["width"] > 0, portal
          assert portal["screencast"]["height"] > 0, portal
          assert len(portal["screencast"]["sha256"]) == 64, portal
          machine.succeed(
              "systemctl --user --machine=alice@ is-active --quiet pipewire.service"
          )
      except Exception:
          for path in [portal_ready_path, portal_status_path, portal_error_path]:
              status, output = machine.execute(f"cat {path}")
              machine.log(f"portal helper {path} (exit {status}):\n{output}")
          log_portal_diagnostics()
          raise
      write_artifact("portal-roundtrip.json", portal_raw)

      modules_raw, modules = wait_for_state(
          lambda response: [
              module["id"] for module in response["data"]["modules"]
          ] == ["net", "cpu", "mem", "clock"],
          "ordered batteryless MVP system modules",
      )
      write_artifact("control-modules-state.json", modules_raw)

      machine.succeed(
          "${pkgs.util-linux}/bin/setsid --wait ${realmYazi}/bin/yazi --version "
          "</dev/null >/tmp/realm-yazi-version 2>&1"
      )
      assert machine.succeed("cat /tmp/realm-yazi-version").startswith("Yazi 25.4.8")
      write_artifact("consumer-versions.json", json.dumps({
          "foot": "${pkgs.foot.version}",
          "zsh": "${pkgs.zsh.version}",
          "starship": "${pkgs.starship.version}",
          "yazi": "${realmYazi.version}",
          "btop": "${pkgs.btop.version}",
          "gtk3": "${pkgs.gtk3.version}",
          "gtk4": "${pkgs.gtk4.version}",
          "qt6ct": "${pkgs.qt6Packages.qt6ct.version}",
      }, indent=2))

      # Exercise the installed fixed-consumer route through River's real
      # default bindings. A PATH grep alone cannot prove that Session emits the
      # typed effect, the worker starts the executor, or the executor selects N.
      login_record = json.loads(machine.succeed(
          "cat /run/user/1000/realm/session-theme.json"
      ))
      generation = login_record["generation"]
      generation_root = (
          f"/home/alice/.config/realm/generated/generations/{generation}"
      )
      machine.succeed(
          "sudo -u alice ${pkgs.foot}/bin/foot --check-config "
          f"--config={generation_root}/foot/foot-modern.ini"
      )

      # SPEC 0032: real installed locker/PAM; idle was stopped after login proof
      # and is restarted below for the unchanged 300/600-second timing checks.
      lock_results = []
      lock_unit = "realm-lock.service"
      def lock_systemctl(*args):
          return machine.succeed(as_alice(
              "timeout", "30", "systemctl", "--user", *args, lock_unit
          )).strip()

      def lock_processes():
          properties = lock_systemctl("show", "--property=ActiveState,SubState,MainPID,ControlGroup")
          machine.log("locker properties:\n" + properties)
          group = lock_systemctl("show", "--property=ControlGroup", "--value")
          assert group.startswith("/") and group != "/", group
          pids = machine.succeed(shlex.join([
              "cat", "/sys/fs/cgroup" + group + "/cgroup.procs"
          ])).split()
          identities = {}
          for pid in pids:
              assert pid.isdigit() and int(pid) > 0, pid
              stat = machine.succeed(f"cat /proc/{pid}/stat")
              fields = stat.rsplit(")", 1)[1].split()
              executable = machine.succeed(f"readlink /proc/{pid}/exe").strip()
              machine.log(f"locker process {pid}: {stat.strip()} exe={executable}")
              assert fields[0] not in ("Z", "X"), stat
              assert "swaylock" in Path(executable).name, executable
              identities[pid] = {"start_time": int(fields[19]), "executable": executable}
          assert identities, "active locker has no live processes"
          return identities

      try:
          for cycle in range(2):
              lock_systemctl("start")
              assert lock_systemctl("is-active") == "active"
              locker_processes = lock_processes()
              lock_systemctl("start")
              assert lock_processes() == locker_processes
              machine.screenshot(f"realm-locked-{cycle}")

              machine.send_key("meta_l-d")
              # Observe a bounded interval: a deferred launch is also a failure.
              machine.succeed(
                  "for attempt in $(seq 1 20); do "
                  "if pgrep -u alice -x fuzzel; then exit 1; fi; sleep 0.1; done"
              )
              # Clear any characters delivered to the locker by the shortcut.
              machine.send_key("ctrl-u")
              machine.send_chars("realmtest")
              machine.send_key("ret")
              machine.wait_until_succeeds(
                  as_alice("systemctl", "--user", "show", "--property=ActiveState",
                           "--value", lock_unit) + " | grep -qx inactive",
                  timeout=STATE_TIMEOUT,
              )
              for pid, identity in locker_processes.items():
                  status, stat = machine.execute(f"cat /proc/{pid}/stat")
                  if status == 0:
                      assert int(stat.rsplit(")", 1)[1].split()[19]) != identity["start_time"], stat
              lock_results.append({
                  "cycle": cycle, "processes": locker_processes,
                  "duplicate_start_same_processes": True, "launcher_binding_suppressed": True,
                  "password_unlock": True,
              })
      finally:
          status, journal = machine.execute(
              "journalctl --no-pager -b _SYSTEMD_USER_UNIT=realm-lock.service"
          )
          write_artifact("lock-journal.txt", journal)
          write_artifact("lock-roundtrip.json", json.dumps(lock_results, indent=2) + "\n")

      # Exercise the installed, unmodified 300/600-second timers. This VM has
      # no backlight; its real helper diagnostic proves that failure cannot
      # prevent the independent locker timeout. Fresh-login activation was
      # checked above; restart here only resets the controlled timing fixture.
      idle_results = {"auto_enabled": True, "backlight": "absent"}
      idle_unit = "realm-idle.service"

      def idle_journal():
          raw = machine.succeed(
              "journalctl --no-pager -b -o json _SYSTEMD_USER_UNIT=realm-idle.service"
          )
          return raw, [json.loads(line) for line in raw.splitlines() if line.strip()]

      def backlight_events(after):
          _raw, entries = idle_journal()
          return [
              int(entry["__MONOTONIC_TIMESTAMP"]) / 1_000_000
              for entry in entries
              if "backlight adjustment unavailable" in entry.get("MESSAGE", "")
              and int(entry["__MONOTONIC_TIMESTAMP"]) / 1_000_000 >= after
          ]

      try:
          machine.succeed("test -z \"$(ls -A /sys/class/backlight)\"")
          machine.succeed(as_alice("systemctl", "--user", "start", idle_unit))
          idle_pid = machine.succeed(as_alice(
              "systemctl", "--user", "show", "--property=MainPID", "--value", idle_unit
          )).strip()
          assert idle_pid.isdigit() and int(idle_pid) > 0, idle_pid
          machine.wait_until_succeeds(
              f"readlink /proc/{idle_pid}/exe | grep -q swayidle", timeout=STATE_TIMEOUT
          )
          idle_executable = machine.succeed(f"readlink /proc/{idle_pid}/exe").strip()
          idle_stat = machine.succeed(f"cat /proc/{idle_pid}/stat")
          idle_start_time = int(idle_stat.rsplit(")", 1)[1].split()[19])
          idle_args = machine.succeed(
              f"tr '\\0' '\\n' < /proc/{idle_pid}/cmdline"
          ).splitlines()
          assert idle_args[1:] == [
              "-w", "-C", "/dev/null",
              "timeout", "300", "realm-backlight dim", "resume", "realm-backlight restore",
              "timeout", "600", "systemctl --user start realm-lock.service",
              "before-sleep", "systemctl --user start realm-lock.service",
              "lock", "systemctl --user start realm-lock.service",
          ], idle_args
          # Give the real client time to bind its idle notifications, then
          # reset inactivity through actual compositor input, not a fake clock.
          time.sleep(1)
          baseline = float(machine.succeed("cut -d ' ' -f 1 /proc/uptime").strip())
          machine.send_key("esc")
          idle_results.update({"idle_pid": idle_pid, "argv": idle_args,
                               "executable": idle_executable, "start_time": idle_start_time,
                               "reset_monotonic_seconds": baseline})
          deadline = time.monotonic() + 640
          dim_time = None
          lock_time = None
          while time.monotonic() < deadline:
              events = backlight_events(baseline)
              if events and dim_time is None:
                  dim_time = events[0]
                  idle_results["dim_elapsed_seconds"] = dim_time - baseline
                  assert 299 <= dim_time - baseline <= 330, idle_results
                  machine.log(f"real idle dim callback after {dim_time - baseline:.2f}s")
              state = lock_systemctl("show", "--property=ActiveState", "--value")
              if state == "active":
                  lock_time = int(lock_systemctl(
                      "show", "--property=ActiveEnterTimestampMonotonic", "--value"
                  )) / 1_000_000
                  idle_results["lock_elapsed_seconds"] = lock_time - baseline
                  assert 599 <= lock_time - baseline <= 630, idle_results
                  break
              time.sleep(5)
          assert dim_time is not None and lock_time is not None, idle_results
          idle_results["locker_processes"] = lock_processes()
          machine.screenshot("realm-idle-locked")
          machine.send_chars("deliberately-wrong-password")
          machine.send_key("ret")
          time.sleep(5)
          assert lock_systemctl("is-active") == "active"
          assert lock_processes() == idle_results["locker_processes"]
          machine.send_key("meta_l-d")
          machine.succeed(
              "for attempt in $(seq 1 20); do "
              "if pgrep -u alice -x fuzzel; then exit 1; fi; sleep 0.1; done"
          )
          idle_results["wrong_password_still_locked"] = True
          machine.screenshot("realm-idle-wrong-password")
          machine.send_key("ctrl-u")
          machine.send_chars("realmtest")
          machine.send_key("ret")
          machine.wait_until_succeeds(
              as_alice("systemctl", "--user", "show", "--property=ActiveState",
                       "--value", lock_unit) + " | grep -qx inactive",
              timeout=STATE_TIMEOUT,
          )
          idle_results["password_unlock"] = True
          # Input wakes the 300-second notification even on a locked display.
          assert len(backlight_events(baseline)) >= 2, "missing idle resume callback"
          idle_results["resume_backlight_noop"] = True
      finally:
          stop_status, stop_output = machine.execute(as_alice(
              "timeout", "30", "systemctl", "--user", "stop", idle_unit
          ))
          idle_results["stop_status"] = stop_status
          idle_results["stop_output"] = stop_output
          # Failed derivation output directories are not uploaded. Emit the
          # bounded diagnostics before any artifact write can itself fail.
          machine.log("idle-roundtrip: " + json.dumps(idle_results, sort_keys=True))
          _status, journal = machine.execute(
              "journalctl --no-pager -b -n 200 -o json _SYSTEMD_USER_UNIT=realm-idle.service"
          )
          machine.log("idle-journal (last 200 entries):\n" + journal)
          write_artifact("idle-journal.jsonl", journal)
          write_artifact("idle-roundtrip.json", json.dumps(idle_results, indent=2) + "\n")
      assert stop_status == 0, stop_output
      machine.succeed(as_alice("systemctl", "--user", "show",
                               "--property=ActiveState", "--value", idle_unit)
                      + " | grep -qx inactive")
      status, remaining_stat = machine.execute(f"cat /proc/{idle_pid}/stat")
      if status == 0:
          assert int(remaining_stat.rsplit(")", 1)[1].split()[19]) != idle_start_time, remaining_stat
      idle_results["stopped_without_live_idle"] = True
      write_artifact("idle-roundtrip.json", json.dumps(idle_results, indent=2) + "\n")

      # Post-MVP reproducer only; manual lock/idle above remain mandatory.
      if os.environ.get("REALM_POST_MVP_SUSPEND") == "1":
          # Actual logind sleep, not an injected PrepareForSleep/Lock signal. The
          # monitor remains reachable while the guest shell is suspended.
          def arm_suspend_socket_timeouts(machine, seconds):
              previous = []
              for transport in (machine.shell, machine.monitor):
                  if transport is not None:
                      previous.append((transport, transport.gettimeout()))
                      transport.settimeout(seconds)
              return previous

          def suspend_host_diagnostics():
              evidence = {
                  "qemu_returncode": machine.process.poll() if machine.process else None,
                  "console_tail": [line[-512:] for line in machine.full_console_log[-80:]],
                  "qmp_events": [],
              }
              if machine.qmp_client is not None:
                  # Pinned QMPSession.read_pending_messages uses a nonblocking
                  # reader. Its wait_for_event spins indefinitely on an empty
                  # queue despite accepting a timeout, so do not call it here.
                  try:
                      for _ in range(128):
                          machine.qmp_client.read_pending_messages()
                  except Exception as error:
                      evidence["qmp_error"] = str(error)
                  for _ in range(128):
                      if machine.qmp_client.pending_events.empty():
                          break
                      evidence["qmp_events"].append(machine.qmp_client.pending_events.get_nowait())
              return evidence

          suspend_results = {"auto_enabled": False,
                             "fixture_experiment": "ICH9 TCO removal; resume root cause unproven",
                             "qemu_option": "-global ICH9-LPC.enable_tco=off"}
          suspend_journal = ""
          login_bus = ["timeout", "5", "busctl", "--system", "--timeout=5", "--json=short"]
          login_object = ["org.freedesktop.login1", "/org/freedesktop/login1",
                          "org.freedesktop.login1.Manager"]
          alice_uid = int(machine.succeed("id -u alice").strip())

          def login_call(method):
              return json.loads(machine.succeed(shlex.join(
                  login_bus + ["call"] + login_object + [method]
              ), timeout=DIAGNOSTIC_TIMEOUT))

          def login_property(name):
              reply = json.loads(machine.succeed(shlex.join(
                  login_bus + ["get-property"] + login_object + [name]
              ), timeout=DIAGNOSTIC_TIMEOUT))
              # Unlike method-call bodies, get-property unwraps the variant and
              # serializes a scalar property as scalar JSON data, not a list.
              assert reply["type"] == "t" and isinstance(reply["data"], int), reply
              return reply["data"]

          def own_sleep_inhibitors(pid):
              # ListInhibitors returns (what, who, why, mode, uid, pid).
              return [row for row in login_call("ListInhibitors")["data"][0]
                      if "sleep" in row[0].split(":") and row[3] == "delay"
                      and row[4] == alice_uid and row[5] == int(pid)]

          suspend_socket_timeouts = arm_suspend_socket_timeouts(machine, 10)
          try:
              assert lock_systemctl("show", "--property=ActiveState", "--value") == "inactive"
              machine.succeed("echo 1 > /sys/power/pm_debug_messages")
              suspend_results["watchdog_inventory"] = machine.succeed(
                  "for name in /sys/class/watchdog/watchdog*/identity; do "
                  "test ! -f \"$name\" || cat \"$name\"; done"
              ).strip()
              suspend_results["pm_debug_messages"] = machine.succeed(
                  "cat /sys/power/pm_debug_messages"
              ).strip()
              assert suspend_results["pm_debug_messages"] == "1", suspend_results
              assert login_call("CanSuspend")["data"] == ["yes"]
              machine.succeed("grep -qw mem /sys/power/state; grep -qw deep /sys/power/mem_sleep")
              # Select supported ACPI sleep only inside this disposable fixture.
              machine.succeed("echo deep > /sys/power/mem_sleep")
              suspend_results["mem_sleep"] = machine.succeed("cat /sys/power/mem_sleep").strip()
              machine.succeed(as_alice("systemctl", "--user", "start", idle_unit))
              suspend_idle_pid = machine.succeed(as_alice(
                  "systemctl", "--user", "show", "--property=MainPID", "--value", idle_unit
              )).strip()
              assert suspend_idle_pid.isdigit() and int(suspend_idle_pid) > 0
              inhibitor_deadline = time.monotonic() + 30
              inhibitors = own_sleep_inhibitors(suspend_idle_pid)
              while not inhibitors and time.monotonic() < inhibitor_deadline:
                  time.sleep(0.2)
                  inhibitors = own_sleep_inhibitors(suspend_idle_pid)
              assert inhibitors, "installed swayidle did not obtain its logind sleep/delay inhibitor"
              suspend_results["inhibitors_before"] = inhibitors
              suspend_results["inhibit_delay_max_usec"] = login_property("InhibitDelayMaxUSec")
              request_time = float(machine.succeed("cut -d ' ' -f 1 /proc/uptime").strip())
              suspend_results["request_monotonic_seconds"] = request_time
              machine.succeed(shlex.join([
                  "systemd-run", "--unit=realm-test-suspend", "--no-block",
                  "${pkgs.systemd}/bin/busctl", "--system", "--timeout=60", "call",
                  *login_object, "Suspend", "b", "false",
              ]), timeout=DIAGNOSTIC_TIMEOUT)
              suspend_deadline = time.monotonic() + 60
              monitor_status = machine.send_monitor_command("info status")
              while "suspended" not in monitor_status and time.monotonic() < suspend_deadline:
                  time.sleep(0.2)
                  monitor_status = machine.send_monitor_command("info status")
              suspend_results["suspended_monitor_status"] = monitor_status
              assert "suspended" in monitor_status, monitor_status
              suspend_results["wake_monitor_reply"] = machine.send_monitor_command("system_wakeup")
              machine.wait_until_succeeds("true", timeout=STATE_TIMEOUT)
              machine.wait_until_succeeds(
                  "systemctl show --property=ActiveState --value systemd-suspend.service | grep -qx inactive",
                  timeout=STATE_TIMEOUT,
              )
              assert lock_systemctl("is-active") == "active"
              ready_time = int(lock_systemctl(
                  "show", "--property=ActiveEnterTimestampMonotonic", "--value"
              )) / 1_000_000
              machine.succeed("journalctl --sync")
              suspend_journal = machine.succeed(
                  "journalctl --no-pager -b -n 200 -o json _SYSTEMD_UNIT=systemd-suspend.service"
              )
              entries = [json.loads(line) for line in suspend_journal.splitlines() if line.strip()]
              starts = [entry for entry in entries
                        if "Performing sleep operation 'suspend'" in entry.get("MESSAGE", "")]
              stops = [entry for entry in entries
                       if "System returned from sleep operation 'suspend'" in entry.get("MESSAGE", "")]
              assert len(starts) == 1 and len(stops) == 1, entries
              sleep_time = int(starts[0]["__MONOTONIC_TIMESTAMP"]) / 1_000_000
              assert request_time <= ready_time <= sleep_time, (request_time, ready_time, sleep_time)
              assert ready_time - request_time <= suspend_results["inhibit_delay_max_usec"] / 1_000_000
              kernel_sleep = machine.succeed(
                  "journalctl --no-pager -b -k -o json --grep='PM: suspend (entry|exit)'"
              )
              assert "PM: suspend entry (deep)" in kernel_sleep and "PM: suspend exit" in kernel_sleep
              suspend_results.update({"ready_monotonic_seconds": ready_time,
                                      "sleep_monotonic_seconds": sleep_time,
                                      "locker_processes_after_resume": lock_processes()})
              machine.screenshot("realm-suspend-resumed-locked")
              machine.send_key("meta_l-d")
              machine.succeed(
                  "for attempt in $(seq 1 20); do "
                  "if pgrep -u alice -x fuzzel; then exit 1; fi; sleep 0.1; done"
              )
              machine.send_key("ctrl-u")
              machine.send_chars("realmtest")
              machine.send_key("ret")
              machine.wait_until_succeeds(
                  as_alice("systemctl", "--user", "show", "--property=ActiveState",
                           "--value", lock_unit) + " | grep -qx inactive",
                  timeout=STATE_TIMEOUT,
              )
              suspend_results["password_unlock_after_resume"] = True
              suspend_results["inhibitors_after"] = own_sleep_inhibitors(suspend_idle_pid)
              assert suspend_results["inhibitors_after"], "swayidle did not reacquire its delay inhibitor"
              write_artifact("suspend-kernel-journal.jsonl", kernel_sleep)
          except TimeoutError as transport_error:
              suspend_results["transport_unusable"] = True
              suspend_results["transport_error"] = str(transport_error)
              raise
          finally:
              # Recovery and logging cannot depend on the guest already being awake.
              suspend_results["host_diagnostics"] = suspend_host_diagnostics()
              machine.log("suspend-roundtrip: " + json.dumps(suspend_results, sort_keys=True))
              try:
                  recovery_status = machine.send_monitor_command("info status")
                  machine.log("suspend recovery monitor: " + recovery_status)
                  if "suspended" in recovery_status:
                      machine.send_monitor_command("system_wakeup")
                  if suspend_results.get("transport_unusable"):
                      raise RuntimeError("guest cleanup skipped after host transport timeout")
                  cleanup_status, cleanup_output = machine.execute(
                      as_alice("timeout", "8", "systemctl", "--user", "stop", idle_unit),
                      timeout=DIAGNOSTIC_TIMEOUT,
                  )
                  suspend_results["idle_stop_status"] = cleanup_status
                  suspend_results["idle_stop_output"] = cleanup_output
                  _status, diagnostics = machine.execute(
                      "journalctl --no-pager -b -n 200 -o json "
                      "_SYSTEMD_UNIT=systemd-suspend.service + _SYSTEMD_UNIT=systemd-logind.service "
                      "+ _SYSTEMD_USER_UNIT=realm-idle.service + _SYSTEMD_USER_UNIT=realm-lock.service "
                      "+ _SYSTEMD_UNIT=realm-test-suspend.service",
                      timeout=DIAGNOSTIC_TIMEOUT,
                  )
                  machine.log("suspend diagnostics (last 200 entries):\n" + diagnostics)
                  write_artifact("suspend-journal.jsonl", diagnostics)
              except Exception as diagnostic_error:
                  if isinstance(diagnostic_error, TimeoutError):
                      suspend_results["transport_unusable"] = True
                  machine.log(f"suspend diagnostic collection failed: {diagnostic_error}")
              machine.log("suspend final results: " + json.dumps(suspend_results, sort_keys=True))
              write_artifact("suspend-roundtrip.json", json.dumps(suspend_results, indent=2) + "\n")
              for transport, previous_timeout in suspend_socket_timeouts:
                  transport.settimeout(previous_timeout)
          assert not suspend_results.get("transport_unusable"), suspend_results
          assert suspend_results.get("idle_stop_status") == 0, suspend_results

      # Also proves the default terminal binding is restored after unlock.
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
              f"--config={generation_root}/foot/foot-modern.ini",
              "--log-level=error",
              "--override=key-bindings.spawn-terminal=none",
              "zsh",
          ],
          generation,
      )
      zsh_pid = wait_for_single_user_process("zsh")
      zsh_environment = process_environment(zsh_pid)
      assert qt6_plugin_root in zsh_environment["QT_PLUGIN_PATH"].split(":"), (
          zsh_environment.get("QT_PLUGIN_PATH"), qt6_plugin_root
      )
      assert zsh_environment["GDK_PIXBUF_MODULE_FILE"] == svg_loader_cache, (
          zsh_environment.get("GDK_PIXBUF_MODULE_FILE"), svg_loader_cache
      )
      assert (
          zsh_environment["REALM_GENERATION"] == generation_root
      ), zsh_environment
      assert (
          zsh_environment["ZDOTDIR"] == f"{generation_root}/zsh"
      ), zsh_environment
      assert (
          zsh_environment["STARSHIP_CONFIG"]
          == f"{generation_root}/starship.toml"
      ), zsh_environment
      assert (
          zsh_environment["YAZI_CONFIG_HOME"]
          == f"{generation_root}/yazi"
      ), zsh_environment
      assert zsh_environment["GTK_THEME"] == "realm", zsh_environment
      assert (
          zsh_environment["XDG_DATA_DIRS"].split(":", 1)[0]
          == f"{generation_root}/share"
      ), zsh_environment
      assert (
          zsh_environment["QT_QPA_PLATFORMTHEME"] == "qt6ct"
      ), zsh_environment
      assert (
          zsh_environment["XDG_CONFIG_DIRS"].split(":", 1)[0]
          == generation_root
      ), zsh_environment

      prompt = machine.succeed(
          "cd /home/alice && "
          + shlex.join([
              "sudo",
              "-u",
              "alice",
              "env",
              "HOME=/home/alice",
              "TERM=foot",
              f"STARSHIP_CONFIG={generation_root}/starship.toml",
              "STARSHIP_SHELL=zsh",
              "${pkgs.zsh}/bin/zsh",
              "-dfc",
              "prompt=\"$(${pkgs.starship}/bin/starship prompt "
              + "--status 0 --cmd-duration 0 --keymap viins)\"; "
              + "print -Pnr -- \"$prompt\"",
          ])
      )
      plain_prompt = re.sub(r"\x1b\[[0-9;?]*[ -/]*[@-~]", "", prompt)
      assert plain_prompt == "alice@machine :: ~ ~% ", repr(plain_prompt)

      # OCR is useful for user-visible proof but cannot reliably join adjacent
      # differently coloured prompt spans. Keep the unmodified prompt in the
      # framebuffer, then prove the real shell is accepting and executing input
      # with a marker that does not occur contiguously in the command itself.
      machine.wait_for_text("alice@machine", timeout=OCR_TIMEOUT)
      terminal_screen = machine.get_screen_text().lower()
      assert "deprecated" not in terminal_screen, terminal_screen
      machine.screenshot("realm-terminal-prompt")
      machine.send_chars("printf 'REALM-%s-READY\\n' SHELL\n")
      machine.wait_for_text("REALM-SHELL-READY", timeout=OCR_TIMEOUT)

      gtk3_css = f"{generation_root}/share/themes/realm/gtk-3.0/gtk.css"
      gtk4_css = f"{generation_root}/share/themes/realm/gtk-4.0/gtk.css"
      qt6ct_config = f"{generation_root}/qt6ct/qt6ct.conf"
      qt6ct_colours = f"{generation_root}/qt6ct/colors/realm.conf"
      machine.succeed(
          f"cmp --silent {shlex.quote(gtk3_css)} "
          f"{shlex.quote(generation_root + '/gtk-3.0/realm.css')}"
      )
      machine.succeed(
          f"cmp --silent {shlex.quote(gtk4_css)} "
          f"{shlex.quote(generation_root + '/gtk-4.0/realm.css')}"
      )
      machine.succeed("test ! -e /home/alice/.config/qt6ct/qt6ct.conf")

      exercise_toolkit(
          "${pkgs.gtk3.dev}/bin/gtk3-widget-factory",
          "gtk3-toolkit",
          [gtk3_css],
          "gtk3-widget-factory",
          "togglebutton",
          r"(css|theme).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(css|theme)",
          screenshot="realm-gtk3-toolkit",
      )
      exercise_toolkit(
          "${pkgs.gtk4.dev}/bin/gtk4-widget-factory",
          "gtk4-toolkit",
          [gtk4_css],
          "GTK Widget Factory",
          "Page 1",
          r"(css|theme).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(css|theme)",
          screenshot="realm-gtk4-toolkit",
      )
      exercise_toolkit(
          "${pkgs.qt6Packages.qt6ct}/bin/qt6ct",
          "qt6-toolkit",
          [qt6ct_config, qt6ct_colours],
          "Qt6 Configuration Tool",
          "Qt6 Configuration Tool",
          r"(qt6ct|palette|colou?r.scheme|config).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(qt6ct|palette|colou?r.scheme|config)",
          screenshot="realm-qt6-toolkit",
      )

      # Apply B changes the next login only. Fresh launcher descendants still
      # inherit A and must actually open A's toolkit files.
      machine.succeed(as_alice(
          "XDG_CONFIG_HOME=/home/alice/.config", "realmctl", "theme", "apply"
      ))
      next_generation = machine.succeed(
          "cat /home/alice/.config/realm/generated/current"
      ).strip()
      assert next_generation != generation, (next_generation, generation)
      assert json.loads(machine.succeed(
          "cat /run/user/1000/realm/session-theme.json"
      ))["generation"] == generation

      exercise_toolkit(
          "${pkgs.gtk3.dev}/bin/gtk3-widget-factory",
          "gtk3-launcher",
          [gtk3_css],
          "gtk3-widget-factory",
          "togglebutton",
          r"(css|theme).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(css|theme)",
          screenshot="realm-gtk3-launcher",
          launcher=True,
      )
      exercise_toolkit(
          "${pkgs.gtk4.dev}/bin/gtk4-widget-factory",
          "gtk4-launcher",
          [gtk4_css],
          "GTK Widget Factory",
          "Page 1",
          r"(css|theme).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(css|theme)",
          screenshot="realm-gtk4-launcher",
          launcher=True,
      )
      exercise_toolkit(
          "${pkgs.qt6Packages.qt6ct}/bin/qt6ct",
          "qt6-launcher",
          [qt6ct_config, qt6ct_colours],
          "Qt6 Configuration Tool",
          "Qt6 Configuration Tool",
          r"(qt6ct|palette|colou?r.scheme|config).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(qt6ct|palette|colou?r.scheme|config)",
          screenshot="realm-qt6-launcher",
          launcher=True,
      )

      machine.succeed(
          "install -d -o alice -g users -m 0700 /home/alice/.config/qt6ct && "
          "printf '%s\\n' '[Appearance]' 'custom_palette=false' "
          "> /home/alice/.config/qt6ct/qt6ct.conf && "
          "chown alice:users /home/alice/.config/qt6ct/qt6ct.conf && "
          "chmod 0600 /home/alice/.config/qt6ct/qt6ct.conf"
      )
      user_qt6ct_digest = machine.succeed(
          "sha256sum /home/alice/.config/qt6ct/qt6ct.conf"
      ).split()[0]
      exercise_toolkit(
          "${pkgs.qt6Packages.qt6ct}/bin/qt6ct",
          "qt6-user-override",
          ["/home/alice/.config/qt6ct/qt6ct.conf"],
          "Qt6 Configuration Tool",
          "Qt6 Configuration Tool",
          r"(qt6ct|palette|colou?r.scheme|config).*(error|failed|invalid|not found|unable|warning)|(error|failed|invalid|warning).*(qt6ct|palette|colou?r.scheme|config)",
      )
      assert machine.succeed(
          "sha256sum /home/alice/.config/qt6ct/qt6ct.conf"
      ).split()[0] == user_qt6ct_digest

      machine.succeed(
          "install -d -o alice -g users -m 0755 /tmp/realm-yazi-proof && "
          "install -o alice -g users -m 0644 /dev/null "
          "/tmp/realm-yazi-proof/realm-yazi-visible"
      )
      btop_config = f"{generation_root}/btop/btop.conf"
      btop_config_digest = machine.succeed(
          f"sha256sum {shlex.quote(btop_config)}"
      ).split()[0]
      machine.succeed(
          f"sudo -u alice test ! -w {shlex.quote(btop_config)}"
      )
      machine.send_chars("cd /tmp/realm-yazi-proof && yazi\n")
      yazi_pid = wait_for_single_user_process("yazi")
      yazi_environment = process_environment(yazi_pid)
      assert (
          yazi_environment["REALM_GENERATION"] == generation_root
      ), yazi_environment
      assert (
          yazi_environment["YAZI_CONFIG_HOME"]
          == f"{generation_root}/yazi"
      ), yazi_environment
      machine.wait_for_text("realm-yazi-visible", timeout=OCR_TIMEOUT)

      machine.send_key("ctrl-p")
      btop_pid = wait_for_single_user_process("btop")
      btop_args = machine.succeed(
          f"tr '\\0' '\\n' < /proc/{btop_pid}/cmdline"
      ).splitlines()
      assert btop_args == [
          "btop",
          "--config",
          f"{generation_root}/btop/btop.conf",
          "--themes-dir",
          f"{generation_root}/btop/themes",
      ], btop_args
      machine.wait_for_text("CPU", timeout=OCR_TIMEOUT)
      machine.fail(
          as_alice("sh", "-c", f"printf x >> {shlex.quote(btop_config)}")
      )
      assert machine.succeed(
          f"sha256sum {shlex.quote(btop_config)}"
      ).split()[0] == btop_config_digest
      machine.send_key("q")
      machine.wait_until_succeeds(
          f"test ! -d /proc/{btop_pid}", timeout=STATE_TIMEOUT
      )
      assert machine.succeed(
          f"sha256sum {shlex.quote(btop_config)}"
      ).split()[0] == btop_config_digest
      machine.send_key("q")
      machine.wait_until_succeeds(
          f"test ! -d /proc/{yazi_pid}", timeout=STATE_TIMEOUT
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
      machine.succeed(f"test -d {shlex.quote(generation_root)}")

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
      # SPEC 0027: dispatch success alone is insufficient. Press the real
      # browser binding and require both Firefox and a compositor-owned window.
      selected_browser = machine.succeed(
          "systemd-run --user --machine=alice@ --wait --pipe --quiet --collect "
          "${pkgs.xdg-utils}/bin/xdg-settings get default-web-browser"
      ).strip()
      assert selected_browser == "realm-browser-test.desktop", selected_browser
      machine.send_key("meta_l-b")
      machine.wait_until_succeeds("pgrep -u alice -f firefox", timeout=STATE_TIMEOUT)
      browser_raw, _browser_state = wait_for_state(
          lambda response: sum(
              cell["windows"] for cell in response["data"]["orbits"]
          ) == 1,
          "default-binding browser window",
      )
      write_artifact("control-browser-state.json", browser_raw)
      machine.screenshot("realm-browser")
      # SPEC 0005 A13b: exercise real getDisplayMedia, Firefox permission,
      # the installed portal picker, browser-delivered pixels and track stop.
      browser_evidence = "/tmp/realm-browser-screencast"
      collector = shlex.join([
          "${pkgs.python3}/bin/python3",
          "${src + /packaging/nix/browser_screencast.py}",
          "--page", "${src + /packaging/nix/browser_screencast.html}",
          "--output", browser_evidence,
      ])
      machine.succeed(
          f"mkdir -p {browser_evidence}; "
          f"{collector} < /dev/null > {browser_evidence}/server.log 2>&1 & "
          f"echo $! > {browser_evidence}/server.pid"
      )
      browser_capture_passed = False
      try:
          machine.wait_until_succeeds(
              f"test -s {browser_evidence}/ready", timeout=STATE_TIMEOUT
          )
          machine.send_key("ctrl-l")
          machine.send_chars("http://127.0.0.1:8765/\n")
          machine.wait_for_text("Realm browser capture ready", timeout=OCR_TIMEOUT)
          machine.send_key("ret")
          machine.wait_for_text("Use operating system settings", timeout=OCR_TIMEOUT)
          machine.screenshot("realm-browser-permission")
          machine.send_key("alt-a")
          select_portal_output("realm-browser-output-chooser")
          machine.wait_until_succeeds(
              f"test -s {browser_evidence}/result.json || test -s {browser_evidence}/error.json",
              timeout=STATE_TIMEOUT,
          )
          machine.succeed(f"test ! -e {browser_evidence}/error.json")
          capture = json.loads(machine.succeed(f"cat {browser_evidence}/result.json"))
          assert capture["stopped"] and capture["trackStates"] == ["ended"], capture
          assert len(capture["frames"]) == 2, capture
          machine.wait_for_text("Realm capture passed and stopped", timeout=OCR_TIMEOUT)
          machine.screenshot("realm-browser-capture-stopped")
          browser_capture_passed = True
      except Exception:
          for name in ("result.json", "error.json", "server.log"):
              try:
                  code, output = machine.execute(
                      f"tail -c 16384 {browser_evidence}/{name}", timeout=DIAGNOSTIC_TIMEOUT
                  )
                  machine.log(f"browser capture {name} (exit {code}):\n{output}")
              except Exception as error:
                  machine.log(f"browser capture {name} unavailable: {error}")
          log_portal_diagnostics()
          raise
      finally:
          for command in (
              f"kill $(cat {browser_evidence}/server.pid)",
              f"${pkgs.firefox}/bin/firefox --version > {browser_evidence}/firefox-version.txt && test -s {browser_evidence}/firefox-version.txt",
          ):
              try:
                  code, output = machine.execute(command, timeout=DIAGNOSTIC_TIMEOUT)
                  if browser_capture_passed:
                      assert code == 0, (command, code, output)
              except Exception as error:
                  machine.log(f"browser capture cleanup/version unavailable: {error}")
                  if browser_capture_passed:
                      raise
          try:
              machine.copy_from_machine(browser_evidence, "browser-screencast")
          except Exception as error:
              machine.log(f"browser capture evidence copy unavailable: {error}")
              if browser_capture_passed:
                  raise
      machine.send_key("meta_l-q")
      wait_for_state(
          lambda response: sum(
              cell["windows"] for cell in response["data"]["orbits"]
          ) == 0,
          "browser window closes through the default binding",
      )

      # Shared installed-window keyboard acceptance.
      window_observations: list[dict[str, object]] = []
      window_result: dict[str, object] = {"passed": False, "observations": window_observations}

      def window_wait(predicate, description):
          observed = None

          def matches(last_try):
              nonlocal observed
              raw = machine.succeed(as_alice(
                  "python3", "-c", WINDOW_SNAPSHOT, "/run/user/1000/realm/ctl.sock"
              ), timeout=DIAGNOSTIC_TIMEOUT)
              observed = json.loads(raw)
              window_result["last_observation"] = observed
              if predicate(observed):
                  window_observations.append({"step": description, **observed})
                  return True
              if last_try:
                  machine.log(f"window control {description}: {observed!r}")
              return False

          retry(matches, timeout=STATE_TIMEOUT)
          return observed

      def window_count(value):
          return sum(len(orbit["windows"]) for orbit in value["ledger"])

      def window_screenshot(name):
          machine.screenshot(name)
          assert (Path(machine.out_dir) / (name + ".png")).stat().st_size > 0

      try:
          window_wait(lambda value: window_count(value) == 0, "empty window fixture")
          for number, letter in enumerate("ABC", 1):
              machine.send_key("meta_l-ret")
              window_wait(lambda value: window_count(value) == number, "open terminal " + letter)
              # Execute only through the real terminal's interactive shell.
              machine.send_chars(
                  "printf '\\033]0;Realm window " + letter + "\\007'; "
                  "printf '\\nRealm window " + letter + "\\nKeyboard acceptance fixture\\n'; "
                  "exec sleep infinity\n"
              )
              window_wait(lambda value: value["state"]["focused_title"] == "Realm window " + letter,
                          "title terminal " + letter)
          exercise_controls(window_wait, machine.send_key, window_screenshot)
          for remaining in (2, 1, 0):
              machine.send_key("meta_l-q")
              window_wait(lambda value: window_count(value) == remaining, "close terminal " + str(remaining))
          window_result["passed"] = True
      finally:
          machine.log(json.dumps(window_result))
          write_artifact("window-roundtrip.json", json.dumps(window_result, indent=2))

      ${lib.optionalString waybarComparison ''
      sys.path.insert(0, "${src + "/packaging/nix"}")
      waybar_probe = importlib.import_module("waybar_comparison_vm")
      waybar_probe.exercise(
          machine, as_alice, window_wait, exercise_controls,
          "${waybarFixture}", "${pkgs.waybar}/bin/waybar",
          "${pkgs.python3}/bin/python3", "${src}", imported_wayland,
      )
      ''}

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
          ("realm-gtk3-toolkit.png", "control-gtk3-toolkit-state.json"),
          ("realm-gtk4-toolkit.png", "control-gtk4-toolkit-state.json"),
          ("realm-qt6-toolkit.png", "control-qt6-toolkit-state.json"),
          ("realm-gtk3-launcher.png", "control-gtk3-launcher-state.json"),
          ("realm-gtk4-launcher.png", "control-gtk4-launcher-state.json"),
          ("realm-qt6-launcher.png", "control-qt6-launcher-state.json"),
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

      # Normal Ly logout/relogin, never a display-manager restart.
      relogin_probe = importlib.import_module("relogin_roundtrip")
      consumer_probe = importlib.import_module("consumer_roundtrip")
      relogin_result: dict[str, object] = {"passed": False}

      def relogin_snapshot(previous):
          return json.loads(machine.succeed(shlex.join([
              "python3", "-c", relogin_probe.SNAPSHOT,
              "/run/user/1000", json.dumps(previous),
          ]), timeout=DIAGNOSTIC_TIMEOUT))

      before_login = relogin_snapshot({})
      assert before_login["login"]["generation"] == generation, before_login
      assert next_generation != generation
      relogin_result["before"] = before_login
      try:
          # Retain the original exact response-drain and River-exit assertions.
          quit_raw, quit_response = control("quit")
          assert quit_response == {"reply": "ok"}, quit_response
          write_artifact("control-quit.json", quit_raw)
          machine.wait_until_succeeds(
              f"test ! -d /proc/{river_pid}", timeout=EXIT_TIMEOUT
          )
          # Ly 1.4.1 clears automatic-login mode after its first logout.
          machine.wait_for_text("logged out", timeout=OCR_TIMEOUT)
          machine.wait_for_text("password", timeout=OCR_TIMEOUT)
          machine.screenshot("relogin-greeter")
          machine.send_chars("realmtest\n")

          def transitioned(last_try):
              try:
                  after = relogin_snapshot(before_login)
                  relogin_probe.validate_transition(before_login, after, next_generation)
              except (RequestedAssertionFailed, AssertionError, ValueError) as error:
                  relogin_result["last_wait_error"] = str(error)
                  if last_try:
                      machine.log(f"relogin transition failed: {error}")
                  return False
              relogin_result["after"] = after
              return True

          retry(transitioned, timeout=STARTUP_TIMEOUT)
          wait_for_managed_window_count(0, "fresh B session")
          machine.send_key("meta_l-ret")
          wait_for_managed_window_count(1, "B terminal managed")
          next_root = f"/home/alice/.config/realm/generated/generations/{next_generation}"
          consumers = {}
          for name, executable in (("foot", "${pkgs.foot}/bin/foot"),
                                   ("zsh", "${pkgs.zsh}/bin/zsh")):
              wait_for_single_user_process(name)
              process = json.loads(machine.succeed(shlex.join([
                  "python3", "-c", consumer_probe.PROCESS, name,
              ]), timeout=DIAGNOSTIC_TIMEOUT))
              canonical_executable = machine.succeed(shlex.join([
                  "readlink", "-f", executable,
              ]), timeout=DIAGNOSTIC_TIMEOUT).strip()
              consumer_probe.assert_consumer(process, next_root, canonical_executable)
              consumers[name] = process
          assert consumers["zsh"]["parent_pid"] == consumers["foot"]["pid"], consumers
          relogin_result["consumers"] = consumers
          machine.screenshot("relogin-terminal-b")
          machine.send_key("meta_l-q")
          wait_for_managed_window_count(0, "B terminal closed")
          machine.wait_until_succeeds(
              "! pgrep -u alice -x foot && ! pgrep -u alice -x zsh", timeout=EXIT_TIMEOUT
          )
          relogin_result["passed"] = True
      finally:
          machine.log(json.dumps(relogin_result))
          write_artifact("relogin-roundtrip.json", json.dumps(relogin_result, indent=2))
    '';
  };
}
