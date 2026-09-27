#!/usr/bin/env python3
"""Source package contract checks; actual installations are checked only in CI."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class IdlePackaging(unittest.TestCase):
    def test_debian_declares_lock_runtime_dependencies(self):
        control = (ROOT / 'packaging/debian/control').read_text()
        depends = re.search(r'^Depends: (.*(?:\n[ \t]+.*)*)', control, re.M).group(1)
        for dependency in ('swayidle', 'swaylock (>= 1.7)', 'brightnessctl'):
            self.assertIn(dependency, depends)

    def test_debian_installs_helpers_and_units(self):
        entries = (ROOT / 'packaging/debian/install').read_text().splitlines()
        entries = [line.split() for line in entries if line and not line.startswith('#')]
        for name in ('realm-idle', 'realm-backlight'):
            self.assertIn([f'debian/realm-workspace/source/packaging/session/{name}', 'usr/bin'], entries)
        for name in ('realm-idle.service', 'realm-lock.service'):
            self.assertIn([f'debian/realm-workspace/source/packaging/systemd/{name}', 'usr/lib/systemd/user'], entries)

    def test_fedora_declares_dependencies_and_payload(self):
        spec = (ROOT / 'packaging/fedora/realm.spec').read_text()
        requirements = re.findall(r'^Requires:\s+(.*)$', spec, re.M)
        for dependency in ('swayidle', 'swaylock >= 1.7', 'brightnessctl'):
            self.assertIn(dependency, requirements)
        files = spec.split('%files', 1)[1]
        for name in ('realm-idle', 'realm-backlight'):
            self.assertIn(f'%{{_bindir}}/{name}\n', files)
        for name in ('realm-idle.service', 'realm-lock.service'):
            self.assertIn(f'%{{_userunitdir}}/{name}\n', files)

    def test_staged_idle_cannot_be_automatically_enabled(self):
        unit = (ROOT / 'packaging/systemd/realm-idle.service').read_text()
        self.assertNotIn('[Install]', unit)
        self.assertNotIn('WantedBy=', unit)


if __name__ == '__main__':
    unittest.main()
