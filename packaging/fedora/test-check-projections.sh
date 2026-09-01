#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
repo_root=$(CDPATH='' cd "$script_dir/../.." && pwd)
guard="$script_dir/check-projections.sh"

# RED is deliberate: the production guard is written only after this harness
# has failed because that guard is absent.
if [ ! -x "$guard" ]; then
    echo "FAIL: missing executable Fedora projection guard: $guard" >&2
    exit 1
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/helm-fedora-projections-test.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

canonical_image='registry.fedoraproject.org/fedora:44@sha256:df52038ff64ee61affa188d78beb85cf6eecfe4e9f6042238269ccdc8e944392'
tests_run=0

# This is the complete live-projection input set from SPEC 0009 section 5.
# Adding a support-facing Fedora projection requires an accepted contract and
# an explicit addition here and in the production guard.
current_inputs='
.github/workflows/distro.yml
packaging/fedora/helm.spec
README.md
docs/INSTALL.md
docs/ARCHITECTURE.md
docs/MVP.md
docs/ROADMAP.md
docs/integration/session-services.md
.github/ISSUE_TEMPLATE/bug.yml
docs/specs/0006-helm-ctl.md
packaging/fedora/baseline.toml'

# These are the only files containing accepted negative fixtures, superseded
# decisions, or third-party Fedora-41 facts. They are inputs to exact-line
# reconciliation, never filename- or directory-level exclusions.
historical_inputs='
docs/adr/0010-nix-flake-as-reference-build.md
docs/adr/0013-river-window-management-backend.md
docs/adr/0015-fedora-44-pre-alpha-baseline.md
docs/specs/0009-fedora-44-pre-alpha-baseline.md
.claude/memory/10-decisions.md
packaging/fedora/test-check-baseline.sh
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md
docs/integration/hardware-media.md'

copy_input() {
    destination=$1
    path=$2

    if [ ! -f "$repo_root/$path" ]; then
        echo "FAIL: projection fixture input is missing: $path" >&2
        exit 1
    fi
    mkdir -p "$destination/$(dirname "$path")"
    cp "$repo_root/$path" "$destination/$path"
}

make_canonical_fixture() {
    destination=$1
    mkdir -p "$destination"
    for path in $current_inputs $historical_inputs; do
        copy_input "$destination" "$path"
    done
}

# Pin every exceptional matching line independently of the guard. This checks
# the test fixture itself; the production guard must carry and enforce its own
# identical path+complete-line allowlist. A changed line is therefore not
# accepted merely because it remains in an historical filename.
assert_exact_historical_fixture() {
    fixture_root=$1

    while IFS= read -r entry || [ -n "$entry" ]; do
        [ -n "$entry" ] || continue
        path=${entry%%:::*}
        exact_line=${entry#*:::}
        count=$(grep -F -x -c -e "$exact_line" "$fixture_root/$path" || true)
        if [ "$count" -ne 1 ]; then
            printf 'FAIL: historical fixture line must occur exactly once\npath: %s\nline: %s\ncount: %s\n' \
                "$path" "$exact_line" "$count" >&2
            exit 1
        fi
    done <<'EOF'
docs/adr/0010-nix-flake-as-reference-build.md:::- **Supersedes / Superseded by:** Fedora 41-or-later baseline clauses and the
docs/adr/0010-nix-flake-as-reference-build.md:::NixOS/Nix, Ubuntu 24.04 LTS and later, Fedora 41 and later. All three are tested
docs/adr/0010-nix-flake-as-reference-build.md:::4. **CI builds and installs all three once each package path is buildable.** A
docs/adr/0010-nix-flake-as-reference-build.md:::7. **A pinned river 0.4.x is vendored**, per
docs/adr/0010-nix-flake-as-reference-build.md:::   requirements but do not yet build or bundle a verified `helm-river` package.
docs/adr/0013-river-window-management-backend.md:::4. Packaging vendors a pinned river 0.4.x. Ubuntu 24.04 and Fedora 41 ship
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::- **Supersedes / Superseded by:** Supersedes ADR 0010's Fedora 41-or-later baseline and Fedora portion of decision 7; supersedes ADR 0013 decision 4 only for Fedora
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::ADR 0010 selected one Fedora target but named Fedora 41 and later. ADR 0013
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::then assumed Fedora 41 provided only River 0.3.x or river-classic and required a
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::vendored River 0.4.x on every target. That was an explicit assumption to verify,
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::Fedora's authoritative lifecycle data now marks Fedora 41 and Fedora 42
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::archived. On 2026-08-29 it marked Fedora 43 current through 2026-12-02 and
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::River 0.3.14 for Fedora 43 and `river-0.4.8-1.fc44` for Fedora 44. River 0.4.0
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::   baseline. Fedora 41 is removed from required CI and current support claims.
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::2. Fedora 43 is not added. Supporting it would expand the one-Fedora-target
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::3. `44+`, `latest`, Rawhide, and implicit future-release support are rejected.
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::| **Fedora 43 and Fedora 44** | Covers both Fedora releases that were current on the decision date | Doubles the Fedora baseline paths; F43 provides River 0.3.14 and reaches EOL on 2026-12-02 |
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::| **Fedora 43 only** | It was still current and could be called the oldest supported Fedora | Preserves the unimplemented vendored-River burden and forces another baseline change after only 95 days |
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::| **Keep Fedora 41 or create a frozen F41 fixture** | Minimizes visible edits or preserves old compatibility investigations | F41 is EOL; a truthful offline fixture would require retained images, repository metadata, RPMs, and toolchains. No named regression question justifies that scope |
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::| **Use `latest`, Rawhide, or `44+`** | Avoids future release-number edits and may reveal future breakage early | Silently makes unadmitted releases support targets and makes CI advance without a reviewed product decision |
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::  Fedora-specific `helm-river` path.
docs/adr/0015-fedora-44-pre-alpha-baseline.md:::  for F41, F42, F43, `44+`, `latest`, Rawhide, and implicit-future wording, and
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::Helm currently names Fedora 41 as a first-class target and runs its only
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::Fedora smoke job in a floating Fedora 41 container. Fedora 41 is archived and
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- retiring Fedora 41 from required CI and current Helm support claims;
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- declining Fedora 43 and any implicit future-release support;
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- an offline Fedora 41 fixture, unless a later issue names a concrete
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::   `Fedora 44+`, `Fedora 44 and later`, `latest`, Rawhide, or an unbounded
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::3. Fedora 41 and Fedora 42 are EOL and are not current targets. Fedora 41 must
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::   not remain in a required job or live Helm support instruction. No Fedora 41
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::4. Fedora 43 is not a Helm target. Although Fedora still marked it current on
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::ADR 0013; Fedora 43's 0.3.14 does not provide it. Therefore Fedora 44 package
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::unimplemented `helm-river` alternative.
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::ADR 0015 supersedes ADR 0010's Fedora 41-or-later baseline clauses as well as
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::example in SPEC 0006 must not present Fedora 41 as current.
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::third-party historical facts, such as a package being available in Fedora 41,
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::1. ADR 0010's header/index must mark its Fedora 41-or-later baseline and the
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::The correction must not describe a direct Fedora 41 to Fedora 44 operating
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A1 | Given the machine-checked inventory of live Fedora claims, when `status = "pre-alpha"` it is validated, then the only admitted release is exactly `44`; when `status = "unsupported"`, no Fedora release is admitted; `41`, `42`, `43`, `44+`, `latest`, Rawhide, and implicit newer releases are always rejected as current Helm targets | *Planned (#138):* `fedora_baseline::only_fedora_44_is_a_live_target`; includes one failing fixture per rejected form and one unsupported-state fixture |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A2 | Given the required distro workflow, when `status = "pre-alpha"`, then it contains exactly one `fedora-44-cargo-smoke` Cargo lane and exactly one `fedora-rpm-package` retained-source RPM build lane, each using the exact official Fedora 44 digest above; no other Fedora lane is present. The RPM lane builds but does not clean-install the resulting package, and neither lane is graphical-session or SELinux evidence. When `status = "unsupported"`, it contains no required Fedora lane; neither state adds a Fedora 41/Fedora 43 lane, runner, or architecture claim | `packaging/fedora/test-check-projections.sh`: `fedora_baseline::required_ci_uses_one_pinned_f44_cargo_smoke_and_one_retained_source_rpm_build` |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A3 | Given the Fedora RPM metadata, when it is inspected, then it identifies Fedora 44, requires Fedora's `river >= 0.4.0`, contains no `helm-river` alternative or false “River unavailable on Fedora” claim, and continues to state that the package is pre-alpha and not a working desktop | *Planned (#138):* `fedora_baseline::rpm_metadata_matches_the_f44_pre_alpha_contract` |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A6 | Given a seeded stale live-support claim such as `Fedora 41+`, when the consistency guard runs, then it fails; given an exact reviewed historical exception in a superseded ADR or third-party history, then it passes without treating that text as current support | *Planned (#138):* `fedora_baseline::stale_live_claims_fail_and_exact_history_exceptions_pass` |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A8 | Given current user-facing and normative documentation, when the consistency guard and doc review run, then Fedora 41 is absent from live Helm support/install examples, Fedora 44 is described as the sole pre-alpha baseline, no text upgrades the Cargo smoke to RPM/session evidence, and no direct Fedora 41 to Fedora 44 OS upgrade is called supported | *Planned (#138):* `fedora_baseline::docs_state_the_evidence_level_truthfully` plus review of rendered Markdown |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A9 | Given the completed #138 diff, when scope is reviewed, then it contains no Yazi/Starship source decision, package publishing/signing infrastructure, extra Fedora lane, KVM/native-runner acquisition, scheduled network job, Fedora 41 fixture, generation rollback design, or `flake.lock` strategy | Required reviewer checklist on the #138 pull request |
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- Fedora Bodhi: F41 `archived`, EOL 2025-12-15; F42 `archived`, EOL
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::  2026-05-27; F43 `current`, EOL 2026-12-02; F44 `current`, EOL 2027-06-02.
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- Fedora package metadata: F43 `river` 0.3.14; F44
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::- Repository state: the Fedora job was a Cargo-only `fedora:41` container;
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A later Fedora release becomes supported through `+`, `latest`, or Rawhide | A1 |
.claude/memory/10-decisions.md:::~~**2026-08-26 — Vendor a pinned river rather than depending on the distro's** —
.claude/memory/10-decisions.md:::because Ubuntu 24.04 and Fedora 41 ship river 0.3.x or river-classic, neither of
.claude/memory/10-decisions.md:::three first-class targets. Puts Zig in the packaging pipeline only — *revisit if*
.claude/memory/10-decisions.md:::~~**2026-08-26 — Packaging vendors river 0.4.8 specifically** — because that is
.claude/memory/10-decisions.md:::vendoring be dropped entirely.~~ **Partially superseded for Fedora on
packaging/fedora/test-check-baseline.sh:::expect_fail_message fedora-41 2027-06-01 'admits only Fedora release 44' \
packaging/fedora/test-check-baseline.sh:::    "$canonical_image" 'track = "latest"'
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::**Goal:** Replace stale Fedora 41 claims with a tested, offline-validated Fedora 44 pre-alpha baseline.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- Fedora 44 only; no F43, `44+`, `latest`, Rawhide, extra lane, runner or architecture claim.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- [ ] Verify every referenced document/path exists and docs contain no live F41 claim outside exact historical exceptions.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- [ ] Write fixtures that fail for EOL equality, post-EOL, an unknown field, and a pre-alpha F41 record.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- [ ] Add a failing consistency inventory for F41/F43/unbounded-target claims and false evidence-level wording.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- [ ] Use Fedora `river >= 0.4.0`; remove the `helm-river` alternative and false unavailable statement without imposing `<0.5`.
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md:::- [ ] Reconcile current support documents, retaining only enumerated historical F41 references.
docs/integration/hardware-media.md:::| [`wiremix`](https://crates.io/crates/wiremix) | Rust, ratatui | v0.11.0, 2026-06-05 | TUI mixer, an ncpamixer clone. Packaged in Arch, Fedora 41–43 and nixpkgs; **not in Debian or Ubuntu** (verified via repology) |
docs/integration/hardware-media.md:::| Power profiles | [power-profiles-daemon](https://repology.org/project/power-profiles-daemon/versions) 0.21–0.30, or [tuned-ppd](https://fedoraproject.org/wiki/Changes/TunedAsTheDefaultPowerProfileManagementDaemon) | same D-Bus API — tuned-ppd is explicitly a drop-in translation layer, which is why Fedora could switch defaults from ppd to tuned in F41 without desktops changing code | Integrate against the API, not the implementation, and both distros are covered |
EOF
}

clone_case() {
    name=$1
    case_root="$tmp_dir/$name"
    cp -R "$tmp_dir/canonical" "$case_root"
    printf '%s\n' "$case_root"
}

append_line() {
    path=$1
    line=$2
    printf '%s\n' "$line" >>"$path"
}

replace_once() {
    path=$1
    old=$2
    new=$3
    output="$path.replaced"

    awk -v old="$old" -v new="$new" '
        BEGIN { replacements = 0 }
        {
            position = index($0, old)
            if (position > 0 && replacements == 0) {
                $0 = substr($0, 1, position - 1) new substr($0, position + length(old))
                replacements++
            }
            print
        }
        END {
            if (replacements != 1) {
                exit 42
            }
        }
    ' "$path" >"$output" || {
        rm -f "$output"
        printf 'FAIL: fixture mutation did not replace exactly one occurrence\npath: %s\nold: %s\n' \
            "$path" "$old" >&2
        exit 1
    }
    mv "$output" "$path"
}

replace_all() {
    replacement_path=$1
    replacement_old=$2
    replacement_new=$3
    replacement_output="$replacement_path.replaced"

    awk -v old="$replacement_old" -v new="$replacement_new" '{ gsub(old, new); print }' \
        "$replacement_path" >"$replacement_output"
    mv "$replacement_output" "$replacement_path"
}

remove_all_containing() {
    path=$1
    line=$2
    output="$path.removed"

    awk -v line="$line" 'index($0, line) == 0 { print }' "$path" >"$output"
    mv "$output" "$path"
}

comment_all_containing() {
    path=$1
    line=$2
    output="$path.commented"

    awk -v line="$line" 'index($0, line) { print "# " $0; next } { print }' \
        "$path" >"$output"
    mv "$output" "$path"
}

write_file() {
    path=$1
    content=$2

    mkdir -p "$(dirname "$path")"
    printf '%s\n' "$content" >"$path"
}

run_guard() {
    fixture_root=$1
    "$guard" --root "$fixture_root" 2>&1
}

expect_pass() {
    name=$1
    fixture_root=$2
    tests_run=$((tests_run + 1))

    if ! output=$(run_guard "$fixture_root"); then
        printf 'FAIL: %s unexpectedly failed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

expect_fail_message() {
    name=$1
    fixture_root=$2
    expected=$3
    tests_run=$((tests_run + 1))

    if output=$(run_guard "$fixture_root"); then
        printf 'FAIL: %s unexpectedly passed\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    case "$output" in
        *"$expected"*) ;;
        *)
            printf 'FAIL: %s failed for the wrong reason\nexpected: %s\nactual: %s\n' \
                "$name" "$expected" "$output" >&2
            exit 1
            ;;
    esac
    printf 'ok %d - %s\n' "$tests_run" "$name"
}

make_canonical_fixture "$tmp_dir/canonical"
assert_exact_historical_fixture "$tmp_dir/canonical"

# SPEC 0009 A2: the retained-source RPM build is an independently named second
# Fedora lane. Renaming its job must not make a different Fedora lane equivalent.
case_root=$(clone_case misnamed-retained-source-rpm-lane)
replace_once "$case_root/.github/workflows/distro.yml" '  fedora-rpm-package:' \
    '  fedora-rpm-build:'
expect_fail_message misnamed-retained-source-rpm-lane "$case_root" \
    'exactly one Fedora 44 Cargo-smoke lane and one retained-source RPM build lane are required'

expect_pass canonical-projections "$tmp_dir/canonical"

case_root=$(clone_case current-f41-claim)
append_line "$case_root/README.md" 'Fedora 41 is a current Helm target.'
expect_fail_message current-f41-claim "$case_root" 'forbidden Fedora 41 current claim'

case_root=$(clone_case fedora-43-lane)
append_line "$case_root/.github/workflows/distro.yml" '          - name: fedora-43-cargo-smoke'
append_line "$case_root/.github/workflows/distro.yml" '            runs-on: ubuntu-24.04'
append_line "$case_root/.github/workflows/distro.yml" '            container: registry.fedoraproject.org/fedora:43'
expect_fail_message fedora-43-lane "$case_root" 'exactly one Fedora 44 Cargo-smoke lane'

case_root=$(clone_case unbounded-fedora-44)
append_line "$case_root/docs/INSTALL.md" 'Fedora 44+ is supported.'
expect_fail_message unbounded-fedora-44 "$case_root" 'implicit or floating Fedora target'

case_root=$(clone_case floating-fedora-image)
replace_once "$case_root/.github/workflows/distro.yml" "$canonical_image" 'registry.fedoraproject.org/fedora:44'
expect_fail_message floating-fedora-image "$case_root" 'workflow image must equal baseline.toml'

case_root=$(clone_case wrong-fedora-digest)
replace_once "$case_root/.github/workflows/distro.yml" "$canonical_image" \
    'registry.fedoraproject.org/fedora:44@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
expect_fail_message wrong-fedora-digest "$case_root" 'workflow image must equal baseline.toml'

case_root=$(clone_case second-fedora-lane)
append_line "$case_root/.github/workflows/distro.yml" '          - name: fedora-44-cargo-smoke-duplicate'
append_line "$case_root/.github/workflows/distro.yml" '            runs-on: ubuntu-24.04'
append_line "$case_root/.github/workflows/distro.yml" "            container: $canonical_image"
expect_fail_message second-fedora-lane "$case_root" 'exactly one Fedora 44 Cargo-smoke lane'

# An additional Fedora-family image is a third Fedora lane even when it does
# not use the canonical base-image repository name.
case_root=$(clone_case extra-fedora-minimal-lane)
append_line "$case_root/.github/workflows/distro.yml" '          - name: fedora-44-minimal-smoke'
append_line "$case_root/.github/workflows/distro.yml" '            runs-on: ubuntu-24.04'
append_line "$case_root/.github/workflows/distro.yml" '            container: registry.fedoraproject.org/fedora-minimal:44'
expect_fail_message extra-fedora-minimal-lane "$case_root" \
    'exactly one Fedora 44 Cargo-smoke lane and one retained-source RPM build lane are required'

# SPEC 0009 A1/A2: retirement is stateful. Renaming the lane must not let an
# unsupported record retain either a Fedora container or live F44 support text.
case_root=$(clone_case unsupported-renamed-lane-and-current-claim)
replace_once "$case_root/packaging/fedora/baseline.toml" 'status = "pre-alpha"' \
    'status = "unsupported"'
replace_once "$case_root/.github/workflows/distro.yml" 'fedora-44-cargo-smoke' \
    'retired-44-cargo-smoke'
expect_fail_message unsupported-renamed-lane-and-current-claim "$case_root" \
    'unsupported Fedora status requires no Fedora lane or current support claim'

# Discovery must also reject a fresh support document which is not named in
# current_inputs. First remove canonical current projections so this fixture
# isolates the otherwise-unlisted document.
case_root=$(clone_case unsupported-unlisted-f44-current-claim)
replace_once "$case_root/packaging/fedora/baseline.toml" 'status = "pre-alpha"' \
    'status = "unsupported"'
for path in $current_inputs; do
    [ "$path" = packaging/fedora/baseline.toml ] && continue
    replace_all "$case_root/$path" 'Fedora 44' 'retired Fedora'
    replace_all "$case_root/$path" 'F44' 'retired'
    replace_all "$case_root/$path" 'fedora-44' 'retired-fedora'
    replace_all "$case_root/$path" 'fedora:44' 'retired-fedora'
done
write_file "$case_root/docs/fedora-44-support.md" 'Fedora 44 is a current Helm support target.'
expect_fail_message unsupported-unlisted-f44-current-claim "$case_root" \
    'unsupported Fedora status requires no Fedora lane or current support claim'

case_root=$(clone_case unsupported-fully-retired)
replace_once "$case_root/packaging/fedora/baseline.toml" 'status = "pre-alpha"' \
    'status = "unsupported"'
for path in $current_inputs; do
    [ "$path" = packaging/fedora/baseline.toml ] && continue
    replace_all "$case_root/$path" 'Fedora 44' 'retired Fedora'
    replace_all "$case_root/$path" 'F44' 'retired'
    replace_all "$case_root/$path" 'fedora-44' 'retired-fedora'
    replace_all "$case_root/$path" 'fedora:44' 'retired-fedora'
done
expect_pass unsupported-fully-retired "$case_root"

case_root=$(clone_case unsupported-unlisted-f44-identity)
replace_once "$case_root/packaging/fedora/baseline.toml" 'status = "pre-alpha"' \
    'status = "unsupported"'
for path in $current_inputs; do
    [ "$path" = packaging/fedora/baseline.toml ] && continue
    replace_all "$case_root/$path" 'Fedora 44' 'retired Fedora'
    replace_all "$case_root/$path" 'F44' 'retired'
    replace_all "$case_root/$path" 'fedora-44' 'retired-fedora'
    replace_all "$case_root/$path" 'fedora:44' 'retired-fedora'
done
write_file "$case_root/docs/fedora-44-platform.md" "Fedora 44 is Helm's Fedora platform."
expect_fail_message unsupported-unlisted-f44-identity "$case_root" \
    'unsupported Fedora status requires no Fedora lane or current support claim'

case_root=$(clone_case helm-river-rpm-alternative)
replace_once "$case_root/packaging/fedora/helm.spec" 'Requires:       river >= 0.4.0' \
    'Requires:       river >= 0.4.0
Requires:       (river >= 0.4.0 or helm-river >= 0.4.0)'
expect_fail_message helm-river-rpm-alternative "$case_root" \
    'RPM must contain only the canonical active river dependency'

# SPEC 0009 A3: the active dependency form is exactly the protocol floor; an
# invented ceiling or another River dependency is not an equivalent projection.
case_root=$(clone_case rpm-river-ceiling)
replace_once "$case_root/packaging/fedora/helm.spec" 'Requires:       river >= 0.4.0' \
    'Requires:       river >= 0.4.0
Requires:       river < 0.5.0'
expect_fail_message rpm-river-ceiling "$case_root" \
    'RPM must contain only the canonical active river dependency'

case_root=$(clone_case rpm-second-river-requirement)
replace_once "$case_root/packaging/fedora/helm.spec" 'Requires:       river >= 0.4.0' \
    'Requires:       river >= 0.4.0
Requires:       river >= 0.4.1'
expect_fail_message rpm-second-river-requirement "$case_root" \
    'RPM must contain only the canonical active river dependency'

case_root=$(clone_case rpm-river-classic-requirement)
replace_once "$case_root/packaging/fedora/helm.spec" 'Requires:       river >= 0.4.0' \
    'Requires:       river >= 0.4.0
Requires:       river-classic'
expect_fail_message rpm-river-classic-requirement "$case_root" \
    'RPM must contain only the canonical active river dependency'

case_root=$(clone_case universal-fedora-river-vendoring)
append_line "$case_root/docs/ARCHITECTURE.md" 'Helm vendors River in every target package, including Fedora.'
expect_fail_message universal-fedora-river-vendoring "$case_root" 'universal River sourcing claim'

# The platform table must distinguish intended delivery targets from the
# narrower evidence now available for Fedora's pre-alpha Cargo smoke.
case_root=$(clone_case architecture-overclaims-ci-evidence)
replace_once "$case_root/docs/ARCHITECTURE.md" 'Target plan and current evidence:' \
    'Supported from day one, tested in CI:'
expect_fail_message architecture-overclaims-ci-evidence "$case_root" \
    'ARCHITECTURE must not claim every target is already supported and tested in CI'

case_root=$(clone_case false-fedora-runtime-evidence)
append_line "$case_root/README.md" 'The Fedora 44 lane proves the RPM and graphical session work under SELinux.'
expect_fail_message false-fedora-runtime-evidence "$case_root" 'Fedora evidence exceeds Cargo smoke'

case_root=$(clone_case missing-rpm-pre-alpha-boundary)
replace_once "$case_root/packaging/fedora/helm.spec" \
    'THIS PACKAGE IS PRE-ALPHA AND DOES NOT INSTALL A WORKING DESKTOP.' \
    'THIS PACKAGE INSTALLS A WORKING DESKTOP.'
expect_fail_message missing-rpm-pre-alpha-boundary "$case_root" 'RPM must retain the pre-alpha no-working-desktop boundary'

case_root=$(clone_case unlisted-historical-f41-line)
append_line "$case_root/docs/integration/hardware-media.md" 'Historical note: Fedora 41 carried example-package.'
expect_fail_message unlisted-historical-f41-line "$case_root" 'unreviewed historical Fedora claim'

# A support projection cannot escape review merely by being placed in a new
# document outside the former hand-maintained current-input inventory.
case_root=$(clone_case unlisted-current-support-document)
write_file "$case_root/docs/fedora-support.md" 'Fedora 41 is a current Helm support target.'
expect_fail_message unlisted-current-support-document "$case_root" \
    'forbidden Fedora 41 current claim'

# The normal PR/push workflow must make each local policy check load-bearing.
# Removing all occurrences exercises the same state whether the source workflow
# has not yet gained the step (RED) or a future edit deletes it (regression).
# shellcheck disable=SC2016 # The workflow must contain this literal command.
for invocation in \
    './packaging/fedora/check-baseline.sh --date "$(date -u +%F)"' \
    './packaging/fedora/check-projections.sh' \
    './packaging/fedora/test-check-baseline.sh' \
    './packaging/fedora/test-check-projections.sh'; do
    # shellcheck disable=SC2016 # The character class is deliberately literal.
    case_root=$(clone_case "missing-$(printf '%s' "$invocation" | tr '/ $()"+' '-')")
    remove_all_containing "$case_root/.github/workflows/distro.yml" "$invocation"
    expect_fail_message "missing-$invocation" "$case_root" \
        "normal PR/push CI must invoke $invocation"
done

case_root=$(clone_case commented-ci-invocations)
# shellcheck disable=SC2016 # These are literal workflow commands.
for invocation in \
    './packaging/fedora/check-baseline.sh --date "$(date -u +%F)"' \
    './packaging/fedora/check-projections.sh' \
    './packaging/fedora/test-check-baseline.sh' \
    './packaging/fedora/test-check-projections.sh'; do
    comment_all_containing "$case_root/.github/workflows/distro.yml" "$invocation"
done
# shellcheck disable=SC2016 # This is a literal guard diagnostic.
expect_fail_message commented-ci-invocations "$case_root" \
    'normal PR/push CI must invoke ./packaging/fedora/check-baseline.sh --date "$(date -u +%F)"'

case_root=$(clone_case changed-allowed-history-line)
replace_once "$case_root/docs/integration/hardware-media.md" \
    'Packaged in Arch, Fedora 41–43 and nixpkgs' \
    'Packaged in Arch, Fedora 41–44 and nixpkgs'
expect_fail_message changed-allowed-history-line "$case_root" 'historical exception changed'

printf 'PASS: %d Fedora projection guard fixtures\n' "$tests_run"
