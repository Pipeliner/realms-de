"""Bounded host QMP input for the real native output chooser."""
import json
import socket
import time


def qmp_command(path, command, arguments=None, timeout=5):
    deadline = time.monotonic() + timeout
    pending = bytearray()
    received = 0
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        def remaining():
            seconds = deadline - time.monotonic()
            if seconds <= 0:
                raise TimeoutError('QMP exchange deadline')
            connection.settimeout(seconds)

        def message():
            nonlocal received
            while b'\n' not in pending:
                remaining()
                chunk = connection.recv(4096)
                if not chunk:
                    raise RuntimeError('QMP closed before response')
                received += len(chunk)
                if received > 1024 * 1024:
                    raise RuntimeError('QMP exchange exceeded reply bound')
                pending.extend(chunk)
            line, _, tail = pending.partition(b'\n')
            pending[:] = tail
            value = json.loads(line)
            if not isinstance(value, dict):
                raise RuntimeError(f'Invalid QMP message: {value!r}')
            return value

        def exchange(name, args, identity):
            request = {'execute': name, 'id': identity}
            if args is not None:
                request['arguments'] = args
            remaining()
            connection.sendall(json.dumps(request).encode() + b'\n')
            while True:
                remaining()
                reply = message()
                if 'event' in reply:
                    continue
                if reply.get('id') != identity or 'return' not in reply:
                    raise RuntimeError(f'QMP {name} failed: {reply!r}')
                return reply['return']

        remaining()
        connection.connect(path)
        if 'QMP' not in message():
            raise RuntimeError('Missing QMP greeting')
        exchange('qmp_capabilities', None, 'capabilities')
        return exchange(command, arguments, 'input')


def pointer_command(path, evidence, command, arguments=None, send=qmp_command):
    record = {'command': command, 'arguments': arguments}
    evidence.setdefault('pointer_commands', []).append(record)
    try:
        record['reply'] = send(path, command, arguments)
    except Exception as error:
        record['error'] = str(error)
        raise
    return record['reply']


def position_pointer(path, evidence, send=qmp_command):
    mice = pointer_command(path, evidence, 'query-mice', send=send)
    evidence['mice'] = mice
    assert isinstance(mice, list), mice
    active = [mouse for mouse in mice if isinstance(mouse, dict) and mouse.get('current') is True]
    assert len(active) == 1 and active[0].get('absolute') is True, mice
    evidence['pointer'] = active[0]
    evidence['pointer_target'] = [16384, 16384]
    pointer_command(path, evidence, 'input-send-event', {'events': [
        {'type': 'abs', 'data': {'axis': 'x', 'value': 16384}},
        {'type': 'abs', 'data': {'axis': 'y', 'value': 16384}},
    ]}, send)


def click_pointer(path, evidence, send=qmp_command):
    for down in (True, False):
        pointer_command(path, evidence, 'input-send-event', {'events': [
            {'type': 'btn', 'data': {'button': 'left', 'down': down}},
        ]}, send)
