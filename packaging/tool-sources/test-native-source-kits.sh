#!/bin/sh
# The distributable native source kits retain packaging inputs, not a checkout.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
builder=$root/packaging/tool-sources/build-native-source-kits.sh
checker=$root/packaging/tool-sources/check-native-source-kit.py
tmp=$(mktemp -d "${TMPDIR:-/tmp}/helm-native-source-kits.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

if [ ! -x "$builder" ] || [ ! -x "$checker" ]; then
    echo "native source-kit producer or checker is missing" >&2
    exit 1
fi

mkdir -p "$tmp/sentinels"
for command in git curl wget ssh scp; do
    sed "s/@COMMAND@/$command/g" >"$tmp/sentinels/$command" <<'EOF'
#!/bin/sh
printf 'forbidden source-kit command: @COMMAND@ %s\n' "$*" >>"${HELM_KIT_SENTINEL_LOG:?}"
exit 97
EOF
    chmod +x "$tmp/sentinels/$command"
done
: >"$tmp/sentinel.log"
PATH="$tmp/sentinels:/usr/bin:/bin" \
    HELM_KIT_SENTINEL_LOG="$tmp/sentinel.log" \
    "$builder" "$tmp/output"

if [ -s "$tmp/sentinel.log" ]; then
    echo "native source-kit production invoked Git or a network command" >&2
    cat "$tmp/sentinel.log" >&2
    exit 1
fi

debian=$tmp/output/helm-debian-0.1.0
rpm_archive=$tmp/output/helm-0.1.0.tar.gz
rpm_spec=$tmp/output/helm.spec
if [ ! -d "$debian" ] || [ ! -f "$rpm_archive" ] || [ ! -f "$rpm_spec" ]; then
    echo "native source-kit producer omitted an output" >&2
    exit 1
fi

"$checker" debian "$debian"
mkdir -p "$tmp/rpm"
tar -C "$tmp/rpm" -xzf "$rpm_archive"
rpm=$tmp/rpm/helm-0.1.0
"$checker" rpm "$rpm"

if "$checker" debian "$root" >"$tmp/out" 2>"$tmp/err"; then
    echo "full checkout was accepted as a Debian source kit" >&2
    exit 1
elif ! grep -F 'Debian source kit top-level inventory differs from policy' "$tmp/err" >/dev/null; then
    echo "full checkout Debian rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    exit 1
fi
if "$checker" rpm "$root" >"$tmp/out" 2>"$tmp/err"; then
    echo "full checkout was accepted as an RPM source kit" >&2
    exit 1
elif ! grep -F 'RPM source kit top-level inventory differs from policy' "$tmp/err" >/dev/null; then
    echo "full checkout RPM rejection had the wrong reason" >&2
    cat "$tmp/err" >&2
    exit 1
fi

diff -qr "$root/packaging/debian" "$debian/debian"
diff -qr "$root/packaging/fedora" "$rpm/packaging/fedora"
cmp "$root/packaging/fedora/helm.spec" "$rpm_spec"
for helper in check-bundle-linkage.py check-native-source-kit.py stage-helm-workspace.py; do
    cmp "$root/packaging/tool-sources/$helper" \
        "$debian/packaging/tool-sources/$helper"
    cmp "$root/packaging/tool-sources/$helper" \
        "$rpm/packaging/tool-sources/$helper"
done
diff -qr "$root/packaging/tool-sources/bundles/helm-workspace" \
    "$debian/packaging/tool-sources/bundles/helm-workspace"
diff -qr "$root/packaging/tool-sources/bundles/helm-workspace" \
    "$rpm/packaging/tool-sources/bundles/helm-workspace"
