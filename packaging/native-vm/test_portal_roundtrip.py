"""Native portal acceptance uses real responses, not backend presence."""
import importlib.util
from pathlib import Path
import unittest
import re

spec = importlib.util.spec_from_file_location('native_portal', Path(__file__).with_name('portal_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class PortalTests(unittest.TestCase):
    def test_fedora_runtime_dependency_includes_pipewire_server(self):
        spec = Path(__file__).parents[1] / 'fedora' / 'realm.spec'
        requirements = re.findall(r'^Requires:\s+(\S+)', spec.read_text(), re.MULTILINE)
        self.assertIn('pipewire', requirements)

    def test_relative_mouse_rejects_absent_or_malformed_id(self):
        self.assertEqual(probe.relative_mouse('Mouse #2: tablet (absolute)\nMouse #4: mouse\n'), '4')
        for reply in ('Mouse #2: tablet (absolute)', 'Mouse #broken: mouse'):
            with self.assertRaises(AssertionError):
                probe.relative_mouse(reply)

    def evidence(self):
        return {'filechooser': {'elapsed_ms': 10, 'completion': 'response', 'response_code': 1},
                'settings': {'reply_type': '(a{sa{sv}})'},
                'screencast': {'node_id': 42, 'buffer_bytes': 4096, 'width': 32, 'height': 32, 'sha256': 'a' * 64}}

    def test_real_cancel_and_pixels_required(self):
        probe.validate_result(self.evidence())
        for mutation in ('success', 'no_pixels', 'no_response'):
            value = self.evidence()
            if mutation == 'success':
                value['filechooser']['response_code'] = 0
            elif mutation == 'no_pixels':
                value['screencast']['buffer_bytes'] = 0
            else:
                value['filechooser']['completion'] = 'close'
            with self.assertRaises(AssertionError):
                probe.validate_result(value)


if __name__ == '__main__':
    unittest.main()
