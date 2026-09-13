import unittest
from unittest import mock

from portal_vm_helper import (
    PORTAL_BUS,
    REQUEST_INTERFACE,
    PortalClient,
    frame_evidence,
    main,
    request_path,
    single_stream_node,
)


class PortalVmHelperContract(unittest.TestCase):
    @mock.patch("portal_vm_helper.run")
    @mock.patch("portal_vm_helper.load_namespaces")
    def test_import_only_mode_loads_namespaces_without_opening_portals(
        self, load_namespaces, run
    ):
        with mock.patch("sys.argv", ["realm-portal-vm", "--check-imports"]):
            self.assertEqual(main(), 0)
        load_namespaces.assert_called_once_with()
        run.assert_not_called()

    def test_request_path_matches_portal_sender_convention(self):
        self.assertEqual(
            request_path(":1.42", "realm_file"),
            "/org/freedesktop/portal/desktop/request/1_42/realm_file",
        )

    def test_request_path_rejects_non_unique_bus_name(self):
        with self.assertRaises(ValueError):
            request_path("org.example.Client", "realm_file")

    def test_single_stream_requires_exactly_one_positive_node(self):
        self.assertEqual(single_stream_node([(41, {})]), 41)
        for streams in [[], [(0, {})], [(4, {}), (5, {})]]:
            with self.subTest(streams=streams), self.assertRaises(ValueError):
                single_stream_node(streams)

    def test_frame_evidence_refuses_metadata_only_success(self):
        self.assertEqual(
            frame_evidence(41, 4096, 1280, 800),
            {
                "node_id": 41,
                "buffer_bytes": 4096,
                "width": 1280,
                "height": 800,
            },
        )
        for size, width, height in [(0, 1280, 800), (1, 0, 800), (1, 1280, 0)]:
            with self.subTest(size=size, width=width, height=height), self.assertRaises(
                ValueError
            ):
                frame_evidence(41, size, width, height)

    def test_request_timeout_during_sync_call_never_enters_dead_loop(self):
        expected = request_path(":1.42", "realm_create")

        class Reply:
            def unpack(self):
                return (expected,)

        class Connection:
            def get_unique_name(self):
                return ":1.42"

            def signal_subscribe(self, *_args):
                return 7

            def signal_unsubscribe(self, subscription):
                self.unsubscribed = subscription

            def call_sync(self, *_args):
                return Reply()

        class Loop:
            def quit(self):
                pass

            def run(self):
                raise AssertionError("expired request entered a main loop")

        class VariantType:
            @staticmethod
            def new(value):
                return value

        class GLib:
            SOURCE_REMOVE = False

            @staticmethod
            def MainLoop():
                return Loop()

            @staticmethod
            def timeout_add(_timeout_ms, callback):
                callback()
                return 9

            @staticmethod
            def source_remove(_source):
                raise AssertionError("expired timeout source was removed twice")

        GLib.VariantType = VariantType

        class DBusCallFlags:
            NONE = 0

        class DBusSignalFlags:
            NONE = 0

        class Gio:
            pass

        Gio.DBusCallFlags = DBusCallFlags
        Gio.DBusSignalFlags = DBusSignalFlags

        portal = PortalClient(Connection(), Gio, GLib)
        with self.assertRaisesRegex(TimeoutError, "CreateSession"):
            portal.begin_request(
                "org.freedesktop.portal.ScreenCast",
                "CreateSession",
                None,
                "realm_create",
            )

    def test_filechooser_response_close_race_needs_exact_completed_response(self):
        expected = request_path(":1.42", "realm_file")

        class Reply:
            def unpack(self):
                return (expected,)

        class Child:
            def __init__(self, value):
                self.value = value

            def get_uint32(self):
                return self.value

            def unpack(self):
                return self.value

        class Response:
            def __init__(self, code):
                self.code = code

            def get_child_value(self, index):
                return Child(self.code if index == 0 else {})

        class RemoteError(Exception):
            remote_name = "org.freedesktop.DBus.Error.UnknownMethod"

        class Connection:
            def __init__(self, response_code):
                self.response_code = response_code
                self.callback = None
                self.unsubscribed = None

            def get_unique_name(self):
                return ":1.42"

            def signal_subscribe(self, *_args):
                self.callback = _args[-1]
                return 7

            def signal_unsubscribe(self, subscription):
                self.unsubscribed = subscription

            def call_sync(self, _bus, path, _interface, method, *_args):
                if method == "OpenFile":
                    return Reply()
                self.assert_close_path = path
                raise RemoteError("request already completed")

        class Loop:
            def __init__(self, connection):
                self.connection = connection
                self.quit_called = False

            def quit(self):
                self.quit_called = True

            def run(self):
                if self.connection.response_code is None:
                    GLib.timeout_callback()
                else:
                    self.connection.callback(
                        None,
                        PORTAL_BUS,
                        expected,
                        REQUEST_INTERFACE,
                        "Response",
                        Response(self.connection.response_code),
                        None,
                    )

        class VariantType:
            @staticmethod
            def new(value):
                return value

        class GLib:
            SOURCE_REMOVE = False
            connection = None
            timeout_callback = None

            @staticmethod
            def MainLoop():
                return Loop(GLib.connection)

            @staticmethod
            def timeout_add(_timeout_ms, callback):
                GLib.timeout_callback = callback
                return 9

            @staticmethod
            def source_remove(_source):
                pass

        GLib.VariantType = VariantType

        class DBusCallFlags:
            NONE = 0

        class DBusSignalFlags:
            NONE = 0

        class DBusError:
            @staticmethod
            def get_remote_error(error):
                return getattr(error, "remote_name", None)

        class Gio:
            pass

        Gio.DBusCallFlags = DBusCallFlags
        Gio.DBusSignalFlags = DBusSignalFlags
        Gio.DBusError = DBusError

        connection = Connection(1)
        GLib.connection = connection
        portal = PortalClient(connection, Gio, GLib)
        outcome = portal.filechooser_roundtrip(None, "realm_file")
        self.assertEqual(outcome["completion"], "response")
        self.assertEqual(outcome["response_code"], 1)
        self.assertEqual(connection.assert_close_path, expected)
        self.assertEqual(connection.unsubscribed, 7)

        for response_code, message in [
            (None, "without an exact Response"),
            (2, "failure response 2"),
        ]:
            with self.subTest(response_code=response_code):
                connection = Connection(response_code)
                GLib.connection = connection
                portal = PortalClient(connection, Gio, GLib)
                with self.assertRaisesRegex(RuntimeError, message):
                    portal.filechooser_roundtrip(None, "realm_file")


if __name__ == "__main__":
    unittest.main()
