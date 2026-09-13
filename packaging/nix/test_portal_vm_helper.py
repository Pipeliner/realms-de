import unittest

from portal_vm_helper import (
    PortalClient,
    frame_evidence,
    request_path,
    single_stream_node,
)


class PortalVmHelperContract(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
