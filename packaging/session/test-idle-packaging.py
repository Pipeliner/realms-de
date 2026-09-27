#!/usr/bin/env python3
"""Source package contract checks; actual installations are checked only in CI."""
from pathlib import Path
import configparser
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class IdlePackaging(unittest.TestCase):
    def test_locker_tracks_all_processes_without_guessing_main_pid(self):
        unit = configparser.ConfigParser(interpolation=None)
        unit.read(ROOT / 'packaging/systemd/realm-lock.service')
        service = unit['Service']
        self.assertEqual(service.get('Type'), 'forking')
        self.assertEqual(service.get('ExitType'), 'cgroup')
        self.assertEqual(service.get('GuessMainPID'), 'no')
        self.assertEqual(service.get('Restart'), 'no')
        self.assertEqual(service.get('ExecStart'), '/usr/bin/swaylock -f -C /dev/null')

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

    def test_fresh_login_starts_idle_but_not_locker(self):
        unit = configparser.ConfigParser(interpolation=None)
        unit.read(ROOT / 'packaging/systemd/realm-idle.service')
        self.assertEqual(unit.get('Install', 'WantedBy'), 'realm-session.target')
        self.assertNotIn('[Install]', (ROOT / 'packaging/systemd/realm-lock.service').read_text())
        for name in ('packaging/debian/rules', 'packaging/fedora/realm.spec', 'packaging/nix/package.nix'):
            source = (ROOT / name).read_text()
            with self.subTest(recipe=name):
                self.assertIn('ln -s', source)
                self.assertIn('../realm-idle.service', source)
                self.assertIn('realm-session.target.wants/realm-idle.service', source)
        rpm_files = (ROOT / 'packaging/fedora/realm.spec').read_text().split('%files', 1)[1]
        self.assertIn('%{_userunitdir}/realm-session.target.wants/realm-idle.service', rpm_files)
        self.assertIn('systemd.user.services.realm-idle.wantedBy = [ "realm-session.target" ];',
                      (ROOT / 'packaging/nix/nixos-module.nix').read_text())

    def test_installed_probes_observe_automatic_start_before_stopping_for_controlled_tests(self):
        native = (ROOT / 'packaging/native-vm/guest-probe.sh').read_text()
        self.assertIn('realm-bar.service realm-idle.service; do', native)
        self.assertIn('user_command systemctl --user stop realm-idle.service', native)
        nix = (ROOT / 'packaging/nix/checks.nix').read_text()
        self.assertIn('"realm-idle.service", user="alice", timeout=STARTUP_TIMEOUT', nix)
        self.assertIn('"stop", "realm-idle.service"', nix)
        self.assertNotIn('test ! -e ${realm}/lib/systemd/user/realm-session.target.wants/realm-idle.service', nix)


if __name__ == '__main__':
    unittest.main()
