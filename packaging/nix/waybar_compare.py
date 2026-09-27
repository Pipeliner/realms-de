#!/usr/bin/env python3
"""CI-only continuous Waybar module for Realm's existing control stream."""

import argparse
import json
import os
import socket
import sys
from pathlib import Path
try:
    import tomllib
except ModuleNotFoundError:  # Local source tests may use Python 3.10.
    import tomli as tomllib


PROTOCOL_VERSION = 2
MAX_FRAME_BYTES = 65_536  # Includes the trailing newline, as in realm-core.


class ProtocolError(Exception):
    """The Realm stream cannot be interpreted as protocol v2."""


def render_state(state: dict) -> dict:
    """Return one Waybar custom-module value from a Realm State event."""
    orbits = state["orbits"]
    if len(orbits) != 6 or [cell["number"] for cell in orbits] != list(range(1, 7)):
        raise ProtocolError("State must contain the six numbered Realm orbits")
    markers = {"active": ("[", "]"), "occupied": ("●", ""), "empty": ("·", "")}
    cells = []
    tooltip = []
    for cell in orbits:
        display = cell["display"]
        if display not in markers:
            raise ProtocolError(f"unknown orbit display: {display!r}")
        prefix, suffix = markers[display]
        cells.append(f"{prefix}{cell['number']}{cell['rune']}{suffix}")
        tooltip.append(
            f"Orbit {cell['number']}: {display} ({cell['windows']} "
            f"{'window' if cell['windows'] == 1 else 'windows'})"
        )
    title = state["focused_title"].replace("\n", " ").replace("\r", " ")
    if len(title) > 32:
        title = title[:31] + "…"
    parts = [
        " ".join(cells), state["layout"].upper(), state["mode"].upper(),
        title or "untitled",
    ]
    if state["chord_echo"]:
        parts.append(state["chord_echo"])
    text = " | ".join(parts)
    return {"text": text, "tooltip": "\n".join(tooltip), "class": "realm"}


def render_css(palette: dict) -> str:
    """Style the experiment from the selected, immutable login palette."""
    background = palette["background"]["bar_top"]
    normal = palette["text"]["normal"]
    bright = palette["text"]["bright"]
    violet = palette["accent"]["violet"]
    starlight = palette["accent"]["starlight"]
    families = palette["typography"]["fallback"]
    family = ", ".join(f'"{name}"' for name in families)
    size = palette["typography"]["size_meta"]
    height = palette["metrics"]["bar_height"]
    return (
        f'* {{ font-family: {family}; font-size: {size}px; }}\n'
        f'window#waybar {{ background: {background}; color: {normal}; min-height: {height}px; }}\n'
        f'#custom-realm {{ color: {bright}; border-top: 1px solid {violet}; padding: 0 8px; }}\n'
        f'#clock, #cpu, #memory, #network, #battery {{ color: {starlight}; padding: 0 6px; }}\n'
    )


def read_frame(reader):
    frame = reader.readline(MAX_FRAME_BYTES + 1)
    if not frame:
        raise ProtocolError("Realm stream closed without Shutdown")
    if len(frame) > MAX_FRAME_BYTES:
        raise ProtocolError("Realm frame exceeds 65536 bytes")
    if not frame.endswith(b"\n"):
        raise ProtocolError("incomplete Realm frame")
    try:
        value = json.loads(frame.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ProtocolError(f"malformed Realm frame: {error}") from error
    if not isinstance(value, dict):
        raise ProtocolError("Realm frame is not an object")
    return value


def run(connection, output):
    """Subscribe once, then flush only changed display values until Shutdown."""
    with connection.makefile("rb") as reader:
        connection.sendall(b'{"cmd":"hello","arg":{"version":2,"client":"realm-waybar-compare"}}\n')
        hello = read_frame(reader)
        if hello.get("reply") != "hello" or not isinstance(hello.get("data"), dict):
            raise ProtocolError("expected Realm Hello response")
        if hello["data"].get("version") != PROTOCOL_VERSION:
            raise ProtocolError(f"Realm protocol version mismatch: {hello['data'].get('version')!r}")
        connection.sendall(b'{"cmd":"subscribe"}\n')
        previous = None
        initial = True
        while True:
            event = read_frame(reader)
            if event.get("event") == "shutdown" and not initial:
                return
            if event.get("event") != "state" or not isinstance(event.get("data"), dict):
                raise ProtocolError("expected Realm State event")
            initial = False
            try:
                value = render_state(event["data"])
            except (KeyError, TypeError, AttributeError, ValueError) as error:
                raise ProtocolError(f"malformed Realm State: {error}") from error
            if value != previous:
                output.write(json.dumps(value, ensure_ascii=False) + "\n")
                output.flush()
                previous = value


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", type=Path, default=None)
    parser.add_argument("--css", type=Path, default=None, metavar="SELECTED_PALETTE_TOML")
    args = parser.parse_args(argv)
    if args.css is not None:
        try:
            sys.stdout.write(render_css(tomllib.loads(args.css.read_text(encoding="utf-8"))))
            sys.stdout.flush()
        except (OSError, KeyError, ValueError) as error:
            print(f"waybar compare: cannot render selected palette: {error}", file=sys.stderr)
            return 1
        return 0
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    path = args.socket or (Path(runtime) / "realm" / "ctl.sock" if runtime else None)
    if path is None:
        print("waybar compare: XDG_RUNTIME_DIR is unset", file=sys.stderr)
        return 1
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.connect(path)
            run(connection, sys.stdout)
    except (OSError, ProtocolError) as error:
        print(f"waybar compare: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
