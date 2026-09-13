#!/bin/sh
# B4/G15 schema guard: Realm's complete terminal profile must target the
# selected tools and must keep every selector generation-local.
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
template=$root/configs/templates/yazi-theme.toml
manager=$root/configs/templates/yazi.toml
keymap=$root/configs/templates/yazi-keymap.toml
profile=$root/configs/templates/zshrc
starship=$root/configs/templates/starship.toml
btop=$root/configs/templates/btop.conf
support=$root/packaging/nix/support.nix
checks=$root/packaging/nix/checks.nix
debian_rules=$root/packaging/debian/rules
fedora_spec=$root/packaging/fedora/realm.spec
native_fixture=$root/packaging/tool-sources/test-native-builds.sh

require() {
    grep -F -q "$1" "$template" || {
        echo "missing Yazi v25.4 field: $1" >&2
        exit 1
    }
}

forbid() {
    if grep -F -q "$1" "$template"; then
        echo "legacy Yazi field remains: $1" >&2
        exit 1
    fi
}

require '[manager]'
require '[mode]'
require 'normal_main ='
require 'normal_alt ='
require 'select_main ='
require 'select_alt ='
require 'unset_main ='
require 'unset_alt ='
require 'perm_sep ='
require 'perm_type ='
require 'perm_read ='
require 'perm_write ='
require 'perm_exec ='

forbid '[mgr]'
forbid 'mode_normal ='
forbid 'mode_select ='
forbid 'mode_unset ='
forbid 'permissions_t ='
forbid 'permissions_r ='
forbid 'permissions_w ='
forbid 'permissions_x ='
forbid 'permissions_s ='

grep -F -q 'ratio = [1, 4, 3]' "$manager"
grep -F -q 'linemode = "size"' "$manager"
grep -F -q 'on = "<C-p>"' "$keymap"
# The dollar and command-substitution forms are the literal selectors under test.
# shellcheck disable=SC2016
grep -F -q 'btop --config \"$REALM_GENERATION/btop/btop.conf\"' "$keymap"
# shellcheck disable=SC2016
grep -F -q 'eval "$(starship init zsh)"' "$profile"
# shellcheck disable=SC2016
grep -F -q 'command btop --config "$REALM_GENERATION/btop/btop.conf"' "$profile"
if grep -F -q -- '--config=' "$keymap" || grep -F -q -- '--config=' "$profile"; then
    echo "pinned btop launch uses an unsupported equals-form config option" >&2
    exit 1
fi
python3 - "$starship" <<'PY'
import sys

lines = open(sys.argv[1], encoding="utf-8").read().splitlines()
start = lines.index("[character]") + 1
end = next(
    (index for index in range(start, len(lines)) if lines[index].startswith("[")),
    len(lines),
)
character = lines[start:end]
assert 'format = "$symbol"' in character, character
for field in ("success_symbol", "error_symbol", "vimcmd_symbol"):
    assert any(
        line.startswith(field + " = ") and line.endswith(' "')
        for line in character
    ), field
PY
grep -F -q 'color_theme = "realm"' "$btop"
grep -F -q 'shown_boxes = "cpu mem net proc"' "$btop"
grep -F -q 'dontUpdateAutotoolsGnuConfigScripts = true;' "$support" || {
    echo "retained Nix Yazi permits automatic vendor mutation" >&2
    exit 1
}
if grep -F -q "assert '\${pkgs.yazi}/bin' in daemon_path" "$checks"; then
    echo "Nix VM expects ambient Yazi instead of Realm's retained selection" >&2
    exit 1
fi
grep -F -q "assert '\${realmYazi}/bin' in daemon_path" "$checks" || {
    echo "Nix VM does not assert Realm's retained Yazi path" >&2
    exit 1
}

# These are literal Nix/Python command fragments, not shell expansions.
# shellcheck disable=SC2016
if grep -F -q 'test \"$(yazi --version)\"' "$checks"; then
    echo "Nix VM runs Yazi's terminal discovery on the driver control terminal" >&2
    exit 1
fi
# shellcheck disable=SC2016
grep -F -q '${pkgs.util-linux}/bin/setsid --wait yazi --version' "$checks" || {
    echo "Nix VM Yazi identity probe is not detached from the driver terminal" >&2
    exit 1
}
grep -F -q '</dev/null >/tmp/realm-yazi-version 2>&1 &&' "$checks" || {
    echo "Nix VM Yazi identity probe does not preserve producer failure" >&2
    exit 1
}

for recipe in "$debian_rules" "$fedora_spec"; do
    if grep -F -q 'REALM_RUNTIME_PATH' "$recipe" \
        || grep -F -q 'runtime_path=' "$recipe"; then
        echo "native runtime validation depends on cross-phase environment: $recipe" >&2
        exit 1
    fi
    grep -F -q 'PATH="/usr/bin:/bin"' "$recipe" || {
        echo "native runtime validation does not select the fixed runtime PATH: $recipe" >&2
        exit 1
    }
    grep -F -q 'env -u CARGO_HOME -u CARGO_TARGET_DIR' "$recipe" || {
        echo "native runtime validation retains build-only Cargo selectors: $recipe" >&2
        exit 1
    }
done

if grep -F -q 'REALM_RUNTIME_PATH=' "$native_fixture"; then
    echo "native fixture still supplies a cross-phase runtime PATH" >&2
    exit 1
fi

runtime_fixture=$(mktemp -d "${TMPDIR:-/tmp}/realm-runtime-path.XXXXXX")
trap 'rm -rf "$runtime_fixture"' EXIT HUP INT TERM
mkdir -p "$runtime_fixture/sentinels"
for command in python3 git cargo; do
    printf '%s\n' '#!/bin/sh' 'exit 97' >"$runtime_fixture/sentinels/$command"
    chmod +x "$runtime_fixture/sentinels/$command"
done
if ! PATH="$runtime_fixture/sentinels:/usr/bin:/bin" \
    REALM_RUNTIME_PATH="$runtime_fixture/sentinels" \
    CARGO_HOME="$runtime_fixture/cargo-home" \
    CARGO_TARGET_DIR="$runtime_fixture/cargo-target" \
    RUSTC="$runtime_fixture/sentinels/rustc" \
    REALM_SENTINEL_LOG="$runtime_fixture/sentinel.log" \
    env -u CARGO_HOME -u CARGO_TARGET_DIR -u RUSTC -u REALM_SENTINEL_LOG \
        PATH="/usr/bin:/bin" python3 - "$runtime_fixture/sentinels" <<'PY'
import os
import shutil
import sys

sentinels = sys.argv[1]
assert os.environ["PATH"] == "/usr/bin:/bin"
assert os.environ["REALM_RUNTIME_PATH"] == sentinels
assert shutil.which("python3") == "/usr/bin/python3"
for name in ("CARGO_HOME", "CARGO_TARGET_DIR", "RUSTC", "REALM_SENTINEL_LOG"):
    assert name not in os.environ
PY
then
    echo "runtime phase did not isolate a dropped selector and poisoned caller PATH" >&2
    exit 1
fi

grep -F -q "starship_version_first=\$(printf '%s\\n' \"\$starship_version\" | sed -n '1p')" \
    "$native_fixture" || {
    echo "native fixture does not isolate Starship's stable version line" >&2
    exit 1
}
# These dollars are the literal shell operands whose removal is under test.
# shellcheck disable=SC2016
if grep -F -q 'cmp "$debian_yazi_binary" "$rpm_yazi_binary"' "$native_fixture"; then
    echo "native fixture compares distro-specific Yazi ELF bytes" >&2
    exit 1
fi
