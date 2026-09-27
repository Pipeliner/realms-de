#!/usr/bin/env python3
"""Capture protocol-v2 Hello and GetState frames from an installed session."""

import json
import socket
import sys
from pathlib import Path


def exchange(socket_path: Path) -> list[object]:
    requests = [
        {"cmd": "hello", "arg": {"version": 2, "client": "realm-native-vm"}},
        {"cmd": "get-state"},
    ]
    responses = []
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(5)
        connection.connect(str(socket_path))
        with connection.makefile("rwb", buffering=0) as stream:
            for request in requests:
                stream.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
                frame = stream.readline(1024 * 1024)
                if not frame.endswith(b"\n"):
                    raise ValueError("control response was absent, oversized, or unterminated")
                responses.append(json.loads(frame))
    return responses


def main(arguments: list[str]) -> int:
    if len(arguments) != 2:
        print("usage: control_get_state.py SOCKET OUTPUT", file=sys.stderr)
        return 2
    frames = exchange(Path(arguments[0]))
    Path(arguments[1]).write_text(
        "".join(json.dumps(frame, separators=(",", ":")) + "\n" for frame in frames),
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
