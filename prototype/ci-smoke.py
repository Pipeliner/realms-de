#!/usr/bin/env python3
"""CI-only real Sway session test; installs/builds nothing itself.

wtype sends virtual keyboard events through Wayland. swaymsg observes the
result and changes only output size; it never substitutes for tested bindings.
"""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def walk(node):
    yield node
    for child in node.get("nodes", []) + node.get("floating_nodes", []):
        yield from walk(child)


def workspace_terminals(tree, number):
    workspace = next((node for node in walk(tree)
                      if node.get("type") == "workspace" and node.get("num") == number), None)
    return [] if workspace is None else [node for node in walk(workspace)
                                         if node.get("app_id") == "foot"]


def process_ids(processes, name):
    return {int(fields[0]) for line in processes.splitlines()
            if len(fields := line.split()) >= 3 and fields[0].isdigit()
            and fields[2].casefold() == name.casefold()}


def process_windows(tree, pids):
    return [node for node in walk(tree) if node.get("pid") in pids
            and (node.get("app_id") or node.get("window"))]


def shutdown(session, run):
    reply = run(["swaymsg", "-r", "-t", "command", "exit"], check=False)
    if reply.returncode:
        if reply.returncode != 1 or "Unable to receive IPC response" not in reply.stderr:
            raise RuntimeError(f"Sway exit IPC failed: {reply.stderr}")
    elif not all(item.get("success") for item in json.loads(reply.stdout)):
        raise RuntimeError(f"Sway rejected exit: {reply.stdout}")
    status = session.wait(timeout=10)
    if status != 0:
        raise RuntimeError(f"Sway exited with status {status}")
    return status


def main():
    prototype = Path(__file__).resolve().parent
    launcher = prototype / "realm-prototype"
    assert launcher.is_file(), "prototype launcher is missing"
    evidence = Path(sys.argv[1] if len(sys.argv) > 1 else "prototype-evidence").resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    runtime = evidence / "runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
    runtime.chmod(0o700)
    env = os.environ.copy()
    env.update({
        "XDG_RUNTIME_DIR": str(runtime),
        "XDG_CONFIG_HOME": str(runtime / "config"),
        "XDG_CACHE_HOME": str(runtime / "cache"),
        "XDG_DATA_HOME": str(runtime / "data"),
        "WLR_BACKENDS": "headless",
        "WLR_HEADLESS_OUTPUTS": "1",
        "WLR_RENDERER": "pixman",
        "WLR_LIBINPUT_NO_DEVICES": "1",
        "TERM": "xterm-256color",
    })
    for name in ("WAYLAND_DISPLAY", "SWAYSOCK", "DISPLAY"):
        env.pop(name, None)

    with (evidence / "commands.log").open("w") as commands:
        def run(argv, check=True):
            commands.write("$ " + repr(argv) + "\n")
            commands.flush()
            result = subprocess.run(argv, env=env, text=True, capture_output=True, timeout=15)
            commands.write(result.stdout + result.stderr)
            commands.flush()
            if check and result.returncode:
                raise RuntimeError(f"{argv!r}: exit {result.returncode}: {result.stderr}")
            return result

        def ipc(kind, *args):
            return json.loads(run(["swaymsg", "-r", "-t", kind, *args]).stdout)

        def command(text):
            reply = ipc("command", text)
            assert all(item.get("success") for item in reply), reply

        def wait_for(predicate, description):
            deadline = time.monotonic() + 25
            while time.monotonic() < deadline:
                if session.poll() is not None:
                    raise RuntimeError(f"Sway exited before {description}; see sway.log")
                value = predicate()
                if value:
                    return value
                time.sleep(0.15)
            raise AssertionError(f"Timed out waiting for {description}")

        def key(name, shift=False):
            argv = ["wtype", "-M", "logo"]
            if shift:
                argv += ["-M", "shift"]
            argv += ["-P", name, "-p", name]
            if shift:
                argv += ["-m", "shift"]
            argv += ["-m", "logo"]
            run(argv)

        def terminals():
            return [node for node in walk(ipc("get_tree")) if node.get("app_id") == "foot"]

        def label_terminal(title, text):
            # Mapping a Foot window precedes shell readiness. Allow input setup,
            # then require the shell's OSC-title feedback before continuing.
            run(["wtype", "-s", "250", "-d", "5",
                 f"PS1='$ '; clear; printf '\\033]0;{title}\\007{text}'\n"])
            wait_for(lambda: any(node.get("name") == title for node in terminals()),
                     f"terminal successfully printing {title}")

        def focused():
            return next((node["id"] for node in terminals() if node.get("focused")), None)

        def workspace():
            return next(item["num"] for item in ipc("get_workspaces") if item["focused"])

        def process_list():
            return run(["ps", "-eo", "pid,ppid,comm,args"]).stdout

        run([str(launcher), "--validate"])
        with (evidence / "sway.log").open("w") as log:
            session = subprocess.Popen([str(launcher), "--debug"], env=env,
                                       stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                def discover():
                    sockets = list(runtime.glob("sway-ipc.*.sock"))
                    displays = [path for path in runtime.glob("wayland-*")
                                if not path.name.endswith(".lock")]
                    if not sockets or not displays:
                        return False
                    env["SWAYSOCK"] = str(sockets[0])
                    env["WAYLAND_DISPLAY"] = displays[0].name
                    return True

                wait_for(discover, "Sway IPC and Wayland sockets")
                outputs = ipc("get_outputs")
                active = next(item for item in outputs if item["active"])
                command(f'output {active["name"]} mode 1280x800')
                (evidence / "version.txt").write_text(run(["sway", "--version"]).stdout)
                (evidence / "outputs.json").write_text(json.dumps(ipc("get_outputs"), indent=2))

                key("1")
                wait_for(lambda: workspace() == 1, "workspace 1 binding")
                key("Return")
                wait_for(lambda: len(terminals()) == 1, "first Foot window from Mod+Return")
                label_terminal("REALM-FIRST-READY",
                               "REALM / SWAY PROTOTYPE\\nTerminal opened by Mod+Return\\n")
                key("Return")
                wait_for(lambda: len(terminals()) == 2, "second Foot window from Mod+Return")
                label_terminal("REALM-SECOND-READY",
                               "SECOND TERMINAL\\nTwo real Wayland windows\\n"
                               "Focus and workspaces tested by keyboard\\n")

                right = focused()
                key("Left")
                left = wait_for(lambda: focused() if focused() != right else None,
                                "focus-left binding changing focused Foot")
                key("Right")
                wait_for(lambda: focused() == right, "focus-right binding restoring focused Foot")

                key("2", shift=True)
                key("2")
                wait_for(lambda: workspace() == 2 and focused() == right,
                         "move-to-workspace and workspace-2 bindings")
                key("1", shift=True)
                key("1")
                wait_for(lambda: workspace() == 1 and
                         len(workspace_terminals(ipc("get_tree"), 1)) == 2,
                         "return both Foot windows to workspace 1")

                assert not process_ids(process_list(), "fuzzel"), "Fuzzel existed before its binding"
                key("d")
                fuzzel_pids = wait_for(lambda: process_ids(process_list(), "fuzzel"),
                                       "configured launcher binding starting Fuzzel")
                (evidence / "launcher-processes.txt").write_text(process_list())
                time.sleep(0.25)
                run(["grim", "-o", active["name"], str(evidence / "launcher.png")])
                run(["wtype", "-s", "100", "-P", "Escape", "-p", "Escape"])
                wait_for(lambda: not process_ids(process_list(), "fuzzel"),
                         "Escape dismissing Fuzzel")

                assert not process_ids(process_list(), "thunar"), "Thunar existed before its binding"
                key("e")
                thunar_pids = wait_for(lambda: process_ids(process_list(), "thunar"),
                                       "configured files binding starting Thunar")
                thunar_windows = wait_for(lambda: process_windows(ipc("get_tree"), thunar_pids),
                                          "real mapped Thunar window matching its process PID")
                thunar_id = thunar_windows[0]["id"]
                wait_for(lambda: any(node.get("focused") for node in
                                     process_windows(ipc("get_tree"), thunar_pids)),
                         "Thunar receiving focus before close binding")
                (evidence / "files-processes.txt").write_text(process_list())
                (evidence / "files-tree.json").write_text(json.dumps(ipc("get_tree"), indent=2))
                time.sleep(0.25)
                run(["grim", "-o", active["name"], str(evidence / "files.png")])
                key("q", shift=True)
                wait_for(lambda: not process_windows(ipc("get_tree"), thunar_pids),
                         "configured close binding unmapping Thunar")
                wait_for(lambda: focused() is not None and workspace() == 1 and
                         len(workspace_terminals(ipc("get_tree"), 1)) == 2,
                         "restoring the two-terminal desktop after application checks")

                bars = ipc("get_bar_config")
                assert bars, "No configured Sway bar"
                bar_configs = [ipc("get_bar_config", bar) for bar in bars]
                processes = run(["ps", "-eo", "pid,ppid,comm,args"]).stdout
                assert any(line.split()[2:3] == ["swaybar"] for line in processes.splitlines()), \
                    "Configured swaybar process is missing"
                assert any(line.split()[2:3] == ["i3status"] for line in processes.splitlines()), \
                    "Configured i3status process is missing"
                (evidence / "processes.txt").write_text(processes)
                (evidence / "bars.json").write_text(json.dumps(bar_configs, indent=2))
                (evidence / "tree.json").write_text(json.dumps(ipc("get_tree"), indent=2))
                (evidence / "workspaces.json").write_text(json.dumps(ipc("get_workspaces"), indent=2))
                time.sleep(1)
                screenshot = evidence / "desktop.png"
                run(["grim", "-o", active["name"], str(screenshot)])
                from PIL import Image
                with Image.open(screenshot) as image:
                    image.load()
                    assert image.format == "PNG" and image.size == (1280, 800), image.size
                    extrema = image.convert("RGB").getextrema()
                    assert max(high - low for low, high in extrema) > 32, "Screenshot is blank"
                summary = {
                    "result": "pass", "revision": os.environ.get("GITHUB_SHA", "unknown"),
                    "backend": "headless", "renderer": "pixman",
                    "terminal_launch": "Mod4+Return virtual keyboard binding twice",
                    "foot_window_ids": [left, right], "focus_bindings": ["Mod4+Left", "Mod4+Right"],
                    "workspace_bindings": ["Mod4+Shift+2", "Mod4+2", "Mod4+Shift+1", "Mod4+1"],
                    "launcher_binding": "Mod4+d launched Fuzzel; Escape dismissed it",
                    "fuzzel_process_ids": sorted(fuzzel_pids),
                    "files_binding": "Mod4+e mapped Thunar; Mod4+Shift+q closed it",
                    "thunar_process_ids": sorted(thunar_pids), "thunar_window_id": thunar_id,
                    "application_screenshots": ["launcher.png", "files.png"],
                    "screenshot": "desktop.png", "bar": "swaybar + i3status processes observed",
                    "limits": "No hardware, PAM, backlight, portals or suspend verification",
                }
                summary["compositor_exit_status"] = shutdown(session, run)
                (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
                print(json.dumps(summary, indent=2))
            finally:
                if session.poll() is None:
                    os.killpg(session.pid, signal.SIGTERM)
                    try:
                        session.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(session.pid, signal.SIGKILL)
                        session.wait()


if __name__ == "__main__":
    main()
