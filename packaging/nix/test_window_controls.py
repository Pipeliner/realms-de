"""Keep the Nix VM wired to the shared real-keyboard acceptance contract."""
from pathlib import Path
import unittest


class WindowControlsTests(unittest.TestCase):
    def test_browser_evidence_uses_current_driver_api(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        self.assertFalse('machine.copy_from_vm(' in source, 'deprecated copy API is rejected by pinned driver typecheck')
        self.assertIn('machine.copy_from_machine(browser_evidence, "browser-screencast")', source)

    def test_shared_keyboard_probe_runs_before_existing_demo(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        start = source.index('      # Shared installed-window keyboard acceptance.')
        end = source.index('      for number in range(1, 4):', start)
        block = source[start:end]
        compile('\n'.join(line[6:] for line in block.splitlines()), '<window-probe>', 'exec')
        self.assertIn('exercise_controls(window_wait, machine.send_key, window_screenshot)', block)
        self.assertIn('WINDOW_SNAPSHOT', block)
        self.assertIn('machine.send_key("meta_l-ret")', block)
        self.assertIn('machine.send_chars', block)
        self.assertIn('machine.send_key("meta_l-q")', block)
        self.assertIn('retry(matches, timeout=STATE_TIMEOUT)', block)
        self.assertIn('finally:', block)
        self.assertIn('window-roundtrip.json', block)
        self.assertNotIn('control("spawn"', block)
        self.assertIn('window_probe = importlib.import_module("window_roundtrip")', source)
        self.assertIn('exercise_controls = window_probe.exercise_controls', source)
        self.assertIn('/packaging/nix/test_window_controls.py', source)


if __name__ == '__main__':
    unittest.main()
