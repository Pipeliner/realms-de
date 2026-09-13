"""Bounded xwininfo observation for the installed-session VM."""

import argparse
import json
import os
import re
import subprocess
import sys


def window_ids(tree: str, title: str) -> list[str]:
    pattern = re.compile(
        r'^\s*(0x[0-9a-fA-F]+) "' + re.escape(title) + r'":',
        re.MULTILINE,
    )
    return pattern.findall(tree)


def run_xwininfo(
    *,
    timeout_bin: str,
    xwininfo_bin: str,
    display: str,
    command_timeout: str,
    arguments: list[str],
) -> dict[str, object]:
    environment = os.environ.copy()
    environment["DISPLAY"] = display
    completed = subprocess.run(
        [
            timeout_bin,
            "--kill-after=1s",
            command_timeout,
            xwininfo_bin,
            *arguments,
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=environment,
    )
    return {"status": completed.returncode, "output": completed.stdout}


def observe_window(
    *,
    timeout_bin: str,
    xwininfo_bin: str,
    display: str,
    title: str,
    command_timeout: str,
) -> dict[str, object]:
    tree = run_xwininfo(
        timeout_bin=timeout_bin,
        xwininfo_bin=xwininfo_bin,
        display=display,
        command_timeout=command_timeout,
        arguments=["-root", "-tree"],
    )
    ids = window_ids(str(tree["output"]), title) if tree["status"] == 0 else []
    stats: list[dict[str, object]] = []
    if len(ids) == 1:
        attributes = run_xwininfo(
            timeout_bin=timeout_bin,
            xwininfo_bin=xwininfo_bin,
            display=display,
            command_timeout=command_timeout,
            arguments=["-id", ids[0], "-stats"],
        )
        attributes["window_id"] = ids[0]
        stats.append(attributes)
    viewable = (
        len(stats) == 1
        and stats[0]["status"] == 0
        and "Map State: IsViewable" in str(stats[0]["output"])
    )
    return {"tree": tree, "window_ids": ids, "stats": stats, "viewable": viewable}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--timeout-bin", required=True)
    parser.add_argument("--xwininfo-bin", required=True)
    parser.add_argument("--display", required=True)
    parser.add_argument("--title", required=True)
    parser.add_argument("--command-timeout", required=True)
    arguments = parser.parse_args(argv)
    result = observe_window(
        timeout_bin=arguments.timeout_bin,
        xwininfo_bin=arguments.xwininfo_bin,
        display=arguments.display,
        title=arguments.title,
        command_timeout=arguments.command_timeout,
    )
    json.dump(result, sys.stdout, sort_keys=True)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
