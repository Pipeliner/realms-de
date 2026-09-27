#!/usr/bin/env python3
"""Exercise the VM's actual logind JSON helpers without a VM or system bus."""
import ast
import datetime as dt
import json
from pathlib import Path
import shlex
import textwrap

source = Path(__file__).with_name("checks.nix").read_text()
source = textwrap.dedent(
    source[source.index("      import datetime"):source.rfind("    '';" )]
)
tree = ast.parse(source)
functions = {"login_call", "login_property", "own_sleep_inhibitors"}
constants = {"login_bus", "login_object", "DIAGNOSTIC_TIMEOUT"}
selected = [
    node for node in tree.body
    if (isinstance(node, ast.FunctionDef) and node.name in functions)
    or (isinstance(node, ast.Assign) and len(node.targets) == 1
        and isinstance(node.targets[0], ast.Name)
        and node.targets[0].id in constants)
]
assert len(selected) == len(functions) + len(constants)


class Machine:
    # Actual read-only busctl responses captured 2026-09-27. get-property's
    # scalar differs from a method reply's outer argument array.
    property_reply = '{"type":"t","data":30000000}'
    call_reply = (
        '{"type":"a(ssssuu)","data":[[["shutdown",'
        '"Unattended Upgrades Shutdown",'
        '"Stop ongoing upgrades or perform upgrades before shutdown",'
        '"delay",0,866]]]}'
    )

    def succeed(self, command, timeout):
        assert command.startswith(
            "timeout 5 busctl --system --timeout=5 --json=short "
        ), command
        assert timeout == dt.timedelta(seconds=10), timeout
        return self.property_reply if " get-property " in command else self.call_reply


machine = Machine()
namespace = dict(machine=machine, json=json, shlex=shlex, dt=dt, alice_uid=4242)
exec(compile(ast.Module(body=selected, type_ignores=[]),
             "<actual VM helpers>", "exec"), namespace)
assert namespace["login_property"]("InhibitDelayMaxUSec") == 30000000
assert namespace["own_sleep_inhibitors"]("866") == []
valid = ["sleep", "swayidle", "sleep", "delay", 4242, 987]
machine.call_reply = json.dumps({"type": "a(ssssuu)", "data": [[
    valid,
    ["sleep", "other", "sleep", "delay", 1000, 987],
    ["sleep", "other", "sleep", "delay", 4242, 988],
    ["sleep", "other", "sleep", "block", 4242, 987],
    ["shutdown", "other", "sleep", "delay", 4242, 987],
]]})
assert namespace["own_sleep_inhibitors"]("987") == [valid]
print("suspend VM helpers: captured JSON shapes, owned inhibitor filtering, bounded queries PASS")
