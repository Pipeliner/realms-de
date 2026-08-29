# ADR 0015 — Fedora 44 is the sole explicit pre-alpha Fedora baseline

- **Status:** Accepted (2026-08-29)
- **Deciders:** helm maintainers, repo owner
- **Supersedes / Superseded by:** Supersedes ADR 0010's Fedora 41-or-later baseline and Fedora portion of decision 7; supersedes ADR 0013 decision 4 only for Fedora

## Context

ADR 0010 selected one Fedora target but named Fedora 41 and later. ADR 0013
then assumed Fedora 41 provided only River 0.3.x or river-classic and required a
vendored River 0.4.x on every target. That was an explicit assumption to verify,
and ADR 0013 names distro catch-up as a reversal signal.

Fedora's authoritative lifecycle data now marks Fedora 41 and Fedora 42
archived. On 2026-08-29 it marked Fedora 43 current through 2026-12-02 and
Fedora 44 current through 2027-06-02. Official repository metadata exposed
River 0.3.14 for Fedora 43 and `river-0.4.8-1.fc44` for Fedora 44. River 0.4.0
is the release that introduced the external window-management architecture
ADR 0013 requires.

The repository's current Fedora job is only a Cargo smoke. The RPM says
explicitly that it installs no Helm binaries and no working desktop. Selecting
a current build/packaging baseline must not be confused with satisfying M3
Fedora support.

## Decision

1. Fedora 44 becomes Helm's sole explicit Fedora pre-alpha CI and packaging
   baseline. Fedora 41 is removed from required CI and current support claims.
2. Fedora 43 is not added. Supporting it would expand the one-Fedora-target
   intent and require a separate River path for a release with only 95 days of
   upstream lifecycle remaining at the decision date.
3. `44+`, `latest`, Rawhide, and implicit future-release support are rejected.
   A successor is admitted only by a later accepted contract based on then-live
   evidence.
4. The one Fedora Cargo-smoke lane uses Fedora's official Fedora 44 image at an
   admitted index digest. The digest pins the base image, not the live Fedora
   repositories or packages resolved from them.
5. Fedora package metadata depends on Fedora's official `river >= 0.4.0`
   package. The observed Fedora 44 candidate is `river-0.4.8-1.fc44`; this is a
   time-scoped package observation, not a tested Helm pairing. No `< 0.5`
   incompatibility policy is adopted. The planned M2 protocol and headless
   integration guards remain responsible for runtime compatibility.
6. This decision changes only Fedora's River source. Ubuntu and Nix River
   choices remain governed by ADR 0010/0013 until independently changed.
7. #138 may close when SPEC 0009's narrow baseline and truthfulness evidence
   pass. Complete Fedora desktop support remains a separate M2/M3 outcome and
   depends on the unresolved package provenance and consumer checks.
8. The recorded F44 EOL is enforced by a deterministic local validator whose
   date is an explicit input. An active baseline fails on and after its EOL.
   This adds no live query, scheduled job, daemon, or release-retirement SLA.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Fedora 44 only** | Longest current Fedora lifecycle; official River 0.4.8 provides the protocol generation ADR 0013 selected; replaces rather than expands the existing lane | **Chosen.** It is the narrowest truthful correction and avoids creating a short-lived second River path |
| **Fedora 43 and Fedora 44** | Covers both Fedora releases that were current on the decision date | Doubles the Fedora baseline paths; F43 provides River 0.3.14 and reaches EOL on 2026-12-02 |
| **Fedora 43 only** | It was still current and could be called the oldest supported Fedora | Preserves the unimplemented vendored-River burden and forces another baseline change after only 95 days |
| **Keep Fedora 41 or create a frozen F41 fixture** | Minimizes visible edits or preserves old compatibility investigations | F41 is EOL; a truthful offline fixture would require retained images, repository metadata, RPMs, and toolchains. No named regression question justifies that scope |
| **Use `latest`, Rawhide, or `44+`** | Avoids future release-number edits and may reveal future breakage early | Silently makes unadmitted releases support targets and makes CI advance without a reviewed product decision |

## Consequences

### Good

- Current CI and documentation stop claiming an EOL Fedora release.
- Matrix breadth remains one Fedora lane.
- Fedora's maintained native River package replaces an unimplemented
  Fedora-specific `helm-river` path.
- A future Fedora release cannot become supported accidentally.
- The recorded EOL becomes a failing local invariant instead of unguarded
  prose.

### Bad

- Fedora's fast lifecycle still requires a later explicit baseline decision.
- The Fedora package repositories used by the smoke lane remain mutable; the
  base-image digest alone does not make package resolution reproducible.
- The decision supplies no evidence that a complete Helm session works on
  Fedora. That remains visible rather than being papered over.

### Neutral

- Fedora 44's current River identity may change through updates. Resolved NEVRA
  is evidence to record, not a second cross-distro source of truth.
- No architecture support follows from the multi-architecture image index or
  from querying package metadata on two architectures.
- Yazi/Starship provenance, package publishing, `flake.lock`, VM/KVM topology,
  SELinux, and session rollback remain in their existing workstreams.

## Reversal

Low for the release identity: accept a later ADR/SPEC, replace the single
Fedora lane, reconcile package metadata and current documentation, and retain
this ADR as history. Reconsider when Fedora 44 reaches EOL, when Fedora removes
the required protocol-generation package, or when an implemented Helm/River
integration demonstrates an incompatibility. Do not infer a successor or an
upper version bound from the release number alone.

## Guard

- *Planned (#138/M0):*
  `fedora_baseline::only_fedora_44_is_a_live_target`, with negative fixtures
  for F41, F42, F43, `44+`, `latest`, Rawhide, and implicit-future wording, and
  exact reviewed exemptions for superseded/historical text.
- *Planned (#138/M0):*
  `fedora_baseline::required_ci_uses_one_pinned_f44_cargo_smoke`.
- *Planned (#138/M0):*
  `fedora_baseline::rpm_metadata_matches_the_f44_pre_alpha_contract`.
- *Planned (#138/M0):*
  `fedora_baseline::lifecycle_boundary_is_offline_and_inclusive_at_eol`, with
  fixed `2027-06-01`, `2027-06-02`, later-date, and unsupported-state fixtures.
  The ordinary repository invocation passes its UTC evaluation date explicitly
  and performs no network request.
- *Planned (#138/M0):*
  `fedora_baseline::decision_projections_record_partial_supersession`, covering
  ADR 0010, ADR 0013, the ADR index question split, and the append-only decision
  memory replacement.
- *Planned (M2, not #138):* the ADR 0013 protocol-version and headless River
  integration tests. Until they exist and pass, documentation must not call
  Fedora River a tested Helm pairing.

## Needs a human

No new human decision is required to select Fedora 44 under #138: it is an
evidence-backed correction that preserves the existing single-Fedora-target
intent. The repo owner authorized this decision after independent review on
2026-08-29.

The unrelated human decisions about Yazi/Starship source and package trust,
signing and publication, material runner infrastructure, and the response to a
future broken River pledge remain where their existing issues/ADRs place them.
This ADR neither resolves nor spends authority on them.
