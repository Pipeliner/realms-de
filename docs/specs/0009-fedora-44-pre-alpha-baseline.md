# SPEC 0009 — Fedora 44 pre-alpha packaging and CI baseline

- **Status:** Accepted (2026-08-29)
- **Amendment:** Accepted (2026-09-01): one retained-source RPM build lane
  accompanies the one Cargo-smoke lane; exactly those two Fedora-family
  containers use the admitted image, and neither is runtime-install evidence.
- **Milestone:** M0 support-claim correction; M3 Fedora runtime acceptance is separate
- **Decision:** [ADR 0015](../adr/0015-fedora-44-pre-alpha-baseline.md)
- **Issue:** [#138](https://github.com/Pipeliner/realms-de/issues/138)
- **Supersedes / Superseded by:** — (ADR 0015 records the superseded ADR clauses)

## Purpose

Helm currently names Fedora 41 as a first-class target and runs its only
Fedora smoke job in a floating Fedora 41 container. Fedora 41 is archived and
no longer receives Fedora updates. This specification replaces that stale
claim with one truthful, explicitly versioned Fedora target without pretending
that the pre-alpha RPM installs a complete desktop.

In this specification, **Fedora baseline** means the sole Fedora release used
for Helm's current pre-alpha Cargo smoke and native-packaging preparation. It
does not mean that the M3 clean-install, login, portal, SELinux, upgrade, or
daily-driver acceptance obligations have passed.

## Scope

**In:**

- selecting Fedora 44 as the only explicit Fedora pre-alpha baseline;
- retiring Fedora 41 from required CI and current Helm support claims;
- declining Fedora 43 and any implicit future-release support;
- using Fedora's official Fedora 44 base image for exactly one Fedora
  Cargo-smoke lane and exactly one retained-source RPM build lane;
- recording Fedora's official `river` package as the Fedora 44 packaging
  candidate; and
- recording the exact Fedora baseline identity and EOL in one small local
  machine-readable record; and
- mechanically detecting stale current Fedora-baseline and lifecycle claims.

**Out:**

- Yazi and Starship provenance, repository ownership, package names, signing
  keys, retention, licensing closure, or incident response ([#134](https://github.com/Pipeliner/realms-de/issues/134));
- testing Helm's Yazi and Starship configurations against selected executables
  ([#135](https://github.com/Pipeliner/realms-de/issues/135));
- immutable generation retention, upgrade, rollback, and teardown semantics
  ([#131](https://github.com/Pipeliner/realms-de/issues/131) and
  [#132](https://github.com/Pipeliner/realms-de/issues/132));
- a Fedora VM, graphical login, portal, SELinux, RPM clean-install, or complete
  M2/M3 support claim;
- additional architectures, runners, KVM procurement, or self-hosted CI;
- a scheduled live-network canary or a release-admission/retirement SLA;
- an offline Fedora 41 fixture, unless a later issue names a concrete
  regression question and accepts its retention cost;
- `flake.lock` policy or implementation, which remains an independent ADR 0010
  reproducibility obligation; and
- any choice about where Helm packages are published. This specification
  neither creates nor publishes infrastructure.

## Behaviour

### 1. Exact target identity

1. With `status = "pre-alpha"`, Fedora 44 is Helm's **only** Fedora packaging
   and CI baseline. With `status = "unsupported"`, Helm has no current Fedora
   target or required Fedora lane.
2. Current support-facing and build-facing material must say `Fedora 44`, not
   `Fedora 44+`, `Fedora 44 and later`, `latest`, Rawhide, or an unbounded
   release expression.
3. Fedora 41 and Fedora 42 are EOL and are not current targets. Fedora 41 must
   not remain in a required job or live Helm support instruction. No Fedora 41
   fixture is created by this change.
4. Fedora 43 is not a Helm target. Although Fedora still marked it current on
   2026-08-29, it had 95 days to EOL and its official repository exposed River
   0.3.14, requiring a second compositor packaging path for a short-lived
   target.
5. Fedora 45 and later releases are not implicitly admitted. Changing the
   Fedora baseline requires new, accepted repository evidence and a deliberate
   contract change; numerical ordering alone never creates support.

### 2. Lifecycle truthfulness

This baseline records that Fedora's Bodhi service reported Fedora 44
as `current` on 2026-08-29, with EOL 2027-06-02. At or after Fedora declares
the release EOL, Helm must not continue describing it as a current Fedora
baseline: a maintainer must either accept a successor baseline or mark Fedora
unsupported.

This is an upstream-EOL boundary, not a new 90-day early-retirement policy.
No scheduled network check or successor canary is required by this
specification.

The implementation stores exactly the locally admitted lifecycle identity in
`packaging/fedora/baseline.toml`:

```toml
schema = "helm-fedora-baseline/v1"
status = "pre-alpha"
release = 44
eol = "2027-06-02"
image = "registry.fedoraproject.org/fedora:44@sha256:df52038ff64ee61affa188d78beb85cf6eecfe4e9f6042238269ccdc8e944392"
```

The record is a reviewed projection of this accepted contract, not live Fedora
metadata. Its fields and enum values are closed: `status` is `pre-alpha` or
`unsupported`; the date is a real ISO `YYYY-MM-DD` calendar date; no unknown
field is allowed. Every record has exactly these five fields: the fixed
`helm-fedora-baseline/v1` schema, one of the two allowed `status` values, and
the exact release, EOL and image above. Thus an `unsupported` record retains
the canonical identity of the last admitted Fedora baseline; `status` is the
only field that changes whether it is current. A later Fedora release requires
a superseding accepted contract before those identity values change.

The lifecycle validator is local and network-free. Its core operation receives
an explicit evaluation date. For `pre-alpha`, it passes only when
`evaluation_date < eol`; equality and later dates fail. For `unsupported`, the
same canonical identity is parsed and validated but the lifecycle comparison
passes; the support-claim guard separately requires that no required Fedora lane
or current-support statement remains. The normal
repository check supplies the current UTC calendar date explicitly; unit
fixtures inject fixed dates immediately before, on, and after EOL. Thus the
result is reproducible for an input date and begins failing at the recorded EOL
without a daemon, scheduled job, or network request.

### 3. Honest CI evidence

Fedora 44 has exactly two and only two workflow lanes. The existing
`fedora-44-cargo-smoke` matrix entry remains the sole Cargo-smoke lane, and
`fedora-rpm-package` is the sole retained-source RPM build lane. These are the
only Fedora-family container images in the workflow, and each resolves to the
official Fedora registry image admitted during review, regardless of whether
the YAML expresses `container` as a scalar, quoted scalar, inline mapping, or
mapping with `image`:

```text
registry.fedoraproject.org/fedora:44@sha256:df52038ff64ee61affa188d78beb85cf6eecfe4e9f6042238269ccdc8e944392
```

That digest pins the base-image index only. Because enabled Fedora repositories
remain live, the lane must be described as **pinned base/current packages**,
not as a reproducible Fedora package snapshot.

The Cargo lane builds and tests the Rust workspace with the repository
toolchain. The RPM lane runs the retained-source-kit producer, copies its
`Source0` and RPM spec into the RPM build tree, and invokes `rpmbuild -bb
--nodeps`; it does not install the resulting RPM. It is truthful to describe
those Cargo and retained-source RPM build facts. Together, they are build
evidence only: neither establishes RPM clean-install or dependency-graph
support, runtime dependency availability, graphical-session or SELinux
support, support for either architecture in the image index, or a working Helm
desktop.

### 4. Fedora's native River package

Fedora 44's official signed repositories resolved
`river-0.4.8-1.fc44` on both x86_64 and aarch64 during the 2026-08-29 review.
River 0.4.0 introduced the `river-window-management-v1` architecture chosen by
ADR 0013; Fedora 43's 0.3.14 does not provide it. Therefore Fedora 44 package
metadata uses Fedora's official `river >= 0.4.0` package and does not use the
unimplemented `helm-river` alternative.

The observed `0.4.8-1.fc44` NEVRA is evidence about Fedora's repository at the
observation time, not a claim that Helm has tested a River 0.4.8 session.
Helm's River backend and end-to-end session test do not yet exist. The lower
bound denotes the protocol-generation boundary; it is not a substitute for the
planned M2 protocol and headless integration guards.

This specification introduces no `< 0.5` RPM ceiling and makes no claim that
River 0.5 or later is incompatible. River upstream describes the protocol as
stable and pledges forward compatibility. If an actual future package violates
Helm's contract, live tests and a new decision—not an invented version rule—must
determine the response.

ADR 0015 supersedes ADR 0010's Fedora 41-or-later baseline clauses as well as
the Fedora-specific vendoring assumptions in ADR 0010 decision 7 and ADR 0013
decision 4. It does not select or change the River source used by Ubuntu or
Nix.

### 5. Documentation and history

All current Helm target, installation, CI, packaging, example-output, and
issue-template claims must be reconciled to the exact Fedora 44 pre-alpha
baseline and its stated evidence level. In particular, the workflow, Fedora
spec header and commentary, `README.md`, `docs/ARCHITECTURE.md`,
`docs/INSTALL.md`, `docs/MVP.md`, `docs/ROADMAP.md`,
`docs/integration/session-services.md`, the bug template, and the current
example in SPEC 0006 must not present Fedora 41 as current.

Superseded ADR 0010 and ADR 0013 wording remains as decision history; their
headers/index entries point to ADR 0015 for the affected clauses. Genuine
third-party historical facts, such as a package being available in Fedora 41,
may remain when they do not imply current Helm support. Every such exception
found by the consistency guard must be enumerated by exact path and matching
context in the guard; no broad directory or filename exemption is permitted.

The authoritative ADR records and their non-normative index/memory projections
require three explicit reconciliations:

1. ADR 0010's header/index must mark its Fedora 41-or-later baseline and the
   Fedora part of decision 7 superseded by ADR 0015, without rewriting the
   historical decision body.
2. The ADR index's combined Ubuntu/Fedora River-verification question must be
   split. Fedora's package-availability premise is resolved by the dated F44
   evidence and ADR 0015; the independent Ubuntu 24.04 question remains open
   under ADR 0013.
3. `.claude/memory/10-decisions.md` must follow its append-only rule: strike
   the stale combined/universal vendoring projections and append a dated
   ADR-0015 entry saying Fedora 44 uses Fedora's native River candidate while
   Ubuntu and Nix remain under ADR 0010/0013. It must not silently rewrite or
   delete the earlier reasoning.

The correction must not describe a direct Fedora 41 to Fedora 44 operating
system upgrade as supported. Fedora's documented supported paths do not cover
that three-release jump, and #138 defines no Helm-specific OS migration. Any
retirement guidance must point to Fedora's current upgrade documentation and
label an EOL/direct-jump attempt accurately, or recommend backup plus a fresh
Fedora 44 installation without promising application-state migration.

### 6. Closure boundary

Passing this specification establishes a truthful Fedora 44 **pre-alpha
baseline only**. Repository text must continue to say that the present RPM does
not install a working desktop while the required binaries and runtime evidence
are absent.

A clean, complete, daily-drivable Fedora support claim remains blocked on the
accepted outcomes of #134 and #135 and on the relevant M2/M3 RPM,
session-login, portal, and SELinux evidence. Those workstreams do not block
correcting the stale Fedora release baseline and closing #138 under this narrow
definition.

## Acceptance criteria

Each planned test is written and observed failing before the corresponding
repository correction is made. Exact final test names may follow the owning
test module, but each row remains independently observable.

| # | Given / When / Then | Test or evidence |
|---|---|---|
| A1 | Given the machine-checked inventory of live Fedora claims, when `status = "pre-alpha"` it is validated, then the only admitted release is exactly `44`; when `status = "unsupported"`, no Fedora release is admitted; `41`, `42`, `43`, `44+`, `latest`, Rawhide, and implicit newer releases are always rejected as current Helm targets | *Planned (#138):* `fedora_baseline::only_fedora_44_is_a_live_target`; includes one failing fixture per rejected form and one unsupported-state fixture |
| A2 | Given the required distro workflow, when `status = "pre-alpha"`, then exactly one `fedora-44-cargo-smoke` Cargo lane and exactly one `fedora-rpm-package` retained-source RPM build lane resolve to the exact official Fedora 44 digest above; no other Fedora-family container image is present, regardless of scalar or mapping YAML syntax. The RPM lane runs the retained-source-kit producer, copies `Source0` and the RPM spec into the build tree, and invokes `rpmbuild -bb --nodeps`, but does not clean-install the resulting package. Cargo and RPM build facts are allowed; neither lane is graphical-session or SELinux evidence. When `status = "unsupported"`, it contains no required Fedora lane; neither state adds a Fedora 41/Fedora 43 lane, runner, or architecture claim | `packaging/fedora/test-check-projections.sh`: `fedora_baseline::required_ci_uses_one_pinned_f44_cargo_smoke_and_one_retained_source_rpm_build` |
| A3 | Given the Fedora RPM metadata, when it is inspected, then it identifies Fedora 44, requires Fedora's `river >= 0.4.0`, contains no `helm-river` alternative or false “River unavailable on Fedora” claim, and continues to state that the package is pre-alpha and not a working desktop | *Planned (#138):* `fedora_baseline::rpm_metadata_matches_the_f44_pre_alpha_contract` |
| A4 | Given a disposable Fedora 44 environment using only official Fedora repositories, when the review probe queries lifecycle/package metadata, then the evidence records Bodhi `F44` as current with EOL 2027-06-02 and records the resolved River NEVRA; the record labels these observations as time-scoped and does not call the River/Helm pairing tested | Review evidence attached to #138 or its pull request; network observation, not a deterministic CI gate |
| A5 | Given the strict local baseline record and an injected evaluation date, when the lifecycle guard runs for 2027-06-01 it passes, when it runs for 2027-06-02 or a later date with `status = "pre-alpha"` it fails, and when Fedora is explicitly `unsupported` it permits retirement only if the support-claim guard finds no required lane/current claim | *Planned (#138):* `fedora_baseline::lifecycle_boundary_is_offline_and_inclusive_at_eol`; fixed-date fixtures and no network |
| A6 | Given a seeded stale live-support claim such as `Fedora 41+`, when the consistency guard runs, then it fails; given an exact reviewed historical exception in a superseded ADR or third-party history, then it passes without treating that text as current support | *Planned (#138):* `fedora_baseline::stale_live_claims_fail_and_exact_history_exceptions_pass` |
| A7 | Given ADR 0010, ADR 0013, the ADR index, and `.claude/memory/10-decisions.md`, when decision projections are checked, then the affected Fedora baseline/vendoring clauses point to ADR 0015, the Fedora half of the shared package-verification question is resolved while Ubuntu remains open, and the stale memory entries are struck plus replaced rather than erased | *Planned (#138):* `fedora_baseline::decision_projections_record_partial_supersession` plus document review |
| A8 | Given current user-facing and normative documentation, when the consistency guard and doc review run, then Fedora 41 is absent from live Helm support/install examples, Fedora 44 is described as the sole pre-alpha baseline, no text upgrades the Cargo smoke to RPM/session evidence, and no direct Fedora 41 to Fedora 44 OS upgrade is called supported | *Planned (#138):* `fedora_baseline::docs_state_the_evidence_level_truthfully` plus review of rendered Markdown |
| A9 | Given the completed #138 diff, when scope is reviewed, then it contains no Yazi/Starship source decision, package publishing/signing infrastructure, extra Fedora lane, KVM/native-runner acquisition, scheduled network job, Fedora 41 fixture, generation rollback design, or `flake.lock` strategy | Required reviewer checklist on the #138 pull request |
| A10 | Given all #138 changes, when existing repository formatting, lint, unit, workflow, and documentation checks run, then they remain green | Existing repository verification suite |

## Evidence and provenance

The contract decision is based on observations independently checked on
2026-08-29:

- Fedora Bodhi: F41 `archived`, EOL 2025-12-15; F42 `archived`, EOL
  2026-05-27; F43 `current`, EOL 2026-12-02; F44 `current`, EOL 2027-06-02.
- Fedora package metadata: F43 `river` 0.3.14; F44
  `river-0.4.8-1.fc44` on the two architectures queried.
- Fedora registry: the Fedora 44 index digest recorded in section 3.
- Repository state: the Fedora job was a Cargo-only `fedora:41` container;
  the RPM explicitly said it did not install a working desktop; neither a
  RiverBackend nor the M2 headless River acceptance test existed.

Primary sources:

- <https://bodhi.fedoraproject.org/releases/?state=current>
- <https://docs.fedoraproject.org/en-US/releases/lifecycle/>
- <https://packages.fedoraproject.org/pkgs/river/river/index.html>
- <https://registry.fedoraproject.org/v2/fedora/manifests/44>
- <https://codeberg.org/api/v1/repos/river/river/releases/tags/v0.4.0>
- <https://codeberg.org/api/v1/repos/river/river/releases/tags/v0.4.8>
- <https://isaacfreund.com/blog/river-window-management/>

These are dated observations. Live repository/package state wins over this
record when later work uses it.

## Budgets

#138 retains the one Cargo-smoke lane and adds the one specified retained-source
RPM build lane; it does not add a third Fedora lane, runner class,
native-architecture matrix, VM, or scheduled job. No new timing budget is
introduced. M2/M3 runtime and VM budgets remain owned by their accepted
contracts.

## Failure modes

| Failure | Guard |
|---|---|
| The recorded baseline crosses EOL without failing locally | A5 |
| An EOL Fedora release remains a live support claim | A1, A5, A6, A8 |
| A later Fedora release becomes supported through `+`, `latest`, or Rawhide | A1 |
| A base-image digest is called a reproducible package snapshot | A2, A8 |
| Fedora package availability is described as tested Helm runtime compatibility | A3, A4, A8 |
| The RPM build is treated as clean-install, graphical-session, or SELinux evidence | A2, A8 |
| Partial supersession leaves the ADR index or operational decision memory stale | A7 |
| #138 silently decides package trust, runner, rollback, or Nix policy | A9 |
| Historical evidence is destroyed merely to satisfy a global string search | A6's exact exception model and A7 |

## Open questions

None within this narrow baseline correction. The package-provenance and
distribution questions remain explicitly unresolved in #134 and ADR 0010. The
future response to a broken River compatibility pledge remains the existing
human question in ADR 0013. Neither is answered by selecting Fedora 44.
