#!/usr/bin/env python3
"""Reject cycles in the shipped Realm graphical-session unit ordering."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
UNIT_ROOT = ROOT / "packaging/systemd"


def directives(path: Path) -> dict[str, dict[str, list[str]]]:
    sections: dict[str, dict[str, list[str]]] = {}
    section = ""
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1]
            sections.setdefault(section, {})
            continue
        key, separator, value = line.partition("=")
        if separator and section:
            sections[section].setdefault(key, []).extend(value.split())
    return sections


def ordering_edges() -> dict[str, set[str]]:
    units = {
        path.name: directives(path)
        for path in (
            UNIT_ROOT / "realm-session.target",
            UNIT_ROOT / "realm-wm.service",
            UNIT_ROOT / "realm-bar.service",
        )
    }
    edges: dict[str, set[str]] = {}

    def before(first: str, second: str) -> None:
        edges.setdefault(first, set()).add(second)
        edges.setdefault(second, set())

    for name, unit in units.items():
        for target in unit.get("Unit", {}).get("Before", []):
            before(name, target)
        for prerequisite in unit.get("Unit", {}).get("After", []):
            before(prerequisite, name)
        # A target's default dependencies order every unit in its .wants
        # directory before the target. The packages materialize WantedBy as
        # those exact symlinks, so model that installed graph here.
        for target in unit.get("Install", {}).get("WantedBy", []):
            before(name, target)
    return edges


def cycle(edges: dict[str, set[str]]) -> list[str] | None:
    visiting: list[str] = []
    visited: set[str] = set()

    def visit(node: str) -> list[str] | None:
        if node in visiting:
            start = visiting.index(node)
            return visiting[start:] + [node]
        if node in visited:
            return None
        visiting.append(node)
        for successor in sorted(edges.get(node, set())):
            found = visit(successor)
            if found is not None:
                return found
        visiting.pop()
        visited.add(node)
        return None

    for node in sorted(edges):
        found = visit(node)
        if found is not None:
            return found
    return None


def reachable(edges: dict[str, set[str]], start: str, finish: str) -> bool:
    pending = [start]
    seen: set[str] = set()
    while pending:
        node = pending.pop()
        if node == finish:
            return True
        if node not in seen:
            seen.add(node)
            pending.extend(edges.get(node, set()))
    return False


def validate_home_manager(source: str) -> None:
    target = source.split("systemd.user.targets.realm-session = {", 1)[1].split(
        "systemd.user.services.realm-wm = {", 1
    )[0]
    wm = source.split("systemd.user.services.realm-wm = {", 1)[1].split(
        "systemd.user.services.realm-session-abort = {", 1
    )[0]
    bar = source.split("systemd.user.services.realm-bar = {", 1)[1]

    required = (
        (target, 'BindsTo = [ "graphical-session.target" ];', "target BindsTo"),
        (target, 'Before = [ "graphical-session.target" ];', "target Before"),
        (target, 'Wants = [ "graphical-session-pre.target" ];', "target Wants"),
        (target, 'After = [ "graphical-session-pre.target" ];', "target After"),
        (wm, 'Install.WantedBy = [ "realm-session.target" ];', "wm WantedBy"),
        (
            bar,
            'Install.WantedBy = lib.optionals cfg.startBar [ "realm-session.target" ];',
            "bar WantedBy",
        ),
        (bar, 'After = [ "realm-wm.service" ];', "bar After"),
    )
    for block, token, label in required:
        if token not in block:
            raise ValueError(f"Home Manager {label} edge is absent")

    if 'After = [ "graphical-session.target" ];' in wm:
        raise ValueError("Home Manager wm reverses the graphical target order")
    after = bar.split("After =", 1)[1].split("];", 1)[0]
    if '"graphical-session.target"' in after:
        raise ValueError("Home Manager bar reverses the graphical target order")
    for name, block in (("wm", wm), ("bar", bar)):
        part_of = block.split("PartOf =", 1)[1].split("];", 1)[0]
        for target_name in ("realm-session.target", "graphical-session.target"):
            if f'"{target_name}"' not in part_of:
                raise ValueError(f"Home Manager {name} PartOf lacks {target_name}")


class SessionOrderingTests(unittest.TestCase):
    def test_installed_unit_graph_is_acyclic_and_orders_session_services(self):
        edges = ordering_edges()
        found = cycle(edges)
        self.assertIsNone(found, f"ordering cycle: {' -> '.join(found or [])}")
        self.assertTrue(reachable(edges, "realm-wm.service", "realm-bar.service"))
        self.assertTrue(reachable(edges, "realm-bar.service", "realm-session.target"))
        self.assertTrue(
            reachable(edges, "realm-session.target", "graphical-session.target")
        )

    def test_session_services_follow_both_teardown_anchors(self):
        expected = {"realm-session.target", "graphical-session.target"}
        for name in ("realm-wm.service", "realm-bar.service"):
            unit = directives(UNIT_ROOT / name)
            self.assertEqual(set(unit["Unit"]["PartOf"]), expected, name)

    def test_shipped_target_retains_its_pull_in_and_readiness_edges(self):
        target = directives(UNIT_ROOT / "realm-session.target")["Unit"]
        self.assertEqual(target["BindsTo"], ["graphical-session.target"])
        self.assertEqual(target["Before"], ["graphical-session.target"])
        self.assertEqual(target["Wants"], ["graphical-session-pre.target"])
        self.assertEqual(target["After"], ["graphical-session-pre.target"])

    def test_home_manager_copy_has_the_same_ordering(self):
        source = (ROOT / "packaging/nix/home-manager-module.nix").read_text(
            encoding="utf-8"
        )
        validate_home_manager(source)

    def test_home_manager_edge_omissions_are_rejected(self):
        source = (ROOT / "packaging/nix/home-manager-module.nix").read_text(
            encoding="utf-8"
        )
        edges = (
            'BindsTo = [ "graphical-session.target" ];',
            'Before = [ "graphical-session.target" ];',
            'Wants = [ "graphical-session-pre.target" ];',
            'After = [ "graphical-session-pre.target" ];',
            'Install.WantedBy = [ "realm-session.target" ];',
            'Install.WantedBy = lib.optionals cfg.startBar [ "realm-session.target" ];',
        )
        for edge in edges:
            with self.subTest(edge=edge):
                self.assertIn(edge, source)
                with self.assertRaisesRegex(ValueError, "edge is absent"):
                    validate_home_manager(source.replace(edge, "", 1))


if __name__ == "__main__":
    unittest.main()
