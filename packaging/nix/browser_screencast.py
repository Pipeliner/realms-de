"""Loopback-only evidence collector for the real Firefox VM capture page."""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import math
from pathlib import Path
import struct


def publish_text(target, text):
    pending = target.with_suffix(target.suffix + ".pending")
    pending.write_text(text)
    pending.replace(target)


def validate_result(output, result):
    frames = result.get("frames", [])
    if len(frames) != 2 or result.get("stopped") is not True:
        raise ValueError("two frames and explicit stop required")
    if not result.get("trackStates") or any(state != "ended" for state in result["trackStates"]):
        raise ValueError("all captured tracks must be ended")
    previous = -1
    for index, frame in enumerate(frames):
        timestamp = frame.get("mediaTime", -1)
        if not isinstance(timestamp, (int, float)) or not math.isfinite(timestamp) or timestamp <= previous:
            raise ValueError("video time must advance")
        previous = timestamp
        try:
            data = (output / f"frame-{index}.png").read_bytes()
        except FileNotFoundError as error:
            raise ValueError("missing frame") from error
        if len(data) <= 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR" or b"IDAT" not in data:
            raise ValueError("nonempty PNG frame required")
        width, height = struct.unpack(">II", data[16:24])
        if not width or not height or (width, height) != (frame.get("width"), frame.get("height")):
            raise ValueError("positive matching frame dimensions required")
        frame.update(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())
    return result


def serve(page, output, port):
    output.mkdir(parents=True, exist_ok=True)

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != "/":
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(page.read_bytes())

        def do_POST(self):
            data = self.rfile.read(int(self.headers["Content-Length"]))
            try:
                if self.path in ("/frame-0.png", "/frame-1.png"):
                    (output / self.path[1:]).write_bytes(data)
                elif self.path == "/result":
                    result = validate_result(output, json.loads(data))
                    publish_text(output / "result.json", json.dumps(result, indent=2) + "\n")
                elif self.path == "/error":
                    if not (output / "error.json").exists():
                        publish_text(output / "error.json", data.decode())
                else:
                    self.send_error(404)
                    return
            except (ValueError, KeyError, TypeError) as error:
                diagnostic = {"error": str(error)}
                if self.path == "/result":
                    # Preserve bounded rejected metadata before the page's
                    # generic HTTP error arrives; never publish it as success.
                    rejected = data[:8192].decode(errors="replace")
                    try:
                        diagnostic["rejected"] = json.loads(rejected)
                    except ValueError:
                        diagnostic["rejected_text"] = rejected
                if not (output / "error.json").exists():
                    publish_text(output / "error.json", json.dumps(diagnostic))
                self.send_error(400, str(error))
                return
            self.send_response(204)
            self.end_headers()

    with HTTPServer(("127.0.0.1", port), Handler) as server:
        publish_text(output / "ready", str(server.server_port))
        server.serve_forever()


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--page", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--port", type=int, default=8765)
    args = parser.parse_args()
    serve(args.page, args.output, args.port)
