"""Source-only guard for real Ly relogin and shared identity assertions."""
from pathlib import Path
import unittest


class ReloginTests(unittest.TestCase):
    def test_ly_fixture_and_post_quit_transition(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        self.assertTrue('services.displayManager.ly.settings.default_input = "password";' in source)
        self.assertTrue('initialPassword = "realmtest";' in source)
        start = source.index('      # Normal Ly logout/relogin, never a display-manager restart.')
        block = source[start:source.index("    '';", start)]
        compile('\n'.join(line[6:] for line in block.splitlines()), '<relogin>', 'exec')
        self.assertLess(block.index('relogin_snapshot({})'), block.index('control("quit")'))
        self.assertLess(block.index('wait_for_text("password"'), block.index('send_chars("realmtest'))
        self.assertIn('relogin_probe.validate_transition(before_login, after, next_generation)', block)
        self.assertIn('consumer_probe.assert_consumer', block)
        self.assertIn('"readlink", "-f", executable', block)
        self.assertIn('consumer_probe.assert_consumer(process, next_root, canonical_executable)', block)
        self.assertIn('retry(transitioned, timeout=STARTUP_TIMEOUT)', block)
        self.assertIn('relogin-roundtrip.json', block)
        self.assertNotIn('systemctl restart', block)
        self.assertTrue('/packaging/nix/test_relogin.py' in source)


if __name__ == '__main__':
    unittest.main()
