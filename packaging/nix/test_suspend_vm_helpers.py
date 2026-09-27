#!/usr/bin/env python3
"""Exercise the VM's actual logind JSON helpers without a VM or system bus."""
import ast
import datetime as dt
import json
import os
from pathlib import Path
import shlex
import socket
import time
from types import SimpleNamespace
import textwrap
from queue import Queue

# The derivation evaluator supplies actual VM options; no nested Nix process
# may initialize store/profile state inside this builder.
fixture = json.loads(os.environ["REALM_SUSPEND_FIXTURE"])
assert '-global ICH9-LPC.enable_tco=off' in fixture['options'], fixture
assert 'initcall_debug' in fixture['params'], fixture
assert 'no_console_suspend' in fixture['params'], fixture
assert fixture['imports'] == ['production-module'] and fixture['manager'] == {}, fixture

source = Path(__file__).with_name("checks.nix").read_text()
source = textwrap.dedent(
    source[source.index("      import datetime"):source.rfind("    '';" )]
)
tree = ast.parse(source)
gate = next((node for node in tree.body if isinstance(node, ast.If)
             and isinstance(node.test, ast.Compare)
             and ast.unparse(node.test) == "os.environ.get('REALM_POST_MVP_SUSPEND') == '1'"), None)
assert gate is not None, 'default MVP still executes deferred suspend acceptance'
for value, expected in ((None, []), ('', []), ('0', []), ('1', ['suspend'])):
    namespace = {'os': SimpleNamespace(environ={} if value is None else
                                      {'REALM_POST_MVP_SUSPEND': value}), 'observed': []}
    inert_gate = ast.If(test=gate.test, body=ast.parse("observed.append('suspend')").body,
                       orelse=[])
    exec(compile(ast.fix_missing_locations(ast.Module(body=[inert_gate], type_ignores=[])),
                 '<real suspend opt-in>', 'exec'), namespace)
    assert namespace['observed'] == expected
gate_index = tree.body.index(gate)
before = ast.unparse(ast.Module(body=tree.body[:gate_index], type_ignores=[]))
after = ast.unparse(ast.Module(body=tree.body[gate_index + 1:], type_ignores=[]))
assert "idle_results['stopped_without_live_idle'] = True" in before
assert 'lock_results.append(' in before
assert 'terminal_pid = wait_for_single_user_process' in after
assert "control('quit')" in after
assert 'arm_suspend_socket_timeouts' in ast.unparse(gate)
# Existing helper and acceptance tests still exercise the retained opt-in body.
tree.body[gate_index:gate_index + 1] = gate.body
suspend_try = next(node for node in tree.body if isinstance(node, ast.Try)
                   and any(isinstance(handler.type, ast.Name) and handler.type.id == 'TimeoutError'
                           for handler in node.handlers))
guest_calls = []
artifacts = {}
cleanup_namespace = {
    'machine': SimpleNamespace(log=lambda value: None,
                               send_monitor_command=lambda command: 'running',
                               execute=lambda *args, **kwargs: guest_calls.append(args) or (0, '')),
    'suspend_results': {'transport_unusable': True},
    'suspend_host_diagnostics': lambda: {'console_tail': ['last resume callback']},
    'suspend_socket_timeouts': [], 'json': json, 'idle_unit': 'realm-idle.service',
    'as_alice': lambda *args: shlex.join(args), 'DIAGNOSTIC_TIMEOUT': dt.timedelta(seconds=10),
    'write_artifact': lambda name, value: artifacts.update({name: value}),
}
exec(compile(ast.Module(body=suspend_try.finalbody, type_ignores=[]), '<suspend-cleanup>', 'exec'), cleanup_namespace)
assert not guest_calls, 'timed-out guest transport reused during failure cleanup'
assert 'last resume callback' in artifacts['suspend-roundtrip.json']
cleanup_namespace['suspend_results'] = {}
guest_calls.clear()
def stop_then_timeout(*args, **kwargs):
    guest_calls.append(args)
    if len(guest_calls) == 1:
        return (0, 'idle stopped')
    raise TimeoutError('journal transport stalled')
cleanup_namespace['machine'].execute = stop_then_timeout
following_assertions = []
for node in tree.body[tree.body.index(suspend_try) + 1:]:
    if not isinstance(node, ast.Assert):
        break
    following_assertions.append(node)
rejected = False
try:
    exec(compile(ast.Module(body=suspend_try.finalbody + following_assertions, type_ignores=[]),
                 '<late-cleanup-timeout>', 'exec'), cleanup_namespace)
except AssertionError:
    rejected = True
assert cleanup_namespace['suspend_results']['idle_stop_status'] == 0
assert cleanup_namespace['suspend_results']['transport_unusable'] is True
assert rejected, 'journal timeout after successful stop permits later guest commands'
deadline_function = next((node for node in tree.body if isinstance(node, ast.FunctionDef)
                          and node.name == 'arm_suspend_socket_timeouts'), None)
assert deadline_function is not None, 'suspend host transport has no independent socket deadline'
deadline_namespace = {}
exec(compile(ast.Module(body=[deadline_function], type_ignores=[]), '<suspend-deadlines>', 'exec'), deadline_namespace)
for channel in ('shell', 'monitor'):
    reader, writer = socket.socketpair()
    try:
        reader.settimeout(None)
        machine = SimpleNamespace(shell=reader if channel == 'shell' else None,
                                  monitor=reader if channel == 'monitor' else None)
        previous = deadline_namespace['arm_suspend_socket_timeouts'](machine, 0.03)
        started = time.monotonic()
        try:
            reader.recv(1)
            raise AssertionError('unresponsive peer unexpectedly returned')
        except TimeoutError:
            assert time.monotonic() - started < 1
        for transport, timeout in previous:
            transport.settimeout(timeout)
        assert reader.gettimeout() is None
    finally:
        reader.close()
        writer.close()
# Execute the real acceptance assertions with good and deliberately bad
# observations. Removing a guard, or weakening its comparison, must fail here.
assertions = [node for node in ast.walk(tree) if isinstance(node, ast.Assert)]
checks = [
    ('"suspended" in monitor_status', {'monitor_status': 'paused (suspended)'},
     {'monitor_status': 'running'}),
    ('request_time <= ready_time <= sleep_time',
     {'request_time': 10, 'ready_time': 11, 'sleep_time': 12},
     {'request_time': 10, 'ready_time': 13, 'sleep_time': 12}),
    ('"PM: suspend entry (deep)" in kernel_sleep and "PM: suspend exit" in kernel_sleep',
     {'kernel_sleep': 'PM: suspend entry (deep)\nPM: suspend exit'},
     {'kernel_sleep': 'PM: suspend entry (deep)'}),
    ('lock_systemctl("is-active") == "active"',
     {'lock_systemctl': lambda *args: 'active'},
     {'lock_systemctl': lambda *args: 'inactive'}),
    ('suspend_results["inhibitors_after"]',
     {'suspend_results': {'inhibitors_after': [['sleep', 'swayidle']]}},
     {'suspend_results': {'inhibitors_after': []}}),
]
for expression, accepted, rejected in checks:
    wanted = ast.dump(ast.parse(expression, mode='eval').body)
    matches = [node for node in assertions if ast.dump(node.test) == wanted]
    assert matches, f'critical acceptance assertion missing: {expression}'
    assertion = ast.Assert(test=matches[0].test, msg=None)
    compiled = compile(ast.fix_missing_locations(ast.Module(body=[assertion], type_ignores=[])),
                       '<real suspend acceptance assertion>', 'exec')
    exec(compiled, accepted)
    try:
        exec(compiled, rejected)
    except AssertionError:
        pass
    else:
        raise AssertionError(f'invalid suspend observation accepted: {expression}')
print('suspend fixture experiment: options isolated; real suspend/readiness/resume/lock guards PASS')
functions = {"login_call", "login_property", "own_sleep_inhibitors", "suspend_host_diagnostics"}
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


class ExitedProcess:
    def poll(self):
        return 1


class QMP:
    def __init__(self):
        self.pending_events = Queue()
        self.reads = 0

    def read_pending_messages(self):
        self.reads += 1
        if self.reads == 1:
            self.pending_events.put({"event": "SHUTDOWN", "data": {"reason": "guest-reset"}})

    def wait_for_event(self, **kwargs):
        raise AssertionError("empty-queue wait can hang beyond its timeout")


machine.process = ExitedProcess()
machine.qmp_client = QMP()
machine.full_console_log = ["old boot line"] * 200 + ["resume failed"]
diagnostics = namespace["suspend_host_diagnostics"]()
assert diagnostics["qemu_returncode"] == 1
assert diagnostics["qmp_events"] == [{"event": "SHUTDOWN", "data": {"reason": "guest-reset"}}]
assert diagnostics["console_tail"][-1] == "resume failed"
assert len(diagnostics["console_tail"]) <= 80
assert 0 < machine.qmp_client.reads <= 128


class BrokenQMP(QMP):
    def read_pending_messages(self):
        raise BrokenPipeError("monitor closed")


machine.qmp_client = BrokenQMP()
diagnostics = namespace["suspend_host_diagnostics"]()
assert diagnostics["qemu_returncode"] == 1
assert "monitor closed" in diagnostics["qmp_error"]
assert diagnostics["console_tail"][-1] == "resume failed"
print("suspend host diagnostics: exited QEMU, shutdown reason, closed monitor, bounded reads PASS")
