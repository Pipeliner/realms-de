#!/usr/bin/env python3
"""Command-boundary tests only; never install packages or lock the host."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent


class IdleGlue(unittest.TestCase):
    def invoke(self, helper, args=(), status=0, brightness='50'):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / 'calls.jsonl'
            stub = '#!/usr/bin/env python3\nimport json,os,sys\nwith open(os.environ["CALLS"],"a") as f: f.write(json.dumps(sys.argv[1:])+"\\n")\nif "get" in sys.argv: print(os.environ["BRIGHTNESS"])\nsys.exit(int(os.environ["STATUS"]))\n'
            for command in ('swayidle', 'brightnessctl'):
                path = root / command
                path.write_text(stub)
                path.chmod(0o755)
            env = dict(os.environ, PATH=f'{root}:/usr/bin:/bin', CALLS=str(log), STATUS=str(status), BRIGHTNESS=brightness)
            result = subprocess.run(['sh', str(HERE / helper), *args], env=env,
                                    capture_output=True, text=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []
            return result, calls

    def test_idle_has_exact_independent_timers_and_synchronous_lock_hooks(self):
        result, calls = self.invoke('realm-idle')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [[
            '-w', '-C', '/dev/null',
            'timeout', '300', 'realm-backlight dim', 'resume', 'realm-backlight restore',
            'timeout', '600', 'systemctl --user start realm-lock.service',
            'before-sleep', 'systemctl --user start realm-lock.service',
            'lock', 'systemctl --user start realm-lock.service',
        ]])

    def test_idle_failure_reaches_service_supervisor(self):
        result, _ = self.invoke('realm-idle', status=7)
        self.assertEqual(result.returncode, 7)

    def test_backlight_uses_save_relative_decrease_and_restore(self):
        for action, expected in [('dim', ['--class=backlight', 'set', '90%-']),
                                 ('restore', ['--class=backlight', '--restore'])]:
            with self.subTest(action=action):
                result, calls = self.invoke('realm-backlight', [action])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(calls, ([['--class=backlight', '--save', 'get']] if action == 'dim' else []) + [expected])

    def test_dim_never_raises_an_already_minimal_backlight(self):
        for brightness in ('0', '1'):
            result, calls = self.invoke('realm-backlight', ['dim'], brightness=brightness)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls, [['--class=backlight', '--save', 'get']])

    def test_unsupported_backlight_is_logged_without_failing_idle(self):
        result, _ = self.invoke('realm-backlight', ['dim'], status=1)
        self.assertEqual(result.returncode, 0)
        self.assertIn('backlight', result.stderr)

    def test_invalid_invocations_never_call_tools(self):
        for helper, args in [('realm-idle', ['extra']), ('realm-backlight', []),
                             ('realm-backlight', ['unknown']), ('realm-backlight', ['dim', 'extra'])]:
            with self.subTest(helper=helper, args=args):
                result, calls = self.invoke(helper, args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(calls, [])


if __name__ == '__main__':
    unittest.main()
