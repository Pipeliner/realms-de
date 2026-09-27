"""Source-only acceptance decisions; no VM or package production."""
import copy
import importlib.util
from pathlib import Path
import unittest


class WindowTests(unittest.TestCase):
    def test_harness_runs_controls_after_consumers_before_portals(self):
        script = Path(__file__).with_name('run-native-session-vm.sh').read_text()
        consumers = script.index('timeout 240 python3 "$script_dir/consumer_roundtrip.py"')
        controls = script.index('timeout 300 python3 "$script_dir/window_roundtrip.py"')
        portals = script.index('timeout 240 python3 "$script_dir/portal_roundtrip.py"')
        self.assertLess(consumers, controls)
        self.assertLess(controls, portals)

    def setUp(self):
        path = Path(__file__).with_name('window_roundtrip.py')
        self.assertTrue(path.exists(), 'native window acceptance helper is missing')
        spec = importlib.util.spec_from_file_location('window_roundtrip', path)
        self.probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.probe)

    def exercise(self, broken=None):
        windows = [dict(id=n, title='Realm window ' + letter, focused=n == 3)
                   for n, letter in enumerate('ABC', 1)]
        value = {'state': {'focused_title': 'Realm window C', 'layout': 'triptych',
                           'whichkey': True, 'grimoire': False},
                 'ledger': [{'orbit': 1, 'active': True, 'layout': 'triptych', 'windows': windows},
                            {'orbit': 2, 'active': False, 'layout': 'triptych', 'windows': []}]}
        keys, pictures = [], []

        def key(chord):
            keys.append(chord)
            orbit = next(o for o in value['ledger'] if o['active'])
            wins = orbit['windows']
            if chord in ('meta_l-j', 'meta_l-k', 'meta_l-l', 'meta_l-h'):
                index = next(i for i, w in enumerate(wins) if w['focused'])
                direction = 1 if chord in ('meta_l-j', 'meta_l-l') else -1
                target = (index + direction) % len(wins)
                if chord in ('meta_l-j', 'meta_l-k') and broken != 'focus':
                    wins[index]['focused'] = False
                    wins[target]['focused'] = True
                elif chord in ('meta_l-l', 'meta_l-h') and broken != 'swap':
                    wins[index], wins[target] = wins[target], wins[index]
            elif chord in ('meta_l-1', 'meta_l-2') and broken != 'orbit':
                for o in value['ledger']:
                    o['active'] = o['orbit'] == int(chord[-1])
            elif chord in ('meta_l-m', 'meta_l-t') and broken != 'layout':
                value['state']['layout'] = orbit['layout'] = 'mono' if chord.endswith('m') else 'triptych'
            elif chord == 'meta_l-w' and broken != 'whichkey':
                value['state']['whichkey'] = not value['state']['whichkey']
            elif chord == 'meta_l-shift-0x35' and broken != 'help':
                value['state']['grimoire'] = True
            elif chord == 'meta_l-esc' and broken != 'dismiss':
                value['state']['grimoire'] = False
            active = next(o for o in value['ledger'] if o['active'])
            value['state']['focused_title'] = next((w['title'] for w in active['windows'] if w['focused']), '')

        def wait(predicate, label):
            self.assertTrue(predicate(value), label)
            return copy.deepcopy(value)

        self.probe.exercise_controls(wait, key, pictures.append)
        return value, keys, pictures

    def test_all_controls_restore_windows_and_dismiss_help(self):
        value, keys, pictures = self.exercise()
        self.assertEqual([w['id'] for w in value['ledger'][0]['windows']], [1, 2, 3])
        self.assertEqual(value['state']['focused_title'], 'Realm window C')
        self.assertFalse(value['state']['grimoire'])
        self.assertTrue(value['state']['whichkey'])
        self.assertIn('window-mono', pictures)
        self.assertIn('window-help', pictures)
        self.assertIn('meta_l-2', keys)

    def test_ignored_or_wrong_actions_never_count_as_success(self):
        for broken in ('focus', 'swap', 'orbit', 'layout', 'whichkey', 'help', 'dismiss'):
            with self.subTest(broken=broken), self.assertRaises(AssertionError):
                self.exercise(broken)


if __name__ == '__main__':
    unittest.main()
