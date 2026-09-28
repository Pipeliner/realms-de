"""Selected native toolkit proof rejects inherited environment without consumption."""

import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


source = Path(__file__).with_name('toolkit_roundtrip.py')
if source.exists():
    spec = importlib.util.spec_from_file_location('native_toolkit', source)
    probe = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(probe)
else:
    probe = None


ROOT = '/home/alice/.config/realm/generated/generations/A'


def observation(kind):
    expected = {
        'gtk3': ('gtk3-widget-factory', 'togglebutton',
                 ROOT + '/share/themes/realm/gtk-3.0/gtk.css'),
        'gtk4': ('GTK Widget Factory', 'Page 1',
                 ROOT + '/share/themes/realm/gtk-4.0/gtk.css'),
        'qt6': ('Qt6 Configuration Tool', 'Appearance',
                ROOT + '/qt6ct/qt6ct.conf'),
    }
    title, label, path = expected[kind]
    paths = [path]
    if kind == 'qt6':
        paths.append(ROOT + '/qt6ct/colors/realm.conf')
    return {
        'state': {'reply': 'state', 'data': {
            'orbits': [{'windows': 1}], 'focused_title': title}},
        'ocr': 'header\n' + label + '\n',
        'environment': {
            'REALM_GENERATION': ROOT, 'GTK_THEME': 'realm',
            'QT_QPA_PLATFORMTHEME': 'qt6ct',
            'XDG_DATA_DIRS': ROOT + '/share:/usr/share',
            'XDG_CONFIG_DIRS': ROOT + ':/etc/xdg',
        },
        'trace': ''.join(f'3835293 openat(AT_FDCWD, "{item}", O_RDONLY) = {index + 3}\n'
                         for index, item in enumerate(paths)),
        'stderr': '', 'exit_status': 0,
    }


class ToolkitTests(unittest.TestCase):
    def validate(self, kind, value):
        self.assertIsNotNone(probe, 'native toolkit probe is absent')
        return probe.validate_probe(kind, ROOT, value)

    def test_each_real_toolkit_requires_its_selected_files(self):
        for kind in ('gtk3', 'gtk4', 'qt6'):
            with self.subTest(kind=kind):
                self.validate(kind, observation(kind))

    def test_environment_alone_cannot_replace_successful_open(self):
        for kind in ('gtk3', 'gtk4', 'qt6'):
            with self.subTest(kind=kind):
                value = observation(kind)
                value['trace'] = value['trace'].replace(' = 3', ' = -1 ENOENT')
                with self.assertRaisesRegex(AssertionError, 'selected.*open'):
                    self.validate(kind, value)
        value = observation('qt6')
        value['trace'] = value['trace'].splitlines(keepends=True)[0]
        with self.assertRaisesRegex(AssertionError, 'selected.*open'):
            self.validate('qt6', value)
        value = observation('gtk3')
        value['trace'] = value['trace'].replace(' openat(', ' read(')
        with self.assertRaisesRegex(AssertionError, 'selected.*open'):
            self.validate('gtk3', value)

    def test_strace_pid_column_may_be_padded(self):
        value = observation('gtk3')
        value['trace'] = value['trace'].replace('3835293 openat(', '1234  openat(')
        self.validate('gtk3', value)
        value['trace'] = value['trace'].replace('openat(', 'close(')
        with self.assertRaisesRegex(AssertionError, 'selected.*open'):
            self.validate('gtk3', value)

    def test_lost_artifact_transport_does_not_hide_acceptance_failure(self):
        self.assertTrue(hasattr(probe, 'retain_probe_artifacts'))
        with tempfile.TemporaryDirectory() as directory:
            with mock.patch.object(probe.subprocess, 'run', side_effect=subprocess.TimeoutExpired('ssh', 15)):
                errors = probe.retain_probe_artifacts(
                    ['ssh', 'guest'], 'gtk3', '/tmp/realm-native-toolkit', Path(directory)
                )
        self.assertEqual(len(errors), 4)
        self.assertTrue(all('TimeoutExpired' in error for error in errors))

    def test_generation_b_and_unrelated_css_cannot_pass(self):
        value = observation('gtk3')
        value['trace'] = value['trace'].replace('/generations/A/', '/generations/B/')
        with self.assertRaisesRegex(AssertionError, 'selected.*open'):
            self.validate('gtk3', value)
        value = observation('gtk4')
        value['environment']['REALM_GENERATION'] = ROOT[:-1] + 'B'
        with self.assertRaisesRegex(AssertionError, 'generation'):
            self.validate('gtk4', value)

    def test_wrong_focus_window_count_and_terminal_only_ocr_fail(self):
        value = observation('gtk3')
        value['state']['data']['focused_title'] = 'foot'
        with self.assertRaisesRegex(AssertionError, 'focused'):
            self.validate('gtk3', value)
        value = observation('gtk3')
        value['state']['data']['orbits'][0]['windows'] = 2
        with self.assertRaisesRegex(AssertionError, 'window'):
            self.validate('gtk3', value)
        value = observation('gtk3')
        value['ocr'] = 'gtk3-widget-factory in Foot command\n'
        with self.assertRaisesRegex(AssertionError, 'visible'):
            self.validate('gtk3', value)

    def test_theme_diagnostic_and_nonzero_exit_fail(self):
        value = observation('gtk3')
        value['stderr'] = 'Gtk-WARNING: theme CSS failed to parse\n'
        with self.assertRaisesRegex(AssertionError, 'diagnostic'):
            self.validate('gtk3', value)
        value = observation('qt6')
        value['stderr'] = 'qt6ct config invalid\n'
        with self.assertRaisesRegex(AssertionError, 'diagnostic'):
            self.validate('qt6', value)
        value = observation('gtk4')
        value['exit_status'] = 1
        with self.assertRaisesRegex(AssertionError, 'exit'):
            self.validate('gtk4', value)

    def test_launcher_runs_real_distro_binary_under_file_trace_without_theme_override(self):
        self.assertIsNotNone(probe, 'native toolkit probe is absent')
        self.assertTrue(hasattr(probe, 'launcher_assets'), 'launcher capture is absent')
        for kind, executable in (
            ('gtk3', '/usr/bin/gtk3-widget-factory'),
            ('gtk4', '/usr/bin/gtk4-widget-factory'),
            ('qt6', '/usr/bin/qt6ct'),
        ):
            with self.subTest(kind=kind):
                script, desktop = probe.launcher_assets(kind, '/tmp/realm-toolkit')
                self.assertIn('strace -f -qq -e trace=openat', script)
                self.assertIn(executable, script)
                self.assertIn('env -0', script)
                self.assertIn('printf', script)
                self.assertNotIn('export GTK_THEME=', script)
                self.assertNotIn('export QT_QPA_PLATFORMTHEME=', script)
                self.assertIn('Exec=/tmp/realm-toolkit/' + kind + '.sh', desktop)
                self.assertIn('Name=realm' + kind + 'probe', desktop)

    def test_launcher_waits_for_process_then_filters_before_accepting(self):
        self.assertTrue(hasattr(probe, 'open_launcher'), 'launcher sequence is absent')
        order = []
        probe.open_launcher(
            'realmgtk3probe',
            lambda key: order.append(('key', key)),
            lambda: order.append(('ready', None)),
            lambda name: order.append(('visible', name)),
        )
        self.assertEqual(order[0:2], [('key', 'meta_l-d'), ('ready', None)])
        self.assertEqual(order[-2:], [('visible', 'realmgtk3probe'), ('key', 'ret')])
        self.assertEqual([value for kind, value in order if kind == 'key'][1:-1],
                         list('realmgtk3probe'))


if __name__ == '__main__':
    unittest.main()
