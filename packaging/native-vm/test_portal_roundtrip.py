"""Native portal acceptance uses real responses, not backend presence."""
import importlib.util
from pathlib import Path
import unittest
import re
import hashlib

spec = importlib.util.spec_from_file_location('native_portal', Path(__file__).with_name('portal_roundtrip.py'))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class PortalTests(unittest.TestCase):
    def test_fedora_runtime_dependency_includes_pipewire_server(self):
        spec = Path(__file__).parents[1] / 'fedora' / 'realm.spec'
        requirements = re.findall(r'^Requires:\s+(\S+)', spec.read_text(), re.MULTILINE)
        self.assertIn('pipewire', requirements)

    def evidence(self):
        return {'filechooser': {'elapsed_ms': 10, 'completion': 'response', 'response_code': 1},
                'file_selection': {'elapsed_ms': 10, 'completion': 'response', 'response_code': 0,
                    'uri': 'file:///tmp/realmfile', 'bytes': 29,
                    'sha256': hashlib.sha256(b'Realm portal selection proof\n').hexdigest()},
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

    def test_selection_requires_exact_uri_content_and_success(self):
        for key, value in [('uri', 'file:///tmp/wrong'), ('response_code', 1),
                           ('bytes', 0), ('sha256', '0' * 64), ('elapsed_ms', 2001)]:
            evidence = self.evidence()
            evidence['file_selection'][key] = value
            with self.subTest(key=key), self.assertRaises(AssertionError):
                probe.validate_result(evidence)

    def test_selection_types_short_fixture_path_then_explicit_open(self):
        self.assertTrue(hasattr(probe, 'select_file'), 'real selection keyboard path missing')
        keys = []
        probe.select_file(keys.append)
        self.assertEqual(keys, ['ctrl-l', 'slash', 't', 'm', 'p', 'slash',
                               'r', 'e', 'a', 'l', 'm', 'f', 'i', 'l', 'e', 'alt-o'])


if __name__ == '__main__':
    unittest.main()
