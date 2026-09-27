"""Native browser input waits for real visible UI and rejects missing capture."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('native_browser', Path(__file__).with_name('browser_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class BrowserTests(unittest.TestCase):
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
