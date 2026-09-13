#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
builder="$repo_root/packaging/river/build-noble-river-source-kit.sh"
checker="$repo_root/packaging/river/check-noble-river-source-kit.py"
manifest="$repo_root/packaging/river/sources.toml"
tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

[[ -x "$builder" && -x "$checker" ]] || {
    echo "Noble River source-kit producer or checker is missing" >&2
    exit 1
}

cache="$tmpdir/cache"
mkdir -p "$cache/archives" "$cache/tools/zig-x86_64-linux-0.16.0" \
    "$cache/zig-global" "$cache/zig-fetch-root/zig-pkg"
printf 'const std = @import("std");\npub fn build(_: *std.Build) void {}\n' \
    >"$cache/zig-fetch-root/build.zig"
cat >"$cache/tools/zig-x86_64-linux-0.16.0/zig" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$cache/tools/zig-x86_64-linux-0.16.0/zig"

fixture_manifest="$tmpdir/sources.toml"
cp "$manifest" "$fixture_manifest"
while IFS=$'\t' read -r _ filename _ _; do
    : >"$cache/archives/$filename"
done < <("$repo_root/packaging/river/check-closure-manifest.py" \
    --repo-root "$repo_root" --emit archives "$manifest")
while IFS=$'\t' read -r _ _ content_hash; do
    mkdir "$cache/zig-fetch-root/zig-pkg/$content_hash"
    printf 'fixture\n' >"$cache/zig-fetch-root/zig-pkg/$content_hash/input"
done < <("$repo_root/packaging/river/check-closure-manifest.py" \
    --repo-root "$repo_root" --emit zig-dependencies "$manifest")
python3 - "$fixture_manifest" "$cache/archives" <<'PY'
import hashlib
import re
import sys
from pathlib import Path

manifest, archives = map(Path, sys.argv[1:])
text = manifest.read_text(encoding="utf-8")
for filename in re.findall(r'^filename = "([^"]+)"$', text, re.M):
    digest = hashlib.sha256((archives / filename).read_bytes()).hexdigest()
    pattern = rf'(filename = "{re.escape(filename)}"\nurl = "[^"]+"\nsha256 = ")[0-9a-f]{{64}}("\n)'
    text, count = re.subn(pattern, rf'\g<1>{digest}\2', text, count=1)
    assert count == 1
manifest.write_text(text, encoding="utf-8")
PY

sentinels="$tmpdir/sentinels"
mkdir "$sentinels"
for command in curl wget git ssh scp; do
    cat >"$sentinels/$command" <<EOF
#!/usr/bin/env bash
echo "$command invoked" >>"$tmpdir/network.log"
exit 97
EOF
    chmod +x "$sentinels/$command"
done
: >"$tmpdir/network.log"
PATH="$sentinels:/usr/bin:/bin" \
    "$builder" "$fixture_manifest" "$cache" "$tmpdir/kit"
[[ ! -s "$tmpdir/network.log" ]] || {
    echo "source-kit construction invoked a network command" >&2
    exit 1
}
"$checker" "$tmpdir/kit"

cp -R "$cache" "$tmpdir/extra-cache"
: >"$tmpdir/extra-cache/unexpected"
if "$builder" "$fixture_manifest" "$tmpdir/extra-cache" "$tmpdir/extra-kit" \
    >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "source-kit producer accepted an extra cache entry" >&2
    exit 1
elif ! grep -F 'acquired cache inventory differs' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

cp -R "$cache" "$tmpdir/symlink-cache"
rm "$tmpdir/symlink-cache/archives/river-0.4.8.tar.gz"
ln -s /dev/null "$tmpdir/symlink-cache/archives/river-0.4.8.tar.gz"
if "$builder" "$fixture_manifest" "$tmpdir/symlink-cache" "$tmpdir/symlink-kit" \
    >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "source-kit producer accepted a symlinked archive" >&2
    exit 1
elif ! grep -F 'cached archive is not a regular file' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

missing_hash=$(find "$cache/zig-fetch-root/zig-pkg" -mindepth 1 -maxdepth 1 \
    -type d -printf '%f\n' | head -n 1)
cp -R "$cache" "$tmpdir/missing-cache"
rm -rf "$tmpdir/missing-cache/zig-fetch-root/zig-pkg/$missing_hash"
if "$builder" "$fixture_manifest" "$tmpdir/missing-cache" "$tmpdir/missing-kit" \
    >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "source-kit producer accepted a missing Zig package" >&2
    exit 1
elif ! grep -F 'Zig package inventory differs' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

if "$builder" "$fixture_manifest" "$cache" "$tmpdir/kit" \
    >"$tmpdir/out" 2>"$tmpdir/err"; then
    echo "source-kit producer replaced an existing destination" >&2
    exit 1
elif ! grep -F 'output path already exists' "$tmpdir/err" >/dev/null; then
    cat "$tmpdir/err" >&2
    exit 1
fi

echo 'PASS: Noble River retained source-kit fixtures'
