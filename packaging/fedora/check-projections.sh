#!/bin/sh

set -eu

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
default_root=$(CDPATH='' cd "$script_dir/../.." && pwd)
root=$default_root

if [ "$#" -gt 0 ]; then
    if [ "$#" -ne 2 ] || [ "$1" != "--root" ]; then
        echo "usage: $0 [--root PATH]" >&2
        exit 2
    fi
    root=$2
fi

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

historical_inputs='
docs/adr/0010-nix-flake-as-reference-build.md
docs/adr/0013-river-window-management-backend.md
docs/adr/0015-fedora-44-pre-alpha-baseline.md
docs/specs/0009-fedora-44-pre-alpha-baseline.md
.claude/memory/10-decisions.md
packaging/fedora/test-check-baseline.sh
docs/superpowers/plans/2026-08-29-fedora-44-baseline.md
docs/integration/hardware-media.md'

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

for path in $current_inputs $historical_inputs; do
    [ -f "$root/$path" ] || fail "Fedora projection input is missing: $path"
done

assert_contains() {
    path=$1
    text=$2
    message=$3
    grep -F -q -e "$text" "$root/$path" || fail "$message"
}

count_fixed() {
    path=$1
    text=$2
    grep -F -c -e "$text" "$root/$path" || true
}

current_matches() {
    pattern=$1
    for path in $current_inputs; do
        if grep -E -i -q -e "$pattern" "$root/$path"; then
            return 0
        fi
    done
    return 1
}

historical_allowlist() {
    cat <<'EOF'
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
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A2 | Given the required distro matrix, when `status = "pre-alpha"`, then it contains one Fedora lane named as a Cargo smoke and uses the exact official Fedora 44 digest above; when `status = "unsupported"`, it contains no required Fedora lane; neither state adds a Fedora 41/Fedora 43 lane, runner, or architecture claim | *Planned (#138):* `fedora_baseline::required_ci_uses_one_pinned_f44_cargo_smoke` |
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
docs/integration/hardware-media.md:::| [NetworkManager](https://networkmanager.dev/blog/networkmanager-1-58/) | The general-purpose one: wifi, ethernet, WWAN, VPN plugins, per-connection DNS | 1.58.1 (Arch), 1.54.3 (Ubuntu 26.04), 1.54.0 (Fedora 43), 1.46 (Ubuntu 24.04). [1.58 released 2026-07-20](https://networkmanager.dev/blog/networkmanager-1-58/) | The only candidate with a real VPN story. Rich D-Bus API. Installed by default on all three targets |
docs/integration/hardware-media.md:::| [`kanshi`](https://repology.org/project/kanshi/versions) | C | 1.9.0 newest; Ubuntu 24.04 has 1.5.1, Fedora 43 has 1.8.0, Ubuntu 26.04 has 1.9.0 | Profile-based: match a set of connected outputs, apply a layout. Packaged on every target |
docs/integration/hardware-media.md:::| Power profiles | [power-profiles-daemon](https://repology.org/project/power-profiles-daemon/versions) 0.21–0.30, or [tuned-ppd](https://fedoraproject.org/wiki/Changes/TunedAsTheDefaultPowerProfileManagementDaemon) | same D-Bus API — tuned-ppd is explicitly a drop-in translation layer, which is why Fedora could switch defaults from ppd to tuned in F41 without desktops changing code | Integrate against the API, not the implementation, and both distros are covered |
docs/integration/hardware-media.md:::on Fedora 43 (verified), and fcitx5's own issue tracker records river being
docs/integration/hardware-media.md:::2.17.1 on Ubuntu 26.04, 2.17.0 on Fedora 43, 2.18.x on Arch and nixpkgs —
docs/integration/hardware-media.md:::everywhere, 4.7 on Fedora 43 — verified) to do what a grid of thumbnails does at
EOF
}

is_historical_path() {
    candidate=$1
    for historical_path in $historical_inputs; do
        [ "$historical_path" = "$candidate" ] && return 0
    done
    return 1
}

is_historical_exception() {
    candidate=$1

    # Do not use grep -q in this pipeline: it closes the pipe after a first
    # match and makes the literal here-document producer report SIGPIPE.
    if historical_allowlist | grep -F -x -e "$candidate" >/dev/null; then
        return 0
    fi
    return 1
}

projection_paths() {
    find "$root" -type f \
        ! -path "$root/.git/*" \
        ! -path "$root/.worktrees/*" \
        ! -path "$root/target/*" \
        ! -path "$root/.superpowers/sdd/*" \
        ! -path "$root/docs/superpowers/plans/*" \
        ! -path "$root/packaging/fedora/check-projections.sh" \
        ! -path "$root/packaging/fedora/test-check-projections.sh" \
        ! -path "$root/packaging/fedora/check-baseline.sh" \
        ! -path "$root/packaging/fedora/test-check-baseline.sh" \
        \( -name '*.md' -o -name '*.yml' -o -name '*.yaml' -o -name '*.toml' -o -name '*.spec' \) \
        | sed "s#^$root/##" | sort
}

assert_discovered_claims_are_reviewed() {
    pattern='(Fedora 41|F41|fedora-41|fedora:41|Fedora 43|F43|fedora-43|fedora:43|Fedora[[:space:]]*44\+|Fedora[[:space:]]*44[[:space:]]+(and[[:space:]]+)?(later|newer)|Fedora[^[:alnum:]]*(latest|Rawhide)|fedora:(latest|rawhide))'

    while IFS= read -r path || [ -n "$path" ]; do
        matches=$(grep -E -i -e "$pattern" "$root/$path" || true)
        while IFS= read -r line || [ -n "$line" ]; do
            [ -n "$line" ] || continue
            entry="$path:::$line"
            if ! is_historical_exception "$entry"; then
                if is_historical_path "$path"; then
                    fail "unreviewed historical Fedora claim: $path"
                fi
                fail 'forbidden Fedora 41 current claim'
            fi
        done <<EOF
$matches
EOF
    done <<EOF
$(projection_paths)
EOF
}

assert_no_live_fedora_support_claims() {
    pattern='Fedora[[:space:]]*44|F44|fedora[-:]44'

    while IFS= read -r path || [ -n "$path" ]; do
        is_historical_path "$path" && continue
        [ "$path" = 'packaging/fedora/baseline.toml' ] && continue
        if grep -E -i -q -e "$pattern" "$root/$path"; then
            fail 'unsupported Fedora status requires no Fedora lane or current support claim'
        fi
    done <<EOF
$(projection_paths)
EOF
}

baseline="$root/packaging/fedora/baseline.toml"
status=$(sed -n 's/^status = "\([^"]*\)"$/\1/p' "$baseline")
image=$(sed -n 's/^image = "\([^"]*\)"$/\1/p' "$baseline")
[ -n "$status" ] || fail 'baseline.toml has no exact status field'
[ -n "$image" ] || fail 'baseline.toml has no exact image field'

workflow="$root/.github/workflows/distro.yml"
workflow_files=$(find "$root/.github/workflows" -type f \( -name '*.yml' -o -name '*.yaml' \) | sort)
[ -n "$workflow_files" ] || fail 'Fedora workflow inventory is empty'
# shellcheck disable=SC2086 # The inventory is a newline-separated repository path list.
inventory=$(awk -v image="$image" '
    function trim(value) {
        sub(/^[[:space:]]+/, "", value)
        sub(/[[:space:]]+$/, "", value)
        return value
    }

    function normalized_image(value) {
        value = trim(value)
        if (value ~ /^\{[[:space:]]*image:[[:space:]]*/) {
            sub(/^\{[[:space:]]*image:[[:space:]]*/, "", value)
            sub(/[[:space:]]*\}$/, "", value)
            value = trim(value)
        }
        if (value ~ /^".*"$/ || value ~ /^\047.*\047$/) {
            value = substr(value, 2, length(value) - 2)
        }
        return value
    }

    function is_fedora_image(value) {
        value = tolower(value)
        return value ~ /^([^[:space:]]*\/)*fedora[^\/:@]*(:|@)/
    }

    function observe_container(value, cargo_context, job_context) {
        value = normalized_image(value)
        if (!is_fedora_image(value)) {
            return
        }
        fedora_containers++
        if (value == image) {
            exact_images++
            if (cargo_context) {
                cargo_images++
            }
            if (job_context == "fedora-rpm-package") {
                rpm_images++
            }
        }
    }

    {
        line = $0
        indentation = line
        sub(/[^[:space:]].*$/, "", indentation)
        indentation = length(indentation)

        if (line ~ /^  [[:alnum:]_-]+:$/) {
            job = line
            sub(/^  /, "", job)
            sub(/:$/, "", job)
            if (job == "fedora-rpm-package") {
                rpm_jobs++
            }
        }

        if (line == "          - name: fedora-44-cargo-smoke") {
            cargo_lanes++
            cargo_context = 1
            next
        }
        if (cargo_context && line ~ /^          - name: /) {
            cargo_context = 0
        }

        if (mapping_container) {
            if (indentation > mapping_indentation && line ~ /^[[:space:]]*image:[[:space:]]*/) {
                value = line
                sub(/^[[:space:]]*image:[[:space:]]*/, "", value)
                observe_container(value, mapping_cargo_context, mapping_job_context)
                mapping_container = 0
                next
            }
            if (indentation <= mapping_indentation) {
                mapping_container = 0
            }
        }

        if (line ~ /^[[:space:]]*container:[[:space:]]*/) {
            value = line
            sub(/^[[:space:]]*container:[[:space:]]*/, "", value)
            if (value == "") {
                mapping_container = 1
                mapping_indentation = indentation
                mapping_cargo_context = cargo_context
                mapping_job_context = job
                next
            }
            observe_container(value, cargo_context, job)
        }

        if (job == "fedora-rpm-package") {
            if (line == "    name: Fedora 44 retained-source RPM build") {
                rpm_names++
            }
            if (line == "      - name: Build retained-only RPM Source0") {
                rpm_build_steps++
            }
            if (line == "          packaging/tool-sources/build-native-source-kits.sh \"$RUNNER_TEMP/helm-native-kits\"") {
                rpm_source_kit_producers++
            }
            if (line == "          cp \"$RUNNER_TEMP/helm-native-kits/helm-0.1.0.tar.gz\" \"$RUNNER_TEMP/rpmbuild/SOURCES/\"") {
                rpm_source0_copies++
            }
            if (line == "          cp \"$RUNNER_TEMP/helm-native-kits/helm.spec\" \"$RUNNER_TEMP/rpmbuild/SPECS/\"") {
                rpm_spec_copies++
            }
            if (line == "          rpmbuild -bb --nodeps \\") {
                rpmbuild_invocations++
            }
            if (line == "            \"$RUNNER_TEMP/rpmbuild/SPECS/helm.spec\"") {
                rpmbuild_spec_targets++
            }
        }
    }

    END {
        print fedora_containers + 0, exact_images + 0, cargo_lanes + 0,
            cargo_images + 0, rpm_jobs + 0, rpm_names + 0, rpm_images + 0,
            rpm_build_steps + 0, rpm_source_kit_producers + 0,
            rpm_source0_copies + 0, rpm_spec_copies + 0,
            rpmbuild_invocations + 0, rpmbuild_spec_targets + 0
    }
' $workflow_files)
# shellcheck disable=SC2086 # The awk inventory emits exactly thirteen integers.
set -- $inventory
fedora_container_count=$1
workflow_image_count=$2
cargo_lane_count=$3
cargo_lane_image_count=$4
rpm_lane_count=$5
rpm_lane_name_count=$6
rpm_lane_image_count=$7
rpm_build_step_count=$8
rpm_source_kit_producer_count=$9
shift 9
rpm_source0_copy_count=$1
rpm_spec_copy_count=$2
rpmbuild_invocation_count=$3
rpmbuild_spec_target_count=$4

assert_ci_invocation() {
    invocation=$1
    if ! awk -v invocation="$invocation" '
        {
            line = $0
            sub(/^[[:space:]]*/, "", line)
            if (line == invocation) {
                found = 1
            }
        }
        END { exit(found ? 0 : 1) }
    ' "$workflow"; then
        fail "normal PR/push CI must invoke $invocation"
    fi
}

assert_fedora_build_evidence_boundary() {
    # A claim may span lines, so examine complete paragraphs in every discovered
    # projection instead of only known current files or single matching lines.
    projection_files=$(projection_paths | sed "s#^#$root/#")
    # shellcheck disable=SC2086 # Projection paths are a newline-separated repository path list.
    if ! awk '
        function inspect_paragraph() {
            lower = tolower(paragraph)
            fedora = "(fedora[[:space:]]*(44|lane)|f44|fedora[-:]44)"
            runtime = "(clean[-[:space:]]*install|package[[:space:]]+installation|graphical|session|selinux)"
            boundary = "(does[[:space:]]+not|do[[:space:]]+not|cannot|not[[:space:]]+(clean|install|graphical|session|selinux|evidence|support|working)|no[[:space:]]+(clean|package|graphical|session|selinux|architecture)|unverified|unsupported|neither|nor[[:space:]]|without|outside|blocked)"
            if (lower ~ fedora && lower ~ runtime && lower !~ boundary) {
                exit 1
            }
        }

        FNR == 1 {
            if (paragraph != "") {
                inspect_paragraph()
            }
            paragraph = ""
        }

        /^[[:space:]]*$/ {
            inspect_paragraph()
            paragraph = ""
            next
        }

        {
            paragraph = paragraph "\n" $0
        }

        END {
            inspect_paragraph()
        }
    ' $projection_files; then
        fail 'Fedora evidence exceeds build-only contract'
    fi
}

case "$status" in
    pre-alpha)
        [ "$fedora_container_count" -eq 2 ] || fail 'exactly one Fedora 44 Cargo-smoke lane and one retained-source RPM build lane are required'
        [ "$workflow_image_count" -eq 2 ] || fail 'workflow image must equal baseline.toml'
        if ! { [ "$cargo_lane_count" -eq 1 ] && [ "$cargo_lane_image_count" -eq 1 ] &&
            [ "$rpm_lane_count" -eq 1 ] && [ "$rpm_lane_name_count" -eq 1 ] &&
            [ "$rpm_lane_image_count" -eq 1 ] && [ "$rpm_build_step_count" -eq 1 ]; }; then
            fail 'exactly one Fedora 44 Cargo-smoke lane and one retained-source RPM build lane are required'
        fi
        if ! { [ "$rpm_source_kit_producer_count" -eq 1 ] && [ "$rpm_source0_copy_count" -eq 1 ] &&
            [ "$rpm_spec_copy_count" -eq 1 ] && [ "$rpmbuild_invocation_count" -eq 1 ] &&
            [ "$rpmbuild_spec_target_count" -eq 1 ]; }; then
            fail 'RPM lane must run the retained-source producer and Source0 RPM build'
        fi
        assert_contains .github/workflows/distro.yml 'pinned base/current packages' \
            'Fedora lane must say pinned base/current packages'
        ;;
    unsupported)
        if [ "$fedora_container_count" -ne 0 ]; then
            fail 'unsupported Fedora status requires no Fedora lane or current support claim'
        fi
        assert_no_live_fedora_support_claims
        ;;
    *) fail "unsupported Fedora baseline status: $status" ;;
esac

# shellcheck disable=SC2016 # The workflow must contain this literal command.
for invocation in \
    './packaging/fedora/check-baseline.sh --date "$(date -u +%F)"' \
    './packaging/fedora/check-projections.sh' \
    './packaging/fedora/test-check-baseline.sh' \
    './packaging/fedora/test-check-projections.sh'; do
    assert_ci_invocation "$invocation"
done

if current_matches '(Fedora 41|F41|fedora-41|fedora:41)'; then
    fail 'forbidden Fedora 41 current claim'
fi

if current_matches '(Fedora 43|F43|fedora-43|fedora:43)'; then
    fail 'exactly one Fedora 44 Cargo-smoke lane and one retained-source RPM build lane are required'
fi

if current_matches '(Fedora[[:space:]]*44\+|Fedora[[:space:]]*44[[:space:]]+(and[[:space:]]+)?(later|newer)|Fedora[^[:alnum:]]*(latest|Rawhide)|fedora:(latest|rawhide))'; then
    fail 'implicit or floating Fedora target'
fi

if [ "$status" = pre-alpha ]; then
rpm="$root/packaging/fedora/helm.spec"
assert_contains packaging/fedora/helm.spec '# helm — Fedora 44 pre-alpha spec (ADR 0015 / SPEC 0009).' \
    'RPM header must identify the Fedora 44 pre-alpha baseline'
rpm_dependency_metadata=$(awk '
    /^%description/ { exit }
    /^[[:space:]]*(Build)?(Requires|Recommends|Suggests|Supplements|Enhances|Conflicts|Obsoletes|Provides):/ {
        print
    }
' "$rpm")
canonical_river_dependency='Requires:       river >= 0.4.0'
river_metadata_count=$(printf '%s\n' "$rpm_dependency_metadata" | awk '
    /(^|[^[:alnum:]_])river([[:space:]-]|$)/ { count++ }
    END { print count + 0 }
')
canonical_river_count=$(printf '%s\n' "$rpm_dependency_metadata" | grep -F -x -c -e "$canonical_river_dependency" || true)
helm_river_metadata_count=$(printf '%s\n' "$rpm_dependency_metadata" | grep -F -c -e 'helm-river' || true)
if ! { [ "$river_metadata_count" -eq 1 ] && [ "$canonical_river_count" -eq 1 ] &&
    [ "$helm_river_metadata_count" -eq 0 ]; }; then
    fail 'RPM must contain only the canonical active river dependency'
fi
assert_contains packaging/fedora/helm.spec \
    'THIS PACKAGE IS PRE-ALPHA AND DOES NOT INSTALL A WORKING DESKTOP.' \
    'RPM must retain the pre-alpha no-working-desktop boundary'

assert_contains README.md 'Today Fedora 44 is the sole Fedora' \
    'README must identify Fedora 44 as the sole pre-alpha Fedora baseline'
assert_contains docs/INSTALL.md '| Fedora 44 (pre-alpha) | RPM from the retained-only source kit' \
    'INSTALL must identify the Fedora 44 retained-only source-kit RPM build'
assert_contains docs/ARCHITECTURE.md '| **Fedora 44 (pre-alpha)** | exactly one pinned Cargo-smoke lane plus exactly one retained-source RPM-build lane' \
    'ARCHITECTURE must identify the two Fedora 44 pre-alpha build lanes'
assert_contains docs/ARCHITECTURE.md 'Target plan and current evidence:' \
    'ARCHITECTURE must not claim every target is already supported and tested in CI'
assert_contains docs/MVP.md 'Fedora uses its official native candidate' \
    'MVP must record target-specific Fedora River packaging'
assert_contains docs/ROADMAP.md "Fedora 44's official native" \
    'ROADMAP must record the Fedora 44 native River candidate'
assert_contains docs/integration/session-services.md \
    'the sole pre-alpha Fedora packaging target; this dated' \
    'session-services must identify the Fedora 44 pre-alpha boundary'
assert_contains .github/ISSUE_TEMPLATE/bug.yml 'Fedora 44, built from source' \
    'bug template must use the exact Fedora 44 source-build example'
assert_contains docs/specs/0006-helm-ctl.md '· Fedora 44 ·' \
    'SPEC 0006 doctor example must use Fedora 44'

if current_matches '(vendor|vendors|vendored).*River.*(every target|all three (target|package))|River.*(every target|all three (target|package))|(every target|all three (target|package)).*River'; then
    fail 'universal River sourcing claim'
fi

assert_fedora_build_evidence_boundary
fi

historical_allowlist() {
    cat <<'EOF'
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
docs/specs/0009-fedora-44-pre-alpha-baseline.md:::| A2 | Given the repository workflow files, when `status = "pre-alpha"`, then exactly one `fedora-44-cargo-smoke` Cargo lane and exactly one `fedora-rpm-package` retained-source RPM build lane resolve to the exact official Fedora 44 digest above; no other Fedora-family container image is present anywhere under `.github/workflows/`, regardless of scalar or mapping YAML syntax. The RPM lane runs the retained-source-kit producer, copies `Source0` and the RPM spec into the build tree, and invokes `rpmbuild -bb --nodeps`, but does not clean-install the resulting package. Cargo and RPM build facts are allowed; neither lane is graphical-session or SELinux evidence. When `status = "unsupported"`, it contains no required Fedora lane; neither state adds a Fedora 41/Fedora 43 lane, runner, or architecture claim | `packaging/fedora/test-check-projections.sh`: `fedora_baseline::required_ci_uses_one_pinned_f44_cargo_smoke_and_one_retained_source_rpm_build` |
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
docs/integration/hardware-media.md:::| [NetworkManager](https://networkmanager.dev/blog/networkmanager-1-58/) | The general-purpose one: wifi, ethernet, WWAN, VPN plugins, per-connection DNS | 1.58.1 (Arch), 1.54.3 (Ubuntu 26.04), 1.54.0 (Fedora 43), 1.46 (Ubuntu 24.04). [1.58 released 2026-07-20](https://networkmanager.dev/blog/networkmanager-1-58/) | The only candidate with a real VPN story. Rich D-Bus API. Installed by default on all three targets |
docs/integration/hardware-media.md:::| [`kanshi`](https://repology.org/project/kanshi/versions) | C | 1.9.0 newest; Ubuntu 24.04 has 1.5.1, Fedora 43 has 1.8.0, Ubuntu 26.04 has 1.9.0 | Profile-based: match a set of connected outputs, apply a layout. Packaged on every target |
docs/integration/hardware-media.md:::| Power profiles | [power-profiles-daemon](https://repology.org/project/power-profiles-daemon/versions) 0.21–0.30, or [tuned-ppd](https://fedoraproject.org/wiki/Changes/TunedAsTheDefaultPowerProfileManagementDaemon) | same D-Bus API — tuned-ppd is explicitly a drop-in translation layer, which is why Fedora could switch defaults from ppd to tuned in F41 without desktops changing code | Integrate against the API, not the implementation, and both distros are covered |
docs/integration/hardware-media.md:::on Fedora 43 (verified), and fcitx5's own issue tracker records river being
docs/integration/hardware-media.md:::2.17.1 on Ubuntu 26.04, 2.17.0 on Fedora 43, 2.18.x on Arch and nixpkgs —
docs/integration/hardware-media.md:::everywhere, 4.7 on Fedora 43 — verified) to do what a grid of thumbnails does at
EOF
}

assert_historical_allowlist_is_exact() {
    while IFS= read -r entry || [ -n "$entry" ]; do
        [ -n "$entry" ] || continue
        allowlist_path=${entry%%:::*}
        exact_line=${entry#*:::}
        count=$(grep -F -x -c -e "$exact_line" "$root/$allowlist_path" || true)
        [ "$count" -eq 1 ] || fail "historical exception changed: $allowlist_path"
    done <<EOF
$(historical_allowlist)
EOF
}

assert_historical_allowlist_is_exact
assert_discovered_claims_are_reviewed

for path in $historical_inputs; do
    matches=$(grep -E -i \
        -e '(Fedora 41|F41|fedora-41|fedora:41|44\+|Rawhide|Fedora[^[:alnum:]]+latest|track = "latest"|helm-river|vendored[^.]*river|vendors[^.]*river)' \
        "$root/$path" || true)
    while IFS= read -r line || [ -n "$line" ]; do
        [ -n "$line" ] || continue
        entry="$path:::$line"
        # Do not use grep -q here: it closes the pipe after the first
        # match and makes the literal here-document producer emit SIGPIPE.
        if ! historical_allowlist | grep -F -x -e "$entry" >/dev/null; then
            fail "unreviewed historical Fedora claim: $path"
        fi
    done <<EOF
$matches
EOF
done

echo 'PASS: Fedora 44 projections match SPEC 0009'
