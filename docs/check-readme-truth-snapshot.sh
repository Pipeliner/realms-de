#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
default_root=$(CDPATH='' cd "$script_dir/.." && pwd)
root=$default_root

if [ "$#" -gt 0 ]; then
    if [ "$#" -ne 2 ] || [ "$1" != "--root" ]; then
        echo "usage: $0 [--root PATH]" >&2
        exit 2
    fi
    root=$2
fi

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

readme="$root/README.md"
workflow="$root/.github/workflows/ci.yml"
roadmap="$root/docs/ROADMAP.md"
[ -f "$readme" ] || fail 'README is required'
[ -f "$workflow" ] || fail 'documentation workflow is required'
[ -f "$roadmap" ] || fail 'roadmap is required'

require_section() {
    section=$1
    text=$2
    message=$3
    printf '%s\n' "$section" | grep -F -q -e "$text" || fail "$message"
}

markdown_tick=$(printf '\140')

intro_section=$(sed -n '1,/^---$/p' "$readme")
rules_section=$(sed -n '/^## What makes it different$/,/^## The one idea$/p' "$readme")
status_section=$(sed -n '/^## Status$/,/^## Try it$/p' "$readme")
map_section=$(sed -n '/^## Repo map$/,/^## Docs$/p' "$readme")

require_section "$intro_section" 'keyboard-first, gapless-tiling, Rust-first Wayland desktop environment' \
    'README first screen must state the full project identity'
require_section "$intro_section" 'zero animations, one palette file' \
    'README first screen must state the motion and palette constraints'
require_section "$intro_section" 'docs/assets/realm-tiled-desktop.png' \
    'README first screen must show the tiled live-VM capture'
require_section "$intro_section" 'docs/assets/realm-grimoire.png' \
    'README first screen must show the grimoire live-VM capture'
require_section "$intro_section" 'NixOS QEMU VM' \
    'README capture caption must identify the NixOS QEMU VM'
require_section "$intro_section" 'River 0.4.8' \
    'README capture caption must identify River 0.4.8'
require_section "$intro_section" '1280×800' \
    'README capture caption must report PNG-IHDR-derived 1280x800 dimensions'
require_section "$intro_section" 'docs/assets/capture-provenance.json' \
    'README capture caption must link the original provenance'
require_section "$intro_section" 'docs/assets/capture-review.md' \
    'README capture caption must link the visual review correction'
require_section "$intro_section" 'not physical-hardware or final-theme evidence' \
    'README capture caption must retain the hardware and final-theme boundary'
require_section "$rules_section" '**The ledger is the truth.**' \
    'README must use the exact ledger rule'
require_section "$rules_section" "**No colour outside \`palette.toml\`.**" \
    'README must use the exact palette rule'
require_section "$rules_section" '**Snappy is a number.**' \
    'README must use the exact frame-budget rule'
require_section "$rules_section" 'docs/adr/0001-ledger-as-single-source-of-truth.md' \
    'README must link ADR 0001'
require_section "$rules_section" 'docs/adr/0005-palette-toml-single-source.md' \
    'README must link ADR 0005'
require_section "$rules_section" 'docs/adr/0009-no-animation-budget.md' \
    'README must link ADR 0009'
require_section "$rules_section" 'realm is the *window manager* for river' \
    "README must state that realm is river's window manager"
require_section "$status_section" 'Milestone M0 is in progress' \
    'README status must state that M0 is in progress'
require_section "$status_section" '**M3 is the MVP.**' \
    'README status must identify M3 as the MVP'

if ! grep -E -q '^\| \[M0\].*\| \*\*in progress\*\* \|$' "$roadmap"; then
    fail 'roadmap must mark M0 in progress'
fi
if ! grep -F -q -e '**[M3](#m3--daily-drivable) is the MVP.**' "$roadmap"; then
    fail 'roadmap must identify M3 as the MVP'
fi

if grep -F -q -e 'NiriBackend' "$roadmap"; then
    fail 'roadmap must not refer to NiriBackend'
fi

for path in \
    flake.nix \
    crates/realm-theme/Cargo.toml \
    crates/realm-theme/src/lib.rs \
    crates/realm-theme/src/theme.rs \
    crates/realm-ctl/Cargo.toml \
    crates/realm-ctl/src/main.rs \
    crates/realm-session/Cargo.toml \
    crates/realm-session/src/lib.rs \
    crates/realm-session/src/backend.rs \
    crates/realm-session/src/runtime.rs \
    crates/realm-session/src/bin/realm-wm.rs \
    crates/realm-bar/Cargo.toml \
    crates/realm-bar/src/main.rs \
    crates/realm-bar/tests/render_contract.rs \
    packaging/session/realm.desktop \
    packaging/session/realm-session \
    packaging/systemd/realm-session.target \
    configs/portal/realm-portals.conf \
    packaging/nix/nixos-module.nix \
    packaging/debian/control \
    packaging/fedora/realm.spec \
    docs/assets/realm-tiled-desktop.png \
    docs/assets/realm-grimoire.png \
    docs/assets/capture-provenance.json \
    docs/assets/capture-review.md \
    docs/assets/control-tiled-state.json \
    docs/assets/control-grimoire-state.json; do
    [ -e "$root/$path" ] || fail "README truth snapshot artifact is missing: $path"
done

sha256_of() {
    sha256sum "$1" | awk '{ print $1 }'
}

tiled_sha=7a538588a797829871e7e55d8e397eabf7232b979928cadf0a047d30085de695
grimoire_sha=60777f7287f64063805a70ddd4d82d251f86021075c374deb3115808705132b0
provenance_sha=0bba679bed1eb33c3e64e1fb805f51afef002ca69f0bca6111e7dd0bed62fa30

[ "$(sha256_of "$root/docs/assets/realm-tiled-desktop.png")" = "$tiled_sha" ] \
    || fail 'README tiled capture SHA-256 differs from preserved provenance'
[ "$(sha256_of "$root/docs/assets/realm-grimoire.png")" = "$grimoire_sha" ] \
    || fail 'README grimoire capture SHA-256 differs from preserved provenance'
[ "$(sha256_of "$root/docs/assets/capture-provenance.json")" = "$provenance_sha" ] \
    || fail 'README original capture provenance must remain byte-for-byte preserved'

capture_provenance=$(cat "$root/docs/assets/capture-provenance.json")
capture_review=$(cat "$root/docs/assets/capture-review.md")
tiled_state=$(cat "$root/docs/assets/control-tiled-state.json")
grimoire_state=$(cat "$root/docs/assets/control-grimoire-state.json")

require_section "$capture_provenance" "$tiled_sha" \
    'README original provenance must bind the tiled capture hash'
require_section "$capture_provenance" "$grimoire_sha" \
    'README original provenance must bind the grimoire capture hash'
require_section "$capture_provenance" '20ef533c6989a5712598b245b937d5d87fa607ec' \
    'README original provenance must identify the tested merge revision'
require_section "$capture_review" 'Both actual PNGs are 1280×800' \
    'README capture review must record PNG-IHDR-derived dimensions'
require_section "$capture_review" "incorrect ${markdown_tick}1920x1080${markdown_tick} environment suffix" \
    'README capture review must correct the preserved requested-resolution metadata'
require_section "$tiled_state" '"windows":3' \
    'README tiled capture state must retain three managed windows'
require_section "$tiled_state" '"whichkey":true' \
    'README tiled capture state must retain visible which-key'
require_section "$tiled_state" '"grimoire":false' \
    'README tiled capture state must precede grimoire'
require_section "$grimoire_state" '"windows":3' \
    'README grimoire capture state must retain three managed windows'
require_section "$grimoire_state" '"whichkey":true' \
    'README grimoire capture state must retain visible which-key'
require_section "$grimoire_state" '"grimoire":true' \
    'README grimoire capture state must retain visible grimoire'

grep -F -q -e '#[test]' "$root/crates/realm-theme/src/theme.rs" \
    || fail 'README truth snapshot realm-theme must retain test evidence'
grep -F -q -e 'name = "realmctl"' "$root/crates/realm-ctl/Cargo.toml" \
    || fail 'README truth snapshot realmctl implementation is missing'
grep -F -q -e 'ThemeCommand' "$root/crates/realm-ctl/src/main.rs" \
    || fail 'README truth snapshot realmctl implementation is missing'
grep -F -q -e 'pub trait WmBackend' "$root/crates/realm-session/src/backend.rs" \
    || fail 'README truth snapshot realm-session seam is missing'

require_section "$status_section" "${markdown_tick}realmctl theme${markdown_tick}" \
    'README status must name the implemented realmctl theme surface'
require_section "$status_section" 'Daemon and real River adapter verified in the installed NixOS QEMU VM' \
    'README must name installed NixOS VM verification for realm-wm'

require_section "$status_section" 'Implemented and verified in the installed NixOS QEMU VM' \
    'README must name installed NixOS VM verification for realm-bar'
require_section "$status_section" 'Physical-hardware, native-package installation, and functional portal verification remain pending' \
    'README status must retain the unverified delivery boundary'
for stale_claim in \
    'There is no desktop environment here yet' \
    'live compositor verification pending' \
    'There are no screenshots of realm' \
    'realm does not run yet'; do
    if printf '%s\n' "$status_section" | grep -F -q -e "$stale_claim"; then
        fail "README status retains stale pre-live claim: $stale_claim"
    fi
done
while IFS='|' read -r path map_name; do
    [ -n "$path" ] || continue
    [ -e "$root/$path" ] || fail "README repository-map path is missing: $path"
    require_section "$map_section" "$map_name" \
        "README repository map must name $path"
done <<'EOF'
crates/|├─ crates/
configs/|├─ configs/
configs/templates/|│  ├─ templates/
configs/portal/|│  └─ portal/
packaging/|├─ packaging/
packaging/nix/|│  ├─ nix/
packaging/debian/|│  ├─ debian/
packaging/fedora/|│  └─ fedora/
flake.nix|├─ flake.nix
docs/|├─ docs/
design/|├─ design/
.claude/|├─ .claude/
palette.toml|└─ palette.toml
EOF

if printf '%s\n' "$status_section" | grep -F -q -e 'Planned. Not started'; then
    fail 'README must not call tracked session assets planned/not started'
fi
if printf '%s\n' "$status_section" | grep -F -q -e 'There is nothing to install: no session entry'; then
    fail 'README must not deny the tracked session entry'
fi
require_section "$status_section" 'realm-theme' \
    'README status must identify the implemented realm-theme library'
require_section "$status_section" 'Tracked pre-alpha contract' \
    'README status must identify tracked pre-alpha delivery assets'
require_section "$map_section" 'root Nix reference-build entry point' \
    'README repository map must name the root flake'

needs_section=$(sed -n '/^## Needs a human$/,/^## Repo map$/p' "$readme")
[ -n "$needs_section" ] || fail 'README needs-human section is required'
require_section "$needs_section" '2026-08-30T06:18:36Z' \
    'README needs-human snapshot timestamp differs from the accepted snapshot'

expected_count=0
while IFS='|' read -r issue title; do
    [ -n "$issue" ] || continue
    expected_count=$((expected_count + 1))
    issue_url="https://github.com/Pipeliner/realms-de/issues/$issue"
    url_count=$(printf '%s\n' "$needs_section" | grep -F -c -e "$issue_url)" || true)
    [ "$url_count" -eq 1 ] || fail "missing needs-human snapshot issue #$issue"
    row_prefix="| [#$issue — $title]($issue_url) |"
    row_count=$(printf '%s\n' "$needs_section" | grep -F -c -e "$row_prefix" || true)
    [ "$row_count" -eq 1 ] || fail 'needs-human snapshot title differs from GitHub'
    row=$(printf '%s\n' "$needs_section" | grep -F -e "$row_prefix")
    printf '%s\n' "$row" | awk -F '|' 'NF == 4 && $3 ~ /[^[:space:]]/ { ok = 1 } END { exit(ok ? 0 : 1) }' \
        || fail 'needs-human snapshot blocker is empty'
done <<'EOF'
168|Reconcile generation GC with transferred lifecycle leases and M1/M2 launch sequencing
166|Specify JSON schemas for realmctl theme lint and diff
135|Complete exact M1 activation assets and supported-consumer probes
134|Decide supported M1 package sources and catalog migration for Yazi and Starship
133|Specify truthful desktop-entry and D-Bus activation for themed Qt launches
132|Reconcile activation launch lifecycle with session teardown and restart
35|Does the 𓂃 prompt sigil survive on the target distros' default fonts, or does `~` become the default?
30|Template: starship prompt for thoth, with the 𓂃 sigil and its ASCII fallback
25|Template: GTK 3, GTK 4 and libadwaita stylesheets
24|Extend the "no colour outside palette.toml" CI guard to templates and generated outputs
23|Add `realmctl theme apply`, `lint` and `diff`
17|Configure branch protection on the default branch with the CI checks as required
16|Enable Dependabot alerts and version updates, and create the labels its config references
EOF

if printf '%s\n' "$needs_section" \
    | grep -F -q -e 'https://github.com/Pipeliner/realms-de/issues/34'; then
    fail 'closed issue #34 must not appear in the needs-human snapshot'
fi

actual_count=$(printf '%s\n' "$needs_section" \
    | grep -E -c 'https://github\.com/Pipeliner/realms-de/issues/[0-9]+' || true)
[ "$actual_count" -eq "$expected_count" ] \
    || fail 'needs-human snapshot must contain exactly the accepted issue set'

docs_job=$(sed -n '/^  docs:/,/^  msrv:/p' "$workflow")
for command in \
    './docs/test-readme-truth-snapshot.sh' \
    './docs/check-readme-truth-snapshot.sh'; do
    if ! printf '%s\n' "$docs_job" | awk -v command="$command" '
        {
            line = $0
            sub(/^[[:space:]]*/, "", line)
            if (line == command) {
                found = 1
            }
        }
        END { exit(found ? 0 : 1) }
    '; then
        if [ "$command" = './docs/test-readme-truth-snapshot.sh' ]; then
            fail 'documentation CI must run the README truth snapshot fixtures'
        fi
        fail 'documentation CI must run the README truth snapshot check'
    fi
done

echo 'README truth snapshot: pass'
