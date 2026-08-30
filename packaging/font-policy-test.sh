#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH='' cd "$script_dir/.." && pwd)
guard="$script_dir/check-font-policy.sh"

if [ ! -x "$guard" ]; then
    echo "FAIL: missing executable font-policy guard: $guard" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/helm-font-policy-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

inputs='
docs/adr/0012-font-fallback-is-a-contract.md
docs/INSTALL.md
packaging/debian/cargo-deb.toml.fragment
packaging/debian/control
packaging/fedora/helm.spec
packaging/nix/nixos-module.nix'

make_fixture() {
    destination=$1
    mkdir -p "$destination"
    for path in $inputs; do
        mkdir -p "$destination/$(dirname "$path")"
        cp "$repo_root/$path" "$destination/$path"
    done
}

expect_pass() {
    "$guard" --root "$1" >/dev/null
}

expect_fail() {
    fixture=$1
    shift
    if "$guard" --root "$fixture" >/dev/null 2>&1; then
        echo "FAIL: expected font-policy guard to reject fixture" >&2
        exit 1
    fi
}

replace_once() {
    path=$1
    old=$2
    new=$3
    output="$path.replaced"
    awk -v old="$old" -v new="$new" '
        index($0, old) && !done { sub(old, new); done = 1 }
        { print }
        END { exit(done ? 0 : 42) }
    ' "$path" >"$output" || {
        rm -f "$output"
        echo "FAIL: fixture mutation did not replace expected text" >&2
        exit 1
    }
    mv "$output" "$path"
}

make_fixture "$tmp_dir/canonical"
expect_pass "$tmp_dir/canonical"

cp -R "$tmp_dir/canonical" "$tmp_dir/debian-hard-symbol"
replace_once "$tmp_dir/debian-hard-symbol/packaging/debian/control" \
    'xdg-desktop-portal-wlr,' 'xdg-desktop-portal-wlr,\n         fonts-symbola,'
expect_fail "$tmp_dir/debian-hard-symbol"

cp -R "$tmp_dir/canonical" "$tmp_dir/fedora-hard-symbol"
replace_once "$tmp_dir/fedora-hard-symbol/packaging/fedora/helm.spec" \
    'Recommends:     google-noto-sans-symbols2-fonts' 'Requires:       google-noto-sans-symbols2-fonts'
expect_fail "$tmp_dir/fedora-hard-symbol"

cp -R "$tmp_dir/canonical" "$tmp_dir/debian-hard-plex"
replace_once "$tmp_dir/debian-hard-plex/packaging/debian/control" \
    'xdg-desktop-portal-wlr,' 'xdg-desktop-portal-wlr,\n         fonts-ibm-plex,'
expect_fail "$tmp_dir/debian-hard-plex"

cp -R "$tmp_dir/canonical" "$tmp_dir/debian-predepends"
replace_once "$tmp_dir/debian-predepends/packaging/debian/control" \
    'Architecture: any' 'Architecture: any\nPre-Depends: fonts-symbola'
expect_fail "$tmp_dir/debian-predepends"

cp -R "$tmp_dir/canonical" "$tmp_dir/cargo-hard-font"
replace_once "$tmp_dir/cargo-hard-font/packaging/debian/cargo-deb.toml.fragment" \
    'foot,' 'foot, fonts-ibm-plex,'
expect_fail "$tmp_dir/cargo-hard-font"

cp -R "$tmp_dir/canonical" "$tmp_dir/debian-suggested-symbol"
replace_once "$tmp_dir/debian-suggested-symbol/packaging/debian/control" \
    'Recommends: fonts-ibm-plex,' 'Suggests: fonts-ibm-plex,'
expect_fail "$tmp_dir/debian-suggested-symbol"

cp -R "$tmp_dir/canonical" "$tmp_dir/cargo-missing-plex"
replace_once "$tmp_dir/cargo-missing-plex/packaging/debian/cargo-deb.toml.fragment" \
    'fonts-ibm-plex, ' ''
expect_fail "$tmp_dir/cargo-missing-plex"

cp -R "$tmp_dir/canonical" "$tmp_dir/rpm-qualified-font"
replace_once "$tmp_dir/rpm-qualified-font/packaging/fedora/helm.spec" \
    'BuildRequires:  rust >= 1.85' 'Requires(pre):  ibm-plex-mono-fonts\nBuildRequires:  rust >= 1.85'
expect_fail "$tmp_dir/rpm-qualified-font"

cp -R "$tmp_dir/canonical" "$tmp_dir/font-byte"
mkdir -p "$tmp_dir/font-byte/assets"
printf 'not a real font\n' >"$tmp_dir/font-byte/assets/NerdSymbols.ttf"
expect_fail "$tmp_dir/font-byte"

cp -R "$tmp_dir/canonical" "$tmp_dir/nix-nerd-font"
replace_once "$tmp_dir/nix-nerd-font/packaging/nix/nixos-module.nix" \
    'pkgs.ibm-plex' 'pkgs.ibm-plex\n      pkgs.nerd-fonts.symbols-only'
expect_fail "$tmp_dir/nix-nerd-font"

echo "font package policy fixtures: pass"
