"""Native browser input waits for real visible UI and rejects missing capture."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('native_browser', Path(__file__).with_name('browser_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class BrowserTests(unittest.TestCase):
    def test_source_list_requires_visible_monitor_before_real_input(self):
        order = []
        self.assertTrue(hasattr(probe, 'select_source'), 'browser assumes Slurp and cannot drive actual source list')
        def visible(text, name, absent=None):
            order.append(('visible', text))
            return 'Monitor: Virtual-1 Red Hat'
        probe.select_source('fuzzel', visible,
                            lambda key: order.append(('key', key)),
                            lambda: self.fail('source list must not use Slurp coordinates'),
                            lambda: order.append(('closed', None)))
        self.assertEqual(order[:2], [('visible', 'Select a source to share'), ('visible', 'Monitor:')])
        keys = [value for kind, value in order if kind == 'key']
        self.assertEqual(keys, ['m', 'o', 'n', 'i', 't', 'o', 'r', 'ret'])
        self.assertEqual(order[-3:], [('visible', 'Monitor:'), ('key', 'ret'), ('closed', None)])

    def test_absent_source_list_never_sends_selection(self):
        self.assertTrue(hasattr(probe, 'select_source'))
        keys = []
        def missing(text, name):
            raise TimeoutError('source prompt absent')
        with self.assertRaises(TimeoutError):
            probe.select_source('fuzzel', missing, keys.append, lambda: None, lambda: None)
        self.assertEqual(keys, [])

    def test_window_remaining_after_filter_never_accepted(self):
        keys = []
        with self.assertRaisesRegex(AssertionError, 'ambiguous'):
            probe.select_source('fuzzel', lambda *args, **kwargs: 'Monitor: Virtual-1 Window: browser',
                                keys.append, lambda: None, lambda: None)
        self.assertNotIn('ret', keys)

    def test_filter_waits_through_old_menu_and_times_out_if_window_remains(self):
        self.assertTrue(hasattr(probe, 'wait_visible'), 'filter readiness must wait for Window disappearance')
        frames = iter(['Monitor: Virtual-1 Window: browser', 'Monitor: Virtual-1'])
        sleeps = []
        self.assertEqual(probe.wait_visible(lambda: next(frames), 'Monitor:', 'Window:',
                         sleep=sleeps.append), 'Monitor: Virtual-1')
        self.assertEqual(sleeps, [0.5])
        times = iter([0, 46])
        with self.assertRaises(AssertionError):
            probe.wait_visible(lambda: 'Monitor: Virtual-1 Window: browser', 'Monitor:',
                               'Window:', clock=lambda: next(times), sleep=lambda seconds: None)

    def test_slurp_keeps_pointer_path_and_unknown_menu_rejected(self):
        self.assertTrue(hasattr(probe, 'select_source'))
        actions = []
        probe.select_source('slurp', lambda *args: self.fail('not a text menu'),
            lambda key: self.fail('not a keyboard chooser'), lambda: actions.append('pointer'), lambda: None)
        self.assertEqual(actions, ['pointer'])
        with self.assertRaises(AssertionError):
            probe.select_source('unknown', lambda *args: None, lambda key: None, lambda: None, lambda: None)

    def test_navigation_replaces_first_run_page_with_real_keyboard_input(self):
        keys = []
        probe.navigate_capture(keys.append)
        self.assertEqual(keys[0], 'ctrl-l')
        self.assertEqual(keys[-1], 'ret')
        self.assertIn('shift-0x27', keys)
        self.assertEqual(keys.count('0x35'), 3)

    def test_result_wait_allows_page_frame_deadline(self):
        def wait(command, seconds=15):
            self.assertIn('/result.json', command)
            self.assertIn('/error.json', command)
            self.assertGreaterEqual(seconds, 35, 'host rejected a valid frame before page deadline')
        probe.wait_for_capture(wait, '/fixture')

    def test_permission_key_follows_visible_prompt(self):
        order = []
        def wait(text, name):
            order.append(('visible', text))
        probe.activate_share(wait, lambda key: order.append(('key', key)))
        self.assertEqual(order, [('visible', 'Realm browser capture ready'), ('key', 'ret'),
                                 ('visible', 'Use operating system settings'), ('key', 'alt-a')])

    def test_missing_permission_never_sends_allow(self):
        keys = []
        def wait(text, name):
            if 'operating system' in text:
                raise TimeoutError('prompt absent')
        with self.assertRaises(TimeoutError):
            probe.activate_share(wait, keys.append)
        self.assertEqual(keys, ['ret'])


if __name__ == '__main__':
    unittest.main()
