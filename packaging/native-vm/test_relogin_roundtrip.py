"""Reject a restarted facade that retained the previous graphical login."""
import copy
import importlib.util
from pathlib import Path
import unittest


class ReloginTests(unittest.TestCase):
    def test_harness_runs_relogin_after_browser_before_shutdown(self):
        script = Path(__file__).with_name('run-native-session-vm.sh').read_text()
        browser = script.index('timeout 300 python3 "$script_dir/browser_roundtrip.py"')
        relogin = script.index('timeout 240 python3 "$script_dir/relogin_roundtrip.py"')
        shutdown = script.index('    stop_qemu "$qemu_pid"', browser)
        self.assertLess(browser, relogin)
        self.assertLess(relogin, shutdown)

    def setUp(self):
        path = Path(__file__).with_name('relogin_roundtrip.py')
        self.assertTrue(path.exists(), 'relogin acceptance helper is not implemented')
        spec = importlib.util.spec_from_file_location('relogin', path)
        self.probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.probe)
        self.before = {'session': '3', 'login': {'generation': 'A', 'owner_pid': 10, 'start_time': 100},
                       'processes': {'river': [11, 101], 'realm-wm': [12, 102], 'realm-bar': [13, 103]}}
        self.after = {'session': '5', 'login': {'generation': 'B', 'owner_pid': 20, 'start_time': 200},
                      'processes': {'river': [21, 201], 'realm-wm': [22, 202], 'realm-bar': [23, 203]},
                      'survivors': [], 'old_session_present': False}

    def test_new_login_and_pid_reuse_with_new_start_time_are_valid(self):
        self.probe.validate_transition(self.before, self.after, 'B')
        self.after['processes']['river'] = [11, 201]
        self.probe.validate_transition(self.before, self.after, 'B')

    def test_snapshot_checks_old_application_identities_as_well_as_daemons(self):
        previous = copy.deepcopy(self.before)
        previous['applications'] = {'foot': [30, 300], 'zsh': [31, 310]}
        body = self.probe.SNAPSHOT.split('old = ', 1)[1].split('print(json.dumps', 1)[0]
        namespace = {'previous': previous}
        exec('old = ' + body, namespace)
        self.assertIn([30, 300], namespace['old'])
        self.assertIn([31, 310], namespace['old'])

    def test_stale_owner_session_process_or_selection_fails(self):
        for mutation in ('session', 'owner', 'process', 'generation', 'survivor', 'old_session', 'missing'):
            value = copy.deepcopy(self.after)
            if mutation == 'session': value['session'] = '3'
            elif mutation == 'owner': value['login'].update(owner_pid=10, start_time=100)
            elif mutation == 'process': value['processes']['realm-bar'] = [13, 103]
            elif mutation == 'generation': value['login']['generation'] = 'A'
            elif mutation == 'survivor': value['survivors'] = [[10, 100]]
            elif mutation == 'old_session': value['old_session_present'] = True
            else: del value['processes']['realm-wm']
            with self.subTest(mutation=mutation), self.assertRaises((AssertionError, KeyError)):
                self.probe.validate_transition(self.before, value, 'B')


if __name__ == '__main__':
    unittest.main()
