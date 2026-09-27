"""One CI-only Waybar comparison in the existing installed Realm VM."""

import datetime as dt
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import sys
import time

import waybar_compare


def selected_palette_path(runtime):
    record = json.loads((Path(runtime) / "realm/session-theme.json").read_text())
    root = Path(record["config_root"])
    generation = record["generation"]
    if not root.is_absolute() or generation in ("", ".", "..") or "/" in generation:
        raise ValueError("invalid selected generation record")
    palette = root / "realm/generated/generations" / generation / "realm/palette.toml"
    if not palette.is_file():
        raise FileNotFoundError(palette)
    return palette


def process_sample(uid):
    """Read raw ticks and RSS from the named guest processes."""
    identities = {}
    for directory in Path("/proc").iterdir():
        if not directory.name.isdigit():
            continue
        try:
            if directory.stat().st_uid != uid:
                continue
            argv = (directory / "cmdline").read_bytes().replace(b"\0", b" ").decode(errors="replace")
            comm = (directory / "comm").read_text().strip()
            if "waybar_compare.py" in argv and comm.startswith("python"):
                name = "adapter"
            elif comm in ("waybar", "realm-bar", "realm-wm"):
                name = comm
            else:
                continue
            stat = (directory / "stat").read_text().rsplit(")", 1)[1].split()
            status = (directory / "status").read_text().splitlines()
            rss = next((int(line.split()[1]) for line in status if line.startswith("VmRSS:")), 0)
            identities[name] = {"pid": int(directory.name), "ticks": int(stat[11]) + int(stat[12]), "rss_kib": rss}
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
    return {"clock_ticks_per_second": os.sysconf("SC_CLK_TCK"), "processes": identities}


def cpu_delta(before, after, ticks_per_second):
    cpu = {}
    for name, start in before.items():
        end = after.get(name)
        if end is not None and end["pid"] == start["pid"]:
            cpu[name] = (end["ticks"] - start["ticks"]) / ticks_per_second
    return {
        "start": before, "end": after, "cpu_seconds": cpu,
        "rss_caveat": "Per-process RSS counts shared pages more than once; sums double count and include VM noise.",
    }


def exercise(machine, as_alice, window_wait, exercise_controls, fixture, waybar, python,
             source, wayland_display):
    """Run the baseline and hybrid in one VM; all actions use actual keyboard input."""
    evidence = Path(machine.out_dir) / "waybar-comparison"
    evidence.mkdir(exist_ok=True)
    result = {
        "schema": "realm-waybar-comparison/v1", "passed": False,
        "environment": {"nixos": machine.succeed("nixos-version").strip(),
                        "kernel": machine.succeed("uname -srmo").strip(),
                        "river": machine.succeed("river -version").strip(),
                        "waybar": machine.succeed(shlex.quote(waybar) + " --version").strip()},
        "scope": "same logged-in NixOS VM; hybrid retains realm-bar for help",
        "not_measured": ["machine cold boot", "true input-to-present latency", "wakeups", "physical battery behavior"],
        "variants": {}, "failures": [],
        "visual_review": "pending inspection of retained framebuffer screenshots",
    }
    runtime = "/run/user/1000"
    candidate_units = []
    guest_probe = source + "/packaging/nix/waybar_comparison_vm.py"
    adapter = source + "/packaging/nix/waybar_compare.py"

    def user(*argv):
        return machine.succeed(as_alice(*argv), timeout=dt.timedelta(seconds=20))

    def capture(name):
        machine.screenshot(name)
        original = Path(machine.out_dir) / (name + ".png")
        assert original.stat().st_size > 0, original
        shutil.copyfile(original, evidence / original.name)
        return original.name

    def sample():
        return json.loads(machine.succeed(shlex.join([
            python, guest_probe, "--sample", "1000"
        ]), timeout=dt.timedelta(seconds=10)))

    def idle(variant):
        time.sleep(10)
        samples = []
        for index in range(31):
            samples.append({"host_monotonic_ns": time.monotonic_ns(), **sample()})
            if index != 30:
                time.sleep(1)
        start, end = samples[0], samples[-1]
        expected = {"realm-bar", "realm-wm"}
        if variant == "hybrid":
            expected.update(("waybar", "adapter"))
        for measured in samples:
            assert expected <= measured["processes"].keys(), (variant, measured)
        return {"settle_seconds": 10, "sample_seconds": 30, "cadence_seconds": 1,
                "samples": samples,
                "delta": cpu_delta(start["processes"], end["processes"], start["clock_ticks_per_second"])}

    def launch(label, start, expected):
        trials = []
        for trial in range(3):
            started = time.monotonic_ns()
            pid = start()
            deadline = started + 30_000_000_000
            remaining = (deadline - time.monotonic_ns()) / 1e9
            assert remaining > 0, (label, trial, "launch command exceeded deadline")
            machine.wait_for_text(expected, timeout=dt.timedelta(seconds=remaining))
            frame = capture(f"{label}-launch-{trial + 1}")
            ended = time.monotonic_ns()
            assert ended <= deadline, (label, trial, "launch-to-visible probe exceeded 30s")
            trials.append({"trial": trial + 1, "start_monotonic_ns": started,
                           "screenshot_monotonic_ns": ended, "upper_bound_seconds": (ended-started)/1e9,
                           "expected_ocr_text": expected, "screenshot": frame,
                           "process_pid": pid})
        return trials

    def transition_snapshot(name):
        observation = window_wait(lambda value: True, name)
        screenshot = capture(name)
        return {"state": observation, "screenshot": screenshot,
                "host_monotonic_ns": time.monotonic_ns()}

    def measured_change(name, chord, predicate, ocr_text):
        dispatched = time.monotonic_ns()
        machine.send_key(chord)
        observation = window_wait(predicate, name)
        observed = time.monotonic_ns()
        revision = observation["state"]["revision"]
        deadline = dispatched + 30_000_000_000
        trace_entry = None
        while time.monotonic_ns() < deadline:
            raw = user("cat", runtime + "/waybar-comparison/adapter.jsonl")
            entries = [json.loads(line) for line in raw.splitlines()]
            trace_entry = next((entry for entry in reversed(entries)
                                if entry["revision"] == revision), None)
            if trace_entry is not None:
                break
            time.sleep(0.1)
        assert trace_entry is not None, (name, "adapter did not flush matching revision")
        assert trace_entry["text"] == waybar_compare.render_state(observation["state"])["text"], \
            (name, trace_entry, observation["state"])
        remaining = (deadline - time.monotonic_ns()) / 1e9
        assert remaining > 0, (name, "state update exceeded deadline")
        machine.wait_for_text(ocr_text, timeout=dt.timedelta(seconds=remaining))
        frame = capture(name)
        shown = time.monotonic_ns()
        assert shown <= deadline, (name, "screenshot exceeded deadline")
        return {"key_dispatch_host_monotonic_ns": dispatched,
                "socket_observation_host_monotonic_ns": observed,
                "first_matching_ocr_screenshot_host_monotonic_ns": shown,
                "screenshot": frame, "ocr_text": ocr_text,
                "state": observation, "adapter": trace_entry,
                "probe_upper_bound_seconds": (shown - dispatched) / 1e9,
                "adapter_receive_to_flush_seconds":
                    (trace_entry["flushed_monotonic_ns"] - trace_entry["received_monotonic_ns"]) / 1e9,
                "precision_caveat": "OCR cadence and screenshot capture are included; this is not input-to-present latency."}

    try:
        record = json.loads(user(python, guest_probe, "--record", runtime))
        record["wayland_display"] = wayland_display
        result["login_selection"] = record
        palette = record["palette"]
        css = runtime + "/waybar-comparison/style.css"
        user("mkdir", "-p", runtime + "/waybar-comparison")
        user(python, adapter, "--css", palette)  # fail before launch if selected palette cannot render
        render_command = shlex.join([python, adapter, "--css", palette]) + " > " + shlex.quote(css)
        machine.succeed(as_alice("sh", "-c", render_command))
        result["selected_css"] = {"palette": palette,
                                   "sha256": machine.succeed("sha256sum " + shlex.quote(css)).split()[0],
                                   "path": css}
        result["battery_devices"] = machine.succeed("ls -1 /sys/class/power_supply || true").splitlines()
        result["battery_hardware_observed"] = any(
            device.startswith("BAT") for device in result["battery_devices"])
        result["fontconfig"] = {
            "realm_rune": user("fc-match", "-f", "%{family}: %{file}", "monospace:charset=16a0"),
            "long_title": user("fc-match", "-f", "%{family}: %{file}", "monospace:charset=754c"),
        }

        # One fixture workload remains open for baseline and hybrid.
        window_wait(lambda value: sum(len(cell["windows"]) for cell in value["ledger"]) == 0,
                    "comparison starts empty")
        for number, letter in enumerate("ABC", 1):
            machine.send_key("meta_l-ret")
            window_wait(lambda value: sum(len(cell["windows"]) for cell in value["ledger"]) == number,
                        "comparison terminal " + letter)
            machine.send_chars("printf '\\033]0;Realm window " + letter + "\\007'; "
                               "printf 'Realm window " + letter + "\\n'; exec sleep infinity\n")
            window_wait(lambda value: value["state"]["focused_title"] == "Realm window " + letter,
                        "comparison title " + letter)

        def restart_bar():
            previous = machine.succeed("pgrep -u alice -xo realm-bar").strip()
            machine.succeed("systemctl --user --machine=alice@ restart realm-bar.service",
                            timeout=dt.timedelta(seconds=15))
            machine.wait_until_succeeds(
                f'test "$(pgrep -u alice -xo realm-bar)" != {shlex.quote(previous)}',
                timeout=dt.timedelta(seconds=15))
            return machine.succeed("pgrep -u alice -xo realm-bar").strip()

        baseline = {"launch_probe": launch("baseline", restart_bar, "hecate")}
        baseline["idle"] = idle("baseline")
        baseline["before_controls"] = transition_snapshot("baseline-before-controls")
        exercise_controls(window_wait, machine.send_key, lambda name: capture("baseline-" + name))
        baseline["after_controls"] = transition_snapshot("baseline-after-controls")
        result["variants"]["baseline"] = baseline

        def start_waybar():
            if candidate_units:
                machine.succeed("systemctl --user --machine=alice@ stop " + candidate_units[-1] + ".service")
                machine.wait_until_succeeds("! pgrep -u alice -x waybar",
                                            timeout=dt.timedelta(seconds=15))
            unit = "realm-waybar-comparison-" + str(len(candidate_units) + 1)
            candidate_units.append(unit)
            machine.succeed(shlex.join([
                "systemd-run", "--user", "--machine=alice@", "--unit=" + unit,
                "--collect", "--quiet", "--setenv=WAYLAND_DISPLAY=" + record["wayland_display"],
                "--setenv=XDG_RUNTIME_DIR=" + runtime, waybar,
                "-c", fixture + "/config.json", "-s", css,
            ]), timeout=dt.timedelta(seconds=15))
            machine.wait_until_succeeds("pgrep -u alice -x waybar",
                                        timeout=dt.timedelta(seconds=15))
            return machine.succeed("pgrep -u alice -xo waybar").strip()

        hybrid = {"launch_probe": launch("hybrid", start_waybar, "cpu")}
        hybrid["idle"] = idle("hybrid")
        machine.wait_for_text("mem", timeout=dt.timedelta(seconds=30))
        hybrid["commodity_modules_screenshot"] = capture("hybrid-commodity-modules")
        hybrid["before_controls"] = transition_snapshot("hybrid-before-controls")
        focused = hybrid["before_controls"]["state"]["state"]["focused_title"]
        hybrid["state_changes"] = [
            measured_change("hybrid-focus-next", "meta_l-j",
                            lambda value: value["state"]["focused_title"] != focused,
                            "Realm window"),
            measured_change("hybrid-mono", "meta_l-m",
                            lambda value: value["state"]["layout"] == "mono", "MONO"),
            measured_change("hybrid-triptych", "meta_l-t",
                            lambda value: value["state"]["layout"] == "triptych", "TRIPTYCH"),
        ]
        # Keep the reusable window-control fixture's expected focus/order.
        machine.send_key("meta_l-k")
        window_wait(lambda value: value["state"]["focused_title"] == focused,
                    "comparison focus returned")
        exercise_controls(window_wait, machine.send_key, lambda name: capture("hybrid-" + name))
        hybrid["after_controls"] = transition_snapshot("hybrid-after-controls")
        machine.send_key("meta_l-r")
        hybrid["resize_mode"] = window_wait(
            lambda value: value["state"]["mode"] == "resize", "comparison resize mode")
        hybrid["resize_mode_screenshot"] = capture("hybrid-resize-mode")
        machine.send_key("meta_l-esc")
        hybrid["nav_mode"] = window_wait(
            lambda value: value["state"]["mode"] == "nav", "comparison navigation mode")
        hybrid["nav_mode_screenshot"] = capture("hybrid-nav-mode")
        machine.send_key("meta_l-f")
        hybrid["fullscreen"] = transition_snapshot("hybrid-fullscreen")
        active = [cell for cell in hybrid["fullscreen"]["state"]["ledger"] if cell["active"]]
        assert len(active) == 1 and active[0]["fullscreen"] is not None, active
        machine.send_key("meta_l-f")
        hybrid["unfullscreen"] = transition_snapshot("hybrid-unfullscreen")
        active = [cell for cell in hybrid["unfullscreen"]["state"]["ledger"] if cell["active"]]
        assert len(active) == 1 and active[0]["fullscreen"] is None, active
        prior_title = hybrid["after_controls"]["state"]["state"]["focused_title"]
        long_title = "界" * 40 + " Realm title"
        escaped = "".join("\\x%02x" % byte for byte in long_title.encode("utf-8"))
        machine.send_chars("printf '\\033]0;" + escaped + "\\007'\n")
        hybrid["long_nonascii_title"] = window_wait(
            lambda value: value["state"]["focused_title"] == long_title,
            "long non-ASCII terminal title")
        title_deadline = time.monotonic() + 10
        while True:
            trace_lines = user("cat", runtime + "/waybar-comparison/adapter.jsonl").splitlines()
            if any("界" * 31 + "…" in json.loads(line)["text"] for line in trace_lines):
                break
            assert time.monotonic() < title_deadline, "bounded non-ASCII title did not reach adapter"
            time.sleep(0.1)
        hybrid["long_nonascii_screenshot"] = capture("hybrid-long-nonascii-title")
        machine.send_chars("printf '\\033]0;" + prior_title + "\\007'\n")
        window_wait(lambda value: value["state"]["focused_title"] == prior_title,
                    "restore window title")

        # A second actual output scale; record the Wayland mode and restore it.
        display = record["wayland_display"]
        output_text = user("env", "WAYLAND_DISPLAY=" + display, "wlr-randr")
        output = output_text.splitlines()[0].split()[0]
        scale = re.search(r"^\s*Scale:\s*([0-9.]+)", output_text, re.MULTILINE)
        assert scale, output_text
        original_scale = scale.group(1)
        try:
            user("env", "WAYLAND_DISPLAY=" + display, "wlr-randr", "--output", output,
                 "--scale", "1.25")
            hybrid["additional_scale"] = {"output": output, "configured_scale": original_scale,
                                           "comparison_scale": "1.25",
                                           "screenshot": capture("hybrid-output-scale-1_25")}
        finally:
            user("env", "WAYLAND_DISPLAY=" + display, "wlr-randr", "--output", output,
                 "--scale", original_scale)
        result["variants"]["hybrid"] = hybrid
        result["adapter_trace"] = user("cat", runtime + "/waybar-comparison/adapter.jsonl")
        (evidence / "adapter.jsonl").write_text(result["adapter_trace"])
        result["adapter_trace"] = "adapter.jsonl"
        result["passed"] = True
    except Exception as error:
        result["failures"].append(str(error))
        raise
    finally:
        status, output = (0, "")
        for unit in reversed(candidate_units):
            stopped, message = machine.execute("systemctl --user --machine=alice@ stop " + unit + ".service",
                                               timeout=dt.timedelta(seconds=15))
            status = max(status, stopped)
            output += unit + ": " + message + "\n"
        result["cleanup"] = {"stop_status": status, "stop_output": output,
                             "units": candidate_units}
        if status != 0:
            result["passed"] = False
            result["failures"].append("candidate stop failed")
        try:
            result["cleanup"]["candidate_remaining"] = machine.succeed(
                "pgrep -u alice -x waybar || true").strip()
            result["cleanup"]["adapter_remaining"] = machine.succeed(
                "pgrep -u alice -f waybar_compare.py || true").strip()
            if result["cleanup"]["candidate_remaining"] or result["cleanup"]["adapter_remaining"]:
                result["passed"] = False
                result["failures"].append("candidate or adapter survived stop")
            machine.send_key("meta_l-w")
            window_wait(lambda value: isinstance(value["state"]["whichkey"], bool),
                        "baseline help responds after candidate stop")
            machine.send_key("meta_l-j")
            window_wait(lambda value: bool(value["state"]["focused_title"]),
                        "baseline focus responds after candidate stop")
            result["cleanup"]["baseline_help_focus"] = True
        except Exception as error:
            result["passed"] = False
            result["failures"].append("baseline recovery: " + str(error))
        for remaining in (2, 1, 0):
            try:
                machine.send_key("meta_l-q")
                window_wait(lambda value: sum(len(cell["windows"]) for cell in value["ledger"]) == remaining,
                            "close comparison terminal " + str(remaining))
            except Exception as error:
                result["passed"] = False
                result["failures"].append("fixture cleanup: " + str(error))
                break
        (evidence / "result.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
        machine.log("waybar comparison summary: " + json.dumps({
            "passed": result["passed"], "failures": result["failures"],
            "cleanup": result["cleanup"], "artifact": str(evidence / "result.json"),
        }, ensure_ascii=False))
    assert result["passed"], result["failures"]


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--record":
        runtime = Path(sys.argv[2])
        palette = selected_palette_path(runtime)
        record = json.loads((runtime / "realm/session-theme.json").read_text())
        print(json.dumps({"generation": record["generation"], "palette": str(palette)}))
    elif len(sys.argv) == 3 and sys.argv[1] == "--sample":
        print(json.dumps(process_sample(int(sys.argv[2]))))
    else:
        raise SystemExit("usage: --record RUNTIME | --sample UID")
