"""Source-only decisions for installed WM crash recovery."""
import copy
import importlib
from pathlib import Path
import unittest

from consumer_process import recovery_app_pids


class RecoveryTests(unittest.TestCase):
    def test_recovery_requires_three_selected_terminals_but_ignores_distro_server(self):
        selected = b'foot\0--config=/login/A/foot/foot.ini\0zsh\0'
        commands = {'10': b'foot\0--server\0', '11': selected, '12': selected,
                    '13': selected, '14': b'foot\0--config=/login/B/foot/foot.ini\0zsh\0'}
        self.assertEqual(recovery_app_pids(list(commands), '/login/A/foot/foot.ini',
                                           lambda pid: commands[pid]), ['11', '12', '13'])
        for removed in ('11', '12', '13'):
            with self.subTest(removed=removed), self.assertRaises(AssertionError):
                recovery_app_pids([pid for pid in commands if pid != removed],
                                  '/login/A/foot/foot.ini', lambda pid: commands[pid])
        commands['15'] = selected
        with self.assertRaises(AssertionError):
            recovery_app_pids(list(commands), '/login/A/foot/foot.ini', lambda pid: commands[pid])

    def test_apps_must_be_in_distinct_session_owned_scopes(self):
        apps = {str(pid): {'scope': 'run-' + str(pid) + '.scope', 'part_of': ['realm-session.target'],
                          'binds_to': ['realm-session.target'], 'start_time': pid * 10}
                for pid in (20, 21, 22)}
        self.probe.validate_app_scopes(apps)
        for field, value in (('scope', 'realm-wm.service'), ('part_of', []), ('binds_to', [])):
            broken = copy.deepcopy(apps)
            broken['20'][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.probe.validate_app_scopes(broken)

    def test_harness_runs_bounded_recovery_between_windows_and_portals(self):
        source = Path(__file__).with_name('run-native-session-vm.sh').read_text()
        windows = source.index('timeout 300 python3 "$script_dir/window_roundtrip.py"')
        recovery = source.index('timeout 300 python3 "$script_dir/recovery_roundtrip.py"')
        portals = source.index('timeout 240 python3 "$script_dir/portal_roundtrip.py"')
        self.assertLess(windows, recovery)
        self.assertLess(recovery, portals)

    def setUp(self):
        self.probe = importlib.import_module('recovery_roundtrip')
        self.before = {'session': '3', 'login': {'owner_pid': 8, 'start_time': 80},
                       'processes': {'river': [10, 100], 'realm-wm': [11, 110], 'realm-bar': [12, 120]},
                       'bar': {'ActiveState': 'active', 'ActiveEnterTimestampMonotonic': '42', 'NRestarts': '0'},
                       'apps': {'20': {'start_time': 200}},
                       'durable': {'bindings': [{'win_id': 1, 'backend_id': 'a'}]}}
        self.after = copy.deepcopy(self.before)
        self.after['processes']['realm-wm'] = [13, 130]

    def test_only_wm_identity_changes(self):
        self.probe.validate_recovery(self.before, self.after)
        for field in ('session', 'login', 'river', 'realm-bar', 'realm-wm', 'bar', 'durable', 'apps'):
            changed = copy.deepcopy(self.after)
            if field in ('river', 'realm-bar'): changed['processes'][field] = [90, 900]
            elif field == 'realm-wm': changed['processes'][field] = [11, 110]
            elif field == 'bar': changed['bar']['NRestarts'] = '1'
            elif field == 'durable': changed['durable']['bindings'][0]['backend_id'] = 'replacement'
            else: changed[field] = None
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.probe.validate_recovery(self.before, changed)

    def test_durable_barrier_rejects_old_order_or_focus(self):
        live = {'ledger': [{'orbit': 1, 'active': True, 'layout': 'triptych',
                           'windows': [{'id': 2, 'focused': True, 'stowed': False},
                                       {'id': 1, 'focused': False, 'stowed': False}]}]}
        durable = {'active_orbit': 0, 'ledger': {'orbits': [{'id': 0, 'windows': [2, 1],
                   'focus': 0, 'stowed': [], 'layout': 'triptych'}]},
                   'bindings': [{'win_id': 1, 'backend_id': 'a'}, {'win_id': 2, 'backend_id': 'b'}]}
        self.probe.validate_durable(live, durable)
        for field, value in (('windows', [1, 2]), ('focus', 1)):
            changed = copy.deepcopy(durable)
            changed['ledger']['orbits'][0][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.probe.validate_durable(live, changed)

    def test_recovered_public_state_cannot_lose_order_focus_or_layout(self):
        before = {'ledger': [{'windows': [{'id': 2, 'focused': True}, {'id': 1, 'focused': False}]}],
                  'state': {'layout': 'triptych', 'focused_title': 'B'}}
        self.probe.validate_state_recovery(before, copy.deepcopy(before))
        for field in ('order', 'focus', 'layout', 'title'):
            after = copy.deepcopy(before)
            if field == 'order': after['ledger'][0]['windows'].reverse()
            elif field == 'focus': after['ledger'][0]['windows'][0]['focused'] = False
            elif field == 'layout': after['state']['layout'] = 'mono'
            else: after['state']['focused_title'] = 'A'
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.probe.validate_state_recovery(before, after)


if __name__ == '__main__':
    unittest.main()
