import json
import os
import tempfile
import unittest
import re
import textwrap
import time
from datetime import timedelta
from pathlib import Path
from unittest import mock

from portal_vm_helper import (
    PORTAL_BUS,
    REQUEST_INTERFACE,
    PortalClient,
    frame_evidence,
    main,
    run,
    request_path,
    single_stream_node,
)


class PortalVmHelperContract(unittest.TestCase):
    def picker(self, mice, results=None):
        source = Path(__file__).with_name('checks.nix').read_text()
        body = source.split('      def select_portal_output(screenshot):', 1)[1].split('      def log_portal_diagnostics():', 1)[0]
        function = textwrap.dedent('      def select_portal_output(screenshot):' + body)
        machine = mock.Mock()
        machine.send_monitor_command.return_value = (
            'Mouse #2: QEMU PS/2 Mouse\n'
            'Mouse #4: QEMU HID Tablet (absolute)\n'
            '* Mouse #6: vmmouse (absolute)\n'
        )
        results = results or {}
        def qmp_send(command, arguments=None):
            if command == 'query-mice':
                return {'return': mice}
            return results.get(command, {'return': {}})
        machine.qmp_client.send.side_effect = qmp_send
        machine.execute.side_effect = [(0, ''), (1, '')]
        namespace = {'machine': machine, 're': re, 'time': time,
                     'STATE_TIMEOUT': timedelta(seconds=1), 'DIAGNOSTIC_TIMEOUT': 1}
        exec(compile(function, 'checks.nix:select_portal_output', 'exec'), namespace)
        return namespace['select_portal_output'], machine

    def test_actual_picker_rejects_active_relative_mouse_before_input(self):
        mice = [{'index': 2, 'name': 'QEMU PS/2 Mouse', 'current': True, 'absolute': False}]
        picker, machine = self.picker(mice)
        with self.assertRaisesRegex(AssertionError, 'active absolute'):
            picker('fixture')
        self.assertEqual(machine.qmp_client.send.call_args_list, [mock.call('query-mice')])

    def test_actual_picker_sends_absolute_midpoint_then_real_click(self):
        mice = [
            {'index': 2, 'name': 'QEMU PS/2 Mouse', 'current': False, 'absolute': False},
            {'index': 4, 'name': 'QEMU HID Tablet', 'current': False, 'absolute': True},
            {'index': 6, 'name': 'vmmouse', 'current': True, 'absolute': True},
        ]
        picker, machine = self.picker(mice)
        picker('fixture')
        self.assertEqual(machine.qmp_client.send.call_args_list, [
            mock.call('query-mice'),
            mock.call('input-send-event', {'events': [
                {'type': 'abs', 'data': {'axis': 'x', 'value': 16384}},
                {'type': 'abs', 'data': {'axis': 'y', 'value': 16384}},
            ]}),
            mock.call('input-send-event', {'events': [
                {'type': 'btn', 'data': {'button': 'left', 'down': True}},
            ]}),
            mock.call('input-send-event', {'events': [
                {'type': 'btn', 'data': {'button': 'left', 'down': False}},
            ]}),
        ])
        self.assertTrue(any('vmmouse' in str(call) for call in machine.log.call_args_list))
        machine.screenshot.assert_any_call('fixture-pointer')

    def test_actual_picker_rejects_missing_active_mouse(self):
        picker, machine = self.picker([
            {'index': 4, 'name': 'QEMU HID Tablet', 'current': False, 'absolute': True},
        ])
        with self.assertRaisesRegex(AssertionError, 'active absolute'):
            picker('fixture')
        self.assertEqual(machine.qmp_client.send.call_args_list, [mock.call('query-mice')])

    def test_actual_picker_rejects_qmp_input_error(self):
        picker, machine = self.picker(
            [{'index': 6, 'name': 'vmmouse', 'current': True, 'absolute': True}],
            {'input-send-event': {'error': {'class': 'GenericError', 'desc': 'input denied'}}},
        )
        with self.assertRaisesRegex(AssertionError, 'input denied'):
            picker('fixture')
        self.assertEqual(machine.screenshot.call_args_list, [mock.call('fixture')])

    def test_interactive_selection_and_start_allow_chooser_longer_than_ten_seconds(self):
        class Selected(Exception):
            pass

        def request(_interface, method, *_args, timeout_ms=10_000):
            if method == "CreateSession":
                return "/request", {"session_handle": "/org/freedesktop/portal/desktop/session/test"}
            if method == interactive_method:
                if timeout_ms < 30_000:
                    raise TimeoutError("healthy interactive selection took 30 seconds")
                self.assertLessEqual(timeout_ms, 120_000)
                raise Selected()
            return "/request", {}

        portal = mock.Mock()
        portal.begin_request.side_effect = request
        portal.call.return_value = ({},)
        for interactive_method in ("SelectSources", "Start"):
            with self.subTest(method=interactive_method), mock.patch("portal_vm_helper.load_namespaces", return_value=(mock.Mock(), mock.Mock(), mock.Mock(), mock.Mock())), mock.patch("portal_vm_helper.PortalClient", return_value=portal), mock.patch.dict(os.environ, REALM_PORTAL_FILECHOOSER_READY="/tmp/test-ready"):
                with self.assertRaises(Selected):
                    run()

    def test_vm_background_launcher_detaches_driver_fds_and_retains_evidence(self):
        source = Path(__file__).with_name("checks.nix").read_text(encoding="utf-8")
        launcher = source.split("portal_command = shlex.join", 1)[1].split(
            "machine.wait_until_succeeds(", 1
        )[0]
        for fragment in [
            "> {portal_output_path}",
            "2> {portal_error_path}",
            "> {portal_status_path}",
            '"< /dev/null > /dev/null 2>&1 &"',
        ]:
            with self.subTest(fragment=fragment):
                self.assertIn(fragment, launcher)

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

    def test_filechooser_marks_valid_handle_then_requires_user_cancel_response(self):
        expected = request_path(":1.42", "realm_file")

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

        class Connection:
            def __init__(self, response_code, returned_path=expected):
                self.response_code = response_code
                self.returned_path = returned_path
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
                    return mock.Mock(unpack=lambda: (self.returned_path,))
                if method == "Close":
                    raise AssertionError("the VM chooser must be cancelled through River")
                raise AssertionError(f"unexpected method {method}")

        class Loop:
            def __init__(self, connection, ready_path):
                self.connection = connection
                self.ready_path = ready_path

            def quit(self):
                pass

            def run(self):
                if not self.ready_path.exists():
                    raise AssertionError("response wait began before the readiness marker")
                if self.connection.response_code == "raise":
                    raise RuntimeError("response loop failed")
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
                    )

        class VariantType:
            @staticmethod
            def new(value):
                return value

        class GLib:
            SOURCE_REMOVE = False
            connection = None
            ready_path = None
            timeout_callback = None
            expire_immediately = False
            removed_sources = []

            @staticmethod
            def MainLoop():
                return Loop(GLib.connection, GLib.ready_path)

            @staticmethod
            def timeout_add(_timeout_ms, callback):
                GLib.timeout_callback = callback
                if GLib.expire_immediately:
                    callback()
                return 9

            @staticmethod
            def source_remove(_source):
                GLib.removed_sources.append(_source)

        GLib.VariantType = VariantType

        class DBusCallFlags:
            NONE = 0

        class DBusSignalFlags:
            NONE = 0

        class Gio:
            pass

        Gio.DBusCallFlags = DBusCallFlags
        Gio.DBusSignalFlags = DBusSignalFlags
        with tempfile.TemporaryDirectory() as directory:
            ready_path = Path(directory) / "filechooser-ready.json"
            GLib.ready_path = ready_path

            connection = Connection(1)
            GLib.connection = connection
            portal = PortalClient(
                connection,
                Gio,
                GLib,
                monotonic=mock.Mock(side_effect=[100.0, 100.004]),
            )
            outcome = portal.filechooser_roundtrip(
                None, "realm_file", ready_path=ready_path
            )
            self.assertEqual(outcome["completion"], "response")
            self.assertEqual(outcome["response_code"], 1)
            self.assertEqual(connection.unsubscribed, 7)
            self.assertEqual(
                json.loads(ready_path.read_text(encoding="utf-8")),
                {"elapsed_ms": 4, "handle": expected},
            )

            for response_code, message in [
                (None, "response exceeded 120000 ms"),
                (0, "expected user-cancel response 1, got 0"),
                (2, "expected user-cancel response 1, got 2"),
            ]:
                with self.subTest(response_code=response_code):
                    ready_path.unlink(missing_ok=True)
                    connection = Connection(response_code)
                    GLib.connection = connection
                    portal = PortalClient(connection, Gio, GLib)
                    with self.assertRaisesRegex((RuntimeError, TimeoutError), message):
                        portal.filechooser_roundtrip(
                            None, "realm_file", ready_path=ready_path
                        )

            ready_path.unlink(missing_ok=True)
            GLib.expire_immediately = True
            connection = Connection(None)
            GLib.connection = connection
            portal = PortalClient(connection, Gio, GLib)
            with self.assertRaisesRegex(TimeoutError, "response exceeded 120000 ms"):
                portal.filechooser_roundtrip(None, "realm_file", ready_path=ready_path)
            GLib.expire_immediately = False

            ready_path.unlink(missing_ok=True)
            connection = Connection("raise")
            GLib.connection = connection
            portal = PortalClient(connection, Gio, GLib)
            with self.assertRaisesRegex(RuntimeError, "response loop failed"):
                portal.filechooser_roundtrip(None, "realm_file", ready_path=ready_path)
            self.assertEqual(GLib.removed_sources[-1], 9)

            ready_path.unlink(missing_ok=True)
            connection = Connection(1, returned_path=expected + "_wrong")
            GLib.connection = connection
            portal = PortalClient(connection, Gio, GLib)
            with self.assertRaisesRegex(RuntimeError, "expected"):
                portal.filechooser_roundtrip(None, "realm_file", ready_path=ready_path)
            self.assertFalse(ready_path.exists())


if __name__ == "__main__":
    unittest.main()
