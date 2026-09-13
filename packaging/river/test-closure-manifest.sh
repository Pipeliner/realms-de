#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
checker="$repo_root/packaging/river/check-closure-manifest.py"
manifest="$repo_root/packaging/river/sources.toml"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

pass_count=0

expect_pass() {
    local name=$1
    shift
    if ! "$@" >"$tmpdir/stdout" 2>"$tmpdir/stderr"; then
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
    shift 2
    if "$@" >"$tmpdir/stdout" 2>"$tmpdir/stderr"; then
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

mutate() {
    local operation=$1
    local destination=$2
    python3 - "$operation" "$manifest" "$destination" <<'PY'
import re
import sys
from pathlib import Path

operation, source, destination = sys.argv[1:]
text = Path(source).read_text(encoding="utf-8")

if operation == "missing":
    text = re.sub(
        r'\n\[\[archive\]\]\nname = "wlroots"\n.*?(?=\n\[\[|\Z)',
        "",
        text,
        count=1,
        flags=re.S,
    )
elif operation == "extra":
    text += '''\n[[archive]]
name = "extra"
version = "1.0"
filename = "extra.tar.gz"
url = "https://example.invalid/extra.tar.gz"
sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
role = "test"
'''
elif operation == "duplicate":
    block = re.search(r'\n\[\[archive\]\]\n.*?(?=\n\[\[|\Z)', text, re.S)
    assert block is not None
    text += block.group(0)
elif operation == "digest":
    text = re.sub(r'sha256 = "[0-9a-f]{64}"', 'sha256 = "NOT-A-DIGEST"', text, count=1)
elif operation == "lock":
    text = re.sub(r'nixpkgs_revision = "[0-9a-f]{40}"', 'nixpkgs_revision = "0000000000000000000000000000000000000000"', text, count=1)
elif operation == "zig-hash":
    text = re.sub(r'content_hash = "[^"]+"', 'content_hash = "invalid"', text, count=1)
else:
    raise SystemExit(f"unknown mutation: {operation}")

Path(destination).write_text(text, encoding="utf-8")
PY
}

expect_pass canonical "$checker" --repo-root "$repo_root" "$manifest"
if ! "$checker" --repo-root "$repo_root" --emit archives "$manifest" \
    | grep -Fx -- $'meson\tmeson-1.4.0.tar.gz\thttps://github.com/mesonbuild/meson/releases/download/1.4.0/meson-1.4.0.tar.gz\t8fd6630c25c27f1489a8a0392b311a60481a3c161aa699b330e25935b750138d' \
        >/dev/null; then
    echo "not ok $((pass_count + 1)) - canonical Meson build-tool pin is absent" >&2
    exit 1
fi
pass_count=$((pass_count + 1))
printf 'ok %s - canonical Meson build-tool pin\n' "$pass_count"
if ! "$checker" --repo-root "$repo_root" --emit archives "$manifest" \
    | grep -Fx -- $'libinput\tlibinput-1.31.3.tar.gz\thttps://gitlab.freedesktop.org/libinput/libinput/-/archive/1.31.3/libinput-1.31.3.tar.gz\tb6749bf6f1890f6631c0a70a027c35fec9d2e096a39f720548896e41474a9854' \
        >/dev/null; then
    echo "not ok $((pass_count + 1)) - canonical libinput runtime pin is absent" >&2
    exit 1
fi
pass_count=$((pass_count + 1))
printf 'ok %s - canonical libinput runtime pin\n' "$pass_count"

require_zig_record() {
    local name=$1
    local expected=$2
    if ! "$checker" --repo-root "$repo_root" --emit zig-dependencies "$manifest" \
        | grep -Fx -- "$expected" >/dev/null; then
        echo "not ok $((pass_count + 1)) - canonical $name Zig pin is absent" >&2
        exit 1
    fi
    pass_count=$((pass_count + 1))
    printf 'ok %s - canonical %s Zig pin\n' "$pass_count" "$name"
}

aro_commit=5f5a050569a95ecc40a426f0c3666ae7ef987ede
require_zig_record aro \
    $'aro\tgit+https://github.com/Vexu/arocc#'"$aro_commit"$'\taro-0.0.0-JSD1Qi7QNgDnfcrdEJf82v3o6MhZySjYVrtdfEf3E4Se'
require_zig_record xkbcommon-wlroots \
    $'xkbcommon-wlroots\thttps://codeberg.org/ifreund/zig-xkbcommon/archive/v0.3.0.tar.gz\txkbcommon-0.3.0-VDqIe3K9AQB2fG5ZeRcMC9i7kfrp5m2rWgLrmdNn9azr'

for case in missing extra duplicate digest lock zig-hash; do
    mutated="$tmpdir/$case.toml"
    mutate "$case" "$mutated"
    case "$case" in
        missing | extra) diagnostic="archive inventory differs" ;;
        duplicate) diagnostic="duplicate archive name" ;;
        digest) diagnostic="invalid archive sha256" ;;
        lock) diagnostic="does not match flake.lock" ;;
        zig-hash) diagnostic="invalid Zig content hash" ;;
    esac
    expect_fail "$case" "$diagnostic" \
        "$checker" --repo-root "$repo_root" "$mutated"
done

cache_manifest="$tmpdir/cache.toml"
archive_dir="$tmpdir/archives"
python3 - "$manifest" "$cache_manifest" "$archive_dir" <<'PY'
import hashlib
import re
import sys
from pathlib import Path

source, destination, archive_dir = map(Path, sys.argv[1:])
empty_digest = hashlib.sha256(b"").hexdigest()
text = source.read_text(encoding="utf-8")
text = re.sub(r'sha256 = "[0-9a-f]{64}"', f'sha256 = "{empty_digest}"', text)
archive_dir.mkdir()
for filename in re.findall(r'^filename = "([^"]+)"$', text, re.M):
    (archive_dir / filename).write_bytes(b"")
destination.write_text(text, encoding="utf-8")
PY

expect_pass archive-cache "$checker" --repo-root "$repo_root" \
    --archive-dir "$archive_dir" "$cache_manifest"
printf 'corrupt' >"$archive_dir/river-0.4.8.tar.gz"
expect_fail corrupt-cache "cached archive digest mismatch: river-0.4.8.tar.gz" \
    "$checker" --repo-root "$repo_root" --archive-dir "$archive_dir" "$cache_manifest"
: >"$archive_dir/river-0.4.8.tar.gz"
: >"$archive_dir/unexpected.tar.gz"
expect_fail extra-cache "archive cache inventory differs" \
    "$checker" --repo-root "$repo_root" --archive-dir "$archive_dir" "$cache_manifest"

printf 'PASS: %s Ubuntu River closure manifest fixtures\n' "$pass_count"
