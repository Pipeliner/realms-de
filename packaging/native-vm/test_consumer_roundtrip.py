"""Native selected-theme acceptance must reject wrong real consumer evidence."""
import importlib.util
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest

from consumer_process import require_one, selected_pids

spec = importlib.util.spec_from_file_location('consumer', Path(__file__).with_name('consumer_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ConsumerTests(unittest.TestCase):
    def test_btop_concurrent_runtime_views_cleanup_on_terminal_group_termination(self):
        adapter = Path(__file__).resolve().parents[2] / 'configs/templates/btop-launch.sh'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            generation = root / 'generation with spaces'
            (generation / 'btop/themes').mkdir(parents=True)
            (generation / 'btop/btop.conf').write_text('selected config')
            binary = root / 'btop'
            binary.write_text('#!/bin/sh\nset -eu\n'
                              'printf "%s" "$XDG_CONFIG_HOME" > "$1"\n'
                              'trap "exit 0" TERM\nwhile :; do sleep 1; done\n')
            binary.chmod(0o700)
            children = []
            try:
                for index in range(2):
                    children.append(subprocess.Popen(
                        ['sh', str(adapter), str(root / f'proof{index}')],
                        env={**os.environ, 'PATH': str(root) + ':' + os.environ['PATH'],
                             'REALM_GENERATION': str(generation), 'XDG_RUNTIME_DIR': str(root)},
                        start_new_session=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
                deadline = time.monotonic() + 3
                while not all((root / f'proof{i}').exists() and (root / f'proof{i}').stat().st_size for i in range(2)):
                    self.assertLess(time.monotonic(), deadline, 'btop adapter did not become ready')
                    time.sleep(.01)
                views = [(root / f'proof{i}').read_text() for i in range(2)]
                self.assertNotEqual(*views)
                for child in children:
                    os.killpg(child.pid, signal.SIGTERM)
                    child.wait(timeout=3)
                self.assertTrue(all(not Path(view).exists() for view in views))
                self.assertEqual((generation / 'btop/btop.conf').read_text(), 'selected config')
            finally:
                for child in children:
                    if child.poll() is None:
                        os.killpg(child.pid, signal.SIGKILL)
                        child.wait(timeout=3)

    def test_trace_fixture_follows_package_only_probe_and_precedes_consumers(self):
        host = Path(__file__).with_name('run-native-session-vm.sh').read_text()
        self.assertIn('consumer-trace "$target"', host)
        self.assertLess(host.index('guest-probe.sh" probe "$target"'), host.index('consumer-trace "$target"'))
        self.assertLess(host.index('consumer-trace "$target"'), host.index('"$script_dir/consumer_roundtrip.py"'))

    def test_btop_requires_successful_own_pid_opens_of_both_selected_files(self):
        self.assertTrue(callable(getattr(probe, 'btop_file_reads', None)),
                        'native consumer must validate real btop file reads')
        trace = ('42 openat(AT_FDCWD, "/A/btop/btop.conf", O_RDONLY) = 4\n'
                 '[pid 42] openat(AT_FDCWD, "/A/btop/themes/realm.theme", O_RDONLY) = 5\n')
        self.assertEqual(len(probe.btop_file_reads(trace, 42, '/A')), 2)
        for bad in (trace.replace('42', '43'), trace.replace('/A/', '/B/'),
                    trace.replace('= 5', '= -1 ENOENT'), trace.splitlines()[0],
                    trace.replace('realm.theme"', 'realm.theme.backup"')):
            with self.subTest(trace=bad), self.assertRaises(AssertionError):
                probe.btop_file_reads(bad, 42, '/A')

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

    def test_selected_terminal_ignores_distro_server_and_other_generation(self):
        commands = {
            '11': b'foot\0--server\0',
            '12': b'foot\0--config=/generation/A/foot/foot.ini\0zsh\0',
            '13': b'foot\0--config=/generation/B/foot/foot.ini\0zsh\0',
        }
        matches = selected_pids('foot', list(commands), '/generation/A/foot/foot.ini',
                                lambda pid: commands[pid])
        self.assertEqual(require_one('foot', matches), '12')

    def test_selected_terminal_rejects_missing_and_duplicate_matches(self):
        selected = b'foot\0--config=/generation/A/foot/foot.ini\0zsh\0'
        commands = {'11': b'foot\0--server\0', '12': selected, '13': selected}
        with self.assertRaises(AssertionError):
            require_one('foot', selected_pids('foot', ['11'], '/generation/A/foot/foot.ini',
                                              lambda pid: commands[pid]))
        with self.assertRaises(AssertionError):
            require_one('foot', selected_pids('foot', list(commands), '/generation/A/foot/foot.ini',
                                              lambda pid: commands[pid]))

    def test_consumer_probe_command_passes_exact_selected_config(self):
        self.assertEqual(probe.consumer_process_command('foot', '/generation/A/foot/foot.ini'),
                         'sudo python3 /var/tmp/realm-native-vm/consumer_process.py foot '
                         '/generation/A/foot/foot.ini present')

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

    def test_real_shell_cache_transition_requires_external_dump_and_clean_generation(self):
        clean = ['zsh/.zshenv', 'zsh/.zshrc']
        before = {'entries': clean, 'dump_present': False}
        after = {'entries': clean, 'dump_present': True}
        probe.assert_shell_cache_transition(before, after)
        for mutation in (
            ({'entries': clean + ['zsh/.zcompdump'], 'dump_present': False}, after),
            (before, {'entries': clean + ['zsh/.zcompdump'], 'dump_present': True}),
            (before, {'entries': clean, 'dump_present': False}),
        ):
            with self.subTest(mutation=mutation), self.assertRaises(AssertionError):
                probe.assert_shell_cache_transition(*mutation)


if __name__ == '__main__':
    unittest.main()
