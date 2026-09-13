#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-private-resolutions.py"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
prefix="$tmpdir/prefix"
mkdir -p "$prefix/lib"

pass_count=0

expect_pass() {
    local name=$1
    local log=$2
    if ! "$checker" "$prefix" "$log" >"$tmpdir/stdout" 2>"$tmpdir/stderr"; then
        printf 'not ok %s - %s\n' "$((pass_count + 1))" "$name" >&2
        sed 's/^/  /' "$tmpdir/stderr" >&2
        exit 1
    fi
    pass_count=$((pass_count + 1))
    printf 'ok %s - %s\n' "$pass_count" "$name"
}

expect_fail() {
    local name=$1
    local diagnostic=$2
    local log=$3
    if "$checker" "$prefix" "$log" >"$tmpdir/stdout" 2>"$tmpdir/stderr"; then
        printf 'not ok %s - %s unexpectedly passed\n' "$((pass_count + 1))" "$name" >&2
        exit 1
    fi
    if ! grep -F -- "$diagnostic" "$tmpdir/stderr" >/dev/null; then
        printf 'not ok %s - %s lacked diagnostic: %s\n' \
            "$((pass_count + 1))" "$name" "$diagnostic" >&2
        sed 's/^/  /' "$tmpdir/stderr" >&2
        exit 1
    fi
    pass_count=$((pass_count + 1))
    printf 'ok %s - %s\n' "$pass_count" "$name"
}

canonical="$tmpdir/canonical.log"
python3 - "$prefix" "$canonical" <<'PY'
import sys
from pathlib import Path

prefix, output = map(Path, sys.argv[1:])
names = [
    "libwlroots-0.20.so.20",
    "libwayland-server.so.0",
    "libwayland-client.so.0",
    "libdrm.so.2",
    "libdrm_amdgpu.so.1",
    "libinput.so.10",
    "libpixman-1.so.0",
    "libxkbcommon.so.0",
    "libdisplay-info.so.0",
]
output.write_text(
    "".join(f"\t{name} => {prefix / 'lib' / name} (0x1)\n" for name in names),
    encoding="utf-8",
)
PY

expect_pass canonical "$canonical"

mixed="$tmpdir/mixed.log"
cp "$canonical" "$mixed"
printf '\tlibdrm.so.2 => /usr/lib/x86_64-linux-gnu/libdrm.so.2 (0x2)\n' >>"$mixed"
expect_fail mixed-private-and-system "private-family resolution escaped prefix: libdrm.so.2" "$mixed"

external_client="$tmpdir/external-client.log"
sed "s|$prefix/lib/libwayland-client.so.0|/usr/lib/libwayland-client.so.0|" \
    "$canonical" >"$external_client"
expect_fail external-wayland-client \
    "private-family resolution escaped prefix: libwayland-client.so.0" "$external_client"

external_companion="$tmpdir/external-companion.log"
sed "s|$prefix/lib/libdrm_amdgpu.so.1|/usr/lib/libdrm_amdgpu.so.1|" \
    "$canonical" >"$external_companion"
expect_fail external-libdrm-companion \
    "private-family resolution escaped prefix: libdrm_amdgpu.so.1" "$external_companion"

external_libinput="$tmpdir/external-libinput.log"
sed "s|$prefix/lib/libinput.so.10|/usr/lib/libinput.so.10|" \
    "$canonical" >"$external_libinput"
expect_fail external-libinput \
    "private-family resolution escaped prefix: libinput.so.10" "$external_libinput"

missing="$tmpdir/missing.log"
grep -v 'libdisplay-info' "$canonical" >"$missing"
expect_fail missing-required-family "required private family absent: libdisplay-info" "$missing"

printf 'PASS: %s Ubuntu River private-resolution fixtures\n' "$pass_count"
