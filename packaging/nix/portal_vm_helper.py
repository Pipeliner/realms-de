#!/usr/bin/env python3
"""Exercise the installed desktop portals from one persistent VM client."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import time
from typing import Any


PORTAL_BUS = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"
REQUEST_INTERFACE = "org.freedesktop.portal.Request"
SESSION_INTERFACE = "org.freedesktop.portal.Session"


def request_path(unique_name: str, token: str) -> str:
    """Return the request object path the portal derives for this caller."""
    if not re.fullmatch(r":[0-9]+(?:\.[0-9]+)+", unique_name):
        raise ValueError(f"not a unique D-Bus name: {unique_name!r}")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", token):
        raise ValueError(f"invalid portal handle token: {token!r}")
    sender = unique_name[1:].replace(".", "_")
    return f"{PORTAL_PATH}/request/{sender}/{token}"


def single_stream_node(streams: list[tuple[int, dict[str, Any]]]) -> int:
    """Require the bounded VM request to select exactly one valid output."""
    if len(streams) != 1:
        raise ValueError(f"expected exactly one ScreenCast stream, got {streams!r}")
    node_id = streams[0][0]
    if isinstance(node_id, bool) or not isinstance(node_id, int) or node_id <= 0:
        raise ValueError(f"invalid ScreenCast PipeWire node: {node_id!r}")
    return node_id


def frame_evidence(node_id: int, size: int, width: int, height: int) -> dict[str, int]:
    """Reject metadata-only success: an actual mapped video buffer is required."""
    if node_id <= 0 or size <= 0 or width <= 0 or height <= 0:
        raise ValueError(
            "ScreenCast frame must have a positive node, byte count, width, and height"
        )
    return {
        "node_id": node_id,
        "buffer_bytes": size,
        "width": width,
        "height": height,
    }


def _deep_unpack(value: Any) -> Any:
    if hasattr(value, "unpack"):
        return _deep_unpack(value.unpack())
    if isinstance(value, dict):
        return {key: _deep_unpack(item) for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return type(value)(_deep_unpack(item) for item in value)
    return value


class PortalClient:
    def __init__(self, connection: Any, gio: Any, glib: Any) -> None:
        self.connection = connection
        self.gio = gio
        self.glib = glib
        unique_name = connection.get_unique_name()
        if unique_name is None:
            raise RuntimeError("session bus connection has no unique name")
        self.unique_name = unique_name

    def call(
        self,
        interface: str,
        method: str,
        parameters: Any,
        reply_type: str,
        *,
        path: str = PORTAL_PATH,
        timeout_ms: int = 10_000,
    ) -> Any:
        return self.connection.call_sync(
            PORTAL_BUS,
            path,
            interface,
            method,
            parameters,
            self.glib.VariantType.new(reply_type),
            self.gio.DBusCallFlags.NONE,
            timeout_ms,
            None,
        )

    def begin_request(
        self,
        interface: str,
        method: str,
        parameters: Any,
        token: str,
        *,
        timeout_ms: int = 10_000,
    ) -> tuple[str, dict[str, Any]]:
        expected_path = request_path(self.unique_name, token)
        response: dict[str, Any] = {}
        loop = self.glib.MainLoop()

        def on_response(
            _connection: Any,
            _sender: str,
            object_path: str,
            _interface: str,
            _signal: str,
            parameters_value: Any,
            _user_data: Any,
        ) -> None:
            code = parameters_value.get_child_value(0).get_uint32()
            results = _deep_unpack(parameters_value.get_child_value(1))
            response.update(code=code, results=results, path=object_path)
            loop.quit()

        subscription = self.connection.signal_subscribe(
            PORTAL_BUS,
            REQUEST_INTERFACE,
            "Response",
            expected_path,
            None,
            self.gio.DBusSignalFlags.NONE,
            on_response,
        )
        timed_out = False

        def on_timeout() -> bool:
            nonlocal timed_out
            timed_out = True
            loop.quit()
            return self.glib.SOURCE_REMOVE

        timeout_source = self.glib.timeout_add(timeout_ms, on_timeout)
        try:
            reply = self.call(
                interface,
                method,
                parameters,
                "(o)",
                timeout_ms=timeout_ms,
            )
            returned_path = reply.unpack()[0]
            if returned_path != expected_path:
                raise RuntimeError(
                    f"{method} returned {returned_path!r}, expected {expected_path!r}"
                )
            if timed_out:
                raise TimeoutError(f"{method} portal response exceeded {timeout_ms} ms")
            # call_sync may itself iterate the thread-default context. Do not
            # enter a fresh loop if the response was already dispatched.
            if not response:
                loop.run()
        finally:
            self.connection.signal_unsubscribe(subscription)
            if not timed_out:
                self.glib.source_remove(timeout_source)

        if timed_out:
            raise TimeoutError(f"{method} portal response exceeded {timeout_ms} ms")
        if response.get("code") != 0:
            raise RuntimeError(f"{method} portal response was {response!r}")
        return expected_path, response["results"]

    def close(self, path: str, interface: str) -> None:
        self.call(interface, "Close", None, "()", path=path, timeout_ms=2_000)


def _options(glib: Any, **values: Any) -> dict[str, Any]:
    encoded: dict[str, Any] = {}
    for key, value in values.items():
        if isinstance(value, bool):
            encoded[key] = glib.Variant("b", value)
        elif isinstance(value, int):
            encoded[key] = glib.Variant("u", value)
        elif isinstance(value, str):
            encoded[key] = glib.Variant("s", value)
        else:
            raise TypeError(f"unsupported portal option {key}={value!r}")
    return encoded


def _consume_frame(gst: Any, fd: int, node_id: int) -> dict[str, Any]:
    pipeline = gst.Pipeline.new("realm-portal-frame")
    source = gst.ElementFactory.make("pipewiresrc", "source")
    convert = gst.ElementFactory.make("videoconvert", "convert")
    sink = gst.ElementFactory.make("appsink", "sink")
    if pipeline is None or source is None or convert is None or sink is None:
        raise RuntimeError("required pipewiresrc/videoconvert/appsink element is absent")

    source.set_property("fd", fd)
    source.set_property("path", str(node_id))
    source.set_property("always-copy", True)
    sink.set_property("sync", False)
    sink.set_property("max-buffers", 1)
    sink.set_property("drop", True)

    pipeline.add(source)
    pipeline.add(convert)
    pipeline.add(sink)
    if not source.link(convert) or not convert.link(sink):
        raise RuntimeError("could not link the PipeWire frame pipeline")

    try:
        transition = pipeline.set_state(gst.State.PLAYING)
        if transition == gst.StateChangeReturn.FAILURE:
            raise RuntimeError("PipeWire frame pipeline refused PLAYING state")
        sample = sink.emit("try-pull-sample", 15 * gst.SECOND)
        if sample is None:
            bus = pipeline.get_bus()
            message = bus.pop_filtered(gst.MessageType.ERROR) if bus is not None else None
            if message is not None:
                error, debug = message.parse_error()
                raise RuntimeError(f"PipeWire frame pipeline failed: {error}; {debug}")
            raise TimeoutError("no ScreenCast video buffer arrived within 15 seconds")

        buffer = sample.get_buffer()
        caps = sample.get_caps()
        if buffer is None or caps is None or caps.get_size() != 1:
            raise RuntimeError("ScreenCast sample lacks one buffer/caps structure")
        structure = caps.get_structure(0)
        has_width, width = structure.get_int("width")
        has_height, height = structure.get_int("height")
        if not has_width or not has_height:
            raise RuntimeError(f"ScreenCast caps lack dimensions: {caps.to_string()}")

        mapped, map_info = buffer.map(gst.MapFlags.READ)
        if not mapped:
            raise RuntimeError("ScreenCast buffer is not CPU-readable")
        try:
            payload = bytes(map_info.data)
        finally:
            buffer.unmap(map_info)

        evidence: dict[str, Any] = frame_evidence(
            node_id, len(payload), width, height
        )
        evidence["sha256"] = hashlib.sha256(payload).hexdigest()
        evidence["caps"] = caps.to_string()
        return evidence
    finally:
        pipeline.set_state(gst.State.NULL)


def load_namespaces() -> tuple[Any, Any, Any, Any]:
    """Load the exact GI namespaces required by the packaged VM helper."""
    import gi

    gi.require_version("Gio", "2.0")
    gi.require_version("Gst", "1.0")
    gi.require_version("GstApp", "1.0")
    from gi.repository import (  # pylint: disable=import-outside-toplevel
        Gio,
        GLib,
        Gst,
        GstApp,
    )

    Gst.init(None)
    return Gio, GLib, Gst, GstApp


def run() -> dict[str, Any]:
    Gio, GLib, Gst, _gst_app = load_namespaces()
    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    portal = PortalClient(connection, Gio, GLib)

    file_token = "realm_file"
    file_options = _options(GLib, handle_token=file_token)
    started = time.monotonic()
    file_reply = portal.call(
        "org.freedesktop.portal.FileChooser",
        "OpenFile",
        GLib.Variant("(ssa{sv})", ("", "Realm portal VM", file_options)),
        "(o)",
        timeout_ms=2_000,
    )
    elapsed_ms = round((time.monotonic() - started) * 1000)
    file_handle = file_reply.unpack()[0]
    expected_file_handle = request_path(portal.unique_name, file_token)
    if file_handle != expected_file_handle or elapsed_ms > 2_000:
        raise RuntimeError(
            f"FileChooser returned {file_handle!r} after {elapsed_ms} ms; "
            f"expected {expected_file_handle!r} within 2000 ms"
        )
    portal.close(file_handle, REQUEST_INTERFACE)

    settings_reply = portal.call(
        "org.freedesktop.portal.Settings",
        "ReadAll",
        GLib.Variant("(as)", (["org.freedesktop.appearance"],)),
        "(a{sa{sv}})",
        timeout_ms=2_000,
    )
    settings = _deep_unpack(settings_reply)[0]
    if not isinstance(settings, dict):
        raise RuntimeError(f"Settings.ReadAll returned malformed data: {settings!r}")

    create_options = _options(
        GLib,
        handle_token="realm_create",
        session_handle_token="realm_session",
    )
    _create_path, create_results = portal.begin_request(
        "org.freedesktop.portal.ScreenCast",
        "CreateSession",
        GLib.Variant("(a{sv})", (create_options,)),
        "realm_create",
    )
    session_handle = create_results.get("session_handle")
    if not isinstance(session_handle, str) or not session_handle.startswith(
        f"{PORTAL_PATH}/session/"
    ):
        raise RuntimeError(f"CreateSession returned no valid session: {create_results!r}")

    select_options = _options(
        GLib,
        handle_token="realm_select",
        types=1,
        multiple=False,
    )
    portal.begin_request(
        "org.freedesktop.portal.ScreenCast",
        "SelectSources",
        GLib.Variant("(oa{sv})", (session_handle, select_options)),
        "realm_select",
    )

    start_options = _options(GLib, handle_token="realm_start")
    _start_path, start_results = portal.begin_request(
        "org.freedesktop.portal.ScreenCast",
        "Start",
        GLib.Variant("(osa{sv})", (session_handle, "", start_options)),
        "realm_start",
    )
    streams = start_results.get("streams")
    if not isinstance(streams, list):
        raise RuntimeError(f"Start returned no stream list: {start_results!r}")
    node_id = single_stream_node(streams)

    empty_options: dict[str, Any] = {}
    remote_reply, fd_list = connection.call_with_unix_fd_list_sync(
        PORTAL_BUS,
        PORTAL_PATH,
        "org.freedesktop.portal.ScreenCast",
        "OpenPipeWireRemote",
        GLib.Variant("(oa{sv})", (session_handle, empty_options)),
        GLib.VariantType.new("(h)"),
        Gio.DBusCallFlags.NONE,
        10_000,
        None,
        None,
    )
    fd_index = remote_reply.unpack()[0]
    remote_fd = fd_list.get(fd_index)
    try:
        screen_evidence = _consume_frame(Gst, remote_fd, node_id)
    finally:
        os.close(remote_fd)
        portal.close(session_handle, SESSION_INTERFACE)

    return {
        "filechooser": {
            "handle": file_handle,
            "elapsed_ms": elapsed_ms,
        },
        "settings": {
            "reply_type": settings_reply.get_type_string(),
            "namespaces": sorted(settings),
        },
        "screencast": screen_evidence,
    }


def main() -> int:
    try:
        if sys.argv[1:] == ["--check-imports"]:
            load_namespaces()
            return 0
        if sys.argv[1:]:
            raise ValueError("expected no arguments or exactly --check-imports")
        evidence = run()
    except Exception as error:  # The VM driver needs one concise failure boundary.
        print(f"realm portal VM helper: {error}", file=sys.stderr)
        return 1
    print(json.dumps(evidence, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
