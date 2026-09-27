"""Native selected-theme acceptance must reject wrong real consumer evidence."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('consumer', Path(__file__).with_name('consumer_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ConsumerTests(unittest.TestCase):
    def test_yazi_exec_is_barrier_before_reading_sequential_proofs(self):
        started = False
        def process(name):
            nonlocal started
            self.assertEqual(name, 'yazi')
            started = True
            return {'pid': 45}
        def read(command):
            self.assertTrue(started, 'proof read before Yazi exec')
            return '5.9\n' if 'shell-proof' in command else '/usr/lib/realm/bin/yazi\n/usr/lib/realm/bin/starship\n'
        files, version, tools = probe.collect_shell_proofs(process, read)
        self.assertEqual(files, {'pid': 45})
        self.assertEqual(version, '5.9')
        self.assertEqual(tools, ['/usr/lib/realm/bin/yazi', '/usr/lib/realm/bin/starship'])

    def test_foot_selects_modern_then_supported_legacy(self):
        calls = []
        def modern(path):
            calls.append(path)
            return True
        self.assertEqual(probe.select_foot_config('/A', modern), '/A/foot/foot-modern.ini')
        self.assertEqual(calls, ['/A/foot/foot-modern.ini'])
        self.assertEqual(probe.select_foot_config('/A', lambda path: path.endswith('/foot.ini')),
                         '/A/foot/foot.ini')

    def test_foot_rejects_neither_variant_supported(self):
        with self.assertRaises(AssertionError):
            probe.select_foot_config('/A', lambda path: False)

    def evidence(self):
        return {'executable': '/usr/lib/realm/bin/yazi', 'environment': {
            'REALM_GENERATION': '/generation/A', 'ZDOTDIR': '/generation/A/zsh',
            'STARSHIP_CONFIG': '/generation/A/starship.toml',
            'YAZI_CONFIG_HOME': '/generation/A/yazi', 'GTK_THEME': 'realm',
            'QT_QPA_PLATFORMTHEME': 'qt6ct', 'XDG_DATA_DIRS': '/generation/A/share:/usr/share',
            'XDG_CONFIG_DIRS': '/generation/A:/etc/xdg'}}

    def test_real_private_consumer_uses_login_a(self):
        probe.assert_consumer(self.evidence(), '/generation/A', '/usr/lib/realm/bin/yazi')

    def test_new_generation_or_distro_substitute_rejected(self):
        for mutation in ('generation', 'executable', 'config'):
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                evidence = self.evidence()
                if mutation == 'generation':
                    evidence['environment']['REALM_GENERATION'] = '/generation/B'
                elif mutation == 'executable':
                    evidence['executable'] = '/usr/bin/yazi'
                else:
                    evidence['environment']['YAZI_CONFIG_HOME'] = '/generation/B/yazi'
                probe.assert_consumer(evidence, '/generation/A', '/usr/lib/realm/bin/yazi')


if __name__ == '__main__':
    unittest.main()
