#!/usr/bin/env python3
"""Exercise the retained Yazi and Starship builds with Realm-rendered config."""
import fcntl
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path


def run_yazi(
    binary: Path,
    config: Path,
    cwd: Path,
    environment: dict[str, str],
    timeout_seconds: float = 10,
) -> bytes:
    border = re.search(r'^border_symbol = "([^"]+)"$', config.read_text(), re.MULTILINE)
    if border is None:
        raise SystemExit("selected Yazi fixture has no configured manager border")
    visible_entry = b"realm-yazi-runtime-visible"
    visible_border = border.group(1).encode()
    pid, descriptor = pty.fork()
    if pid == 0:
        os.chdir(cwd)
        os.execve(binary, [str(binary)], environment)

    fcntl.ioctl(descriptor, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    output = bytearray()
    deadline = time.monotonic() + timeout_seconds
    sent_quit = False
    status = None
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([descriptor], [], [], 0.1)
            if ready:
                try:
                    output.extend(os.read(descriptor, 65536))
                except OSError:
                    pass
            if not sent_quit and visible_entry in output and visible_border in output:
                os.write(descriptor, b"q")
                sent_quit = True
            waited, candidate_status = os.waitpid(pid, os.WNOHANG)
            if waited == pid:
                status = candidate_status
                break
        if status is None:
            os.kill(pid, 9)
            _, status = os.waitpid(pid, 0)
            raise SystemExit("selected Yazi did not terminate after its quit input")
    finally:
        os.close(descriptor)
    if not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
        raise SystemExit(f"selected Yazi exited unsuccessfully: status {status}")
    lowered = bytes(output).lower()
    for diagnostic in (b"failed to load", b"failed to parse", b"unknown field", b"invalid type"):
        if diagnostic in lowered:
            raise SystemExit(f"selected Yazi emitted a configuration diagnostic: {diagnostic!r}")
    if not config.is_file():
        raise SystemExit("selected Yazi fixture lost its rendered theme")
    screen = bytes(output).decode("utf-8", errors="replace")
    if visible_entry.decode() not in screen:
        raise SystemExit("selected Yazi did not render the controlled directory entry")
    if border.group(1) not in screen:
        raise SystemExit("selected Yazi did not render the configured manager border")
    return bytes(output)


def self_test_timeout_reaps_child() -> None:
    with tempfile.TemporaryDirectory(prefix="realm-yazi-timeout-") as temporary:
        root = Path(temporary)
        pid_file = root / "pid"
        child = root / "ignores-q"
        child.write_text(
            f"#!{sys.executable}\n"
            "import os, time\n"
            "from pathlib import Path\n"
            "Path(os.environ['REALM_TEST_PID_FILE']).write_text(str(os.getpid()))\n"
            "print('realm-yazi-runtime-visible R', flush=True)\n"
            "while True:\n"
            "    time.sleep(1)\n",
            encoding="utf-8",
        )
        child.chmod(0o755)
        config = root / "theme.toml"
        config.write_text('border_symbol = "R"\n', encoding="utf-8")
        environment = os.environ.copy()
        environment["REALM_TEST_PID_FILE"] = str(pid_file)
        failure = None
        try:
            run_yazi(child, config, root, environment, timeout_seconds=0.3)
        except SystemExit as error:
            failure = str(error)

        child_pid = int(pid_file.read_text(encoding="utf-8"))
        leaked = False
        try:
            waited, _ = os.waitpid(child_pid, os.WNOHANG)
            if waited == 0:
                leaked = True
                os.kill(child_pid, 9)
                os.waitpid(child_pid, 0)
        except ChildProcessError:
            pass

        expected = "selected Yazi did not terminate after its quit input"
        if failure != expected or leaked:
            raise SystemExit(
                "Yazi timeout regression did not fail within its bound and reap its child"
            )


def main() -> None:
    if sys.argv[1:] == ["--self-test"]:
        self_test_timeout_reaps_child()
        return
    if len(sys.argv) != 6:
        raise SystemExit(
            "usage: test-tool-runtime.py REALMCTL YAZI YA STARSHIP REALM_SOURCE"
        )
    realmctl, yazi, ya, starship, source = map(Path, sys.argv[1:])
    for binary in (realmctl, yazi, ya, starship):
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise SystemExit(f"runtime fixture executable is missing: {binary}")

    with tempfile.TemporaryDirectory(prefix="realm-tool-runtime-") as temporary:
        root = Path(temporary)
        config_root = root / "config"
        (config_root / "realm").mkdir(parents=True, mode=0o700)
        apply = subprocess.run(
            [str(realmctl), "theme", "apply", "--config-root", str(config_root)],
            cwd=source,
            check=True,
            capture_output=True,
            text=True,
        )
        match = re.fullmatch(
            r"generation ([0-9a-f]{32}) selected for future launches\n", apply.stdout
        )
        if match is None:
            raise SystemExit(f"theme apply returned an unexpected receipt: {apply.stdout!r}")
        generation = config_root / "realm" / "generated" / "generations" / match.group(1)
        yazi_theme = generation / "yazi" / "theme.toml"
        starship_config = generation / "starship.toml"
        if not yazi_theme.is_file() or not starship_config.is_file():
            raise SystemExit("theme generation omitted a selected-tool output")

        yazi_config = root / "yazi"
        yazi_config.mkdir()
        shutil.copyfile(yazi_theme, yazi_config / "theme.toml")
        home = root / "home"
        work = home / "work"
        work.mkdir(parents=True)
        (work / "realm-yazi-runtime-visible").write_text("fixture\n", encoding="utf-8")
        environment = os.environ.copy()
        environment.update(
            {
                "HOME": str(home),
                "TERM": "xterm-256color",
                "XDG_CACHE_HOME": str(root / "cache"),
                "XDG_CONFIG_HOME": str(config_root),
                "YAZI_CONFIG_HOME": str(yazi_config),
            }
        )
        run_yazi(yazi, yazi_config / "theme.toml", work, environment)

        starship_environment = environment.copy()
        starship_environment.update(
            {
                "STARSHIP_CONFIG": str(starship_config),
                "STARSHIP_SHELL": "zsh",
            }
        )
        prompt = subprocess.run(
            [
                str(starship),
                "prompt",
                "--status",
                "0",
                "--cmd-duration",
                "2500",
                "--keymap",
                "viins",
            ],
            cwd=work,
            env=starship_environment,
            check=True,
            capture_output=True,
        )
        if prompt.stderr:
            raise SystemExit(f"selected Starship emitted a configuration diagnostic: {prompt.stderr!r}")
        plain_prompt = re.sub(rb"\x1b\[[0-9;?]*[ -/]*[@-~]", b"", prompt.stdout)
        if b" :: " not in plain_prompt or b"~%" not in plain_prompt:
            raise SystemExit("selected Starship did not render Realm's configured prompt features")


if __name__ == "__main__":
    main()
