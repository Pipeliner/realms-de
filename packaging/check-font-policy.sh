#!/bin/sh

set -eu

usage() {
    echo "usage: $0 [--root PATH]" >&2
    exit 2
}

root=.
if [ "$#" -eq 2 ] && [ "$1" = '--root' ]; then
    root=$2
elif [ "$#" -ne 0 ]; then
    usage
fi

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

require_file() {
    [ -f "$root/$1" ] || fail "missing policy input: $1"
}

for path in \
    docs/adr/0012-font-fallback-is-a-contract.md \
    docs/INSTALL.md \
    packaging/debian/cargo-deb.toml.fragment \
    packaging/debian/control \
    packaging/fedora/helm.spec \
    packaging/nix/nixos-module.nix; do
    require_file "$path"
done

adr="$root/docs/adr/0012-font-fallback-is-a-contract.md"
grep -Fq 'Helm does not redistribute Symbola or a generic Nerd Font.' "$adr" \
    || fail 'ADR 0012 does not prohibit Symbola/Nerd Font redistribution'
grep -Fq 'may only recommend distribution-reviewed symbol-font or' "$adr" \
    || fail 'ADR 0012 does not retain the optional symbol-font policy'

debian="$root/packaging/debian/control"
if awk '
    /^(Pre-Depends|Depends):/ { field = "hard" }
    /^[A-Za-z-]+:/ && !/^(Pre-Depends|Depends):/ { field = "other" }
    field == "hard" && !/^[[:space:]]*#/ && /(fonts-symbola|fonts-ibm-plex)/ { found = 1 }
    END { exit(found ? 0 : 1) }
' "$debian"; then
    fail 'Debian control makes Symbola a hard dependency'
fi
for font in fonts-symbola fonts-ibm-plex; do
    awk -v font="$font" '
        /^Recommends:/ { field = "recommend" }
        /^[A-Za-z-]+:/ && !/^Recommends:/ { field = "other" }
        field == "recommend" && !/^[[:space:]]*#/ && index($0, font) { found = 1 }
        END { exit(found ? 0 : 1) }
    ' "$debian" || fail "Debian control does not recommend $font"
done

cargo_deb="$root/packaging/debian/cargo-deb.toml.fragment"
if grep -E '^depends *=.*(fonts-symbola|fonts-ibm-plex)' "$cargo_deb" >/dev/null; then
    fail 'cargo-deb metadata makes a font a hard dependency'
fi
grep -E '^recommends *=.*fonts-symbola' "$cargo_deb" >/dev/null \
    || fail 'cargo-deb metadata does not recommend Symbola'
grep -E '^recommends *=.*fonts-ibm-plex' "$cargo_deb" >/dev/null \
    || fail 'cargo-deb metadata does not recommend IBM Plex'

fedora="$root/packaging/fedora/helm.spec"
if grep -E '^Requires(\([^)]*\))?:.*(google-noto-sans-symbols2-fonts|nerd-font|ibm-plex-mono-fonts)' "$fedora" >/dev/null; then
    fail 'Fedora spec makes a font a hard dependency'
fi
grep -E '^Recommends:.*google-noto-sans-symbols2-fonts' "$fedora" >/dev/null \
    || fail 'Fedora spec does not recommend its symbol font'
grep -E '^Recommends:.*ibm-plex-mono-fonts' "$fedora" >/dev/null \
    || fail 'Fedora spec does not recommend IBM Plex'

nix="$root/packaging/nix/nixos-module.nix"
if grep -E 'nerd-fonts|nerdfonts' "$nix" >/dev/null; then
    fail 'NixOS module installs a Nerd Font unconditionally'
fi
grep -Fq 'pkgs.ibm-plex' "$nix" \
    || fail 'NixOS module does not select IBM Plex'

if find "$root" -type f \( -iname '*symbola*.ttf' -o -iname '*symbola*.otf' -o -iname '*nerd*.ttf' -o -iname '*nerd*.otf' \) -print -quit | grep -q .; then
    fail 'package inputs contain a prohibited Symbola or Nerd Font artifact'
fi

install="$root/docs/INSTALL.md"
grep -Fq 'does not install a Symbola or Nerd Font' "$install" \
    || fail 'installation guidance does not state the optional symbol-font policy'

echo 'font package policy: pass'
