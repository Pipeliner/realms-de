"""Keep the Nix VM wired to the shared real-keyboard acceptance contract."""
from pathlib import Path
import ast
import textwrap
import unittest


class WindowControlsTests(unittest.TestCase):
    def test_observation_accumulator_has_a_list_type_independent_of_result_flags(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        start = source.index('      # Shared installed-window keyboard acceptance.')
        end = source.index('      def window_count', start)
        tree = ast.parse(textwrap.dedent(source[start:end]))
        append = next(node for node in ast.walk(tree)
                      if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                      and node.func.attr == 'append')
        self.assertIsInstance(append.func.value, ast.Name,
                              'append must target the typed list, not a bool/list dictionary lookup')
        name = append.func.value.id
        annotation = next(node for node in tree.body
                          if isinstance(node, ast.AnnAssign) and node.target.id == name)
        self.assertEqual(ast.unparse(annotation.annotation), 'list[dict[str, object]]')
        namespace = {}
        initializers = [node for node in tree.body if isinstance(node, (ast.Assign, ast.AnnAssign))]
        exec(compile(ast.Module(body=initializers, type_ignores=[]), '<window-result>', 'exec'), namespace)
        namespace[name].append({'step': 'focus', 'focused_title': 'Realm window A'})
        self.assertEqual(namespace['window_result']['observations'],
                         [{'step': 'focus', 'focused_title': 'Realm window A'}])
        self.assertIs(namespace['window_result']['passed'], False)

    def test_browser_evidence_uses_current_driver_api(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        self.assertFalse('machine.copy_from_vm(' in source, 'deprecated copy API is rejected by pinned driver typecheck')
        self.assertIn('machine.copy_from_machine(browser_evidence, "browser-screencast")', source)

    def test_shared_keyboard_probe_runs_before_existing_demo(self):
        source = Path(__file__).with_name('checks.nix').read_text()
        start = source.index('      # Shared installed-window keyboard acceptance.')
        end = source.index('      for number in range(1, 4):', start)
        block = source[start:end]
        opt_in = block.index("      ${lib.optionalString waybarComparison ''")
        opt_out = block.index("      ''}", opt_in) + len("      ''}")
        self.assertIn('waybar_probe.exercise(', block[opt_in:opt_out])
        ordinary_block = block[:opt_in] + block[opt_out:]
        compile('\n'.join(line[6:] for line in ordinary_block.splitlines()), '<window-probe>', 'exec')
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
