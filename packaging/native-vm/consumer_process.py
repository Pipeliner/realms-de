"""Inspect the exact selected native consumer, excluding distro Foot servers."""

import json
from pathlib import Path
import subprocess
import sys


def selected_pids(name, pids, selected_config, read_cmdline):
    if name != 'foot':
        return pids
    expected = ('--config=' + selected_config).encode()
    return [pid for pid in pids if expected in read_cmdline(pid).split(b'\0')]


def require_one(name, pids):
    assert len(pids) == 1, (name, pids)
    return pids[0]


def recovery_app_pids(pids, selected_config, read_cmdline):
    matches = selected_pids('foot', pids, selected_config, read_cmdline)
    assert len(matches) == 3, ('recovery terminals', matches)
    return matches


def main():
    name, selected_config, mode = sys.argv[1:]
    assert mode in ('present', 'absent'), mode
    query = subprocess.run(['pgrep', '-u', 'alice', '-x', name], capture_output=True, text=True)
    assert query.returncode in (0, 1), query.stderr
    pids = query.stdout.split()
    matches = selected_pids(name, pids, selected_config,
                            lambda pid: (Path('/proc') / pid / 'cmdline').read_bytes())
    if mode == 'absent':
        assert not matches, (name, matches)
        return
    pid = require_one(name, matches)
    base = Path('/proc') / pid
    fields = (base / 'stat').read_text().rsplit(')', 1)[1].split()
    assert fields[0] not in ('Z', 'X'), fields
    environment = dict(item.split('=', 1) for item in (base / 'environ').read_bytes().decode().split('\0') if '=' in item)
    print(json.dumps({'pid': int(pid), 'parent_pid': int(fields[1]), 'start_time': int(fields[19]),
        'executable': str((base / 'exe').resolve(strict=True)),
        'arguments': (base / 'cmdline').read_bytes().decode().rstrip('\0').split('\0'),
        'environment': environment}))


if __name__ == '__main__':
    main()
