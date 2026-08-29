# Fedora 44 Baseline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task.

**Goal:** Replace stale Fedora 41 claims with a tested, offline-validated Fedora 44 pre-alpha baseline.

**Architecture:** A checked-in Fedora baseline manifest is validated by a small local deterministic guard. Documentation, RPM metadata and the existing single Fedora Cargo-smoke job project that manifest without claiming M2/M3 runtime proof.

**Tech Stack:** TOML, POSIX shell, Rust workspace verification, GitHub Actions YAML, Markdown.

**Spec:** `docs/specs/0009-fedora-44-pre-alpha-baseline.md` and `docs/adr/0015-fedora-44-pre-alpha-baseline.md`.

## Global Constraints

- Fedora 44 only; no F43, `44+`, `latest`, Rawhide, extra lane, runner or architecture claim.
- Guard is offline and accepts an explicit date; it fails on and after EOL.
- The smoke lane is pinned-base/current-packages Cargo evidence only.
- Do not decide #134 provenance/signing, #135 consumer proof, KVM/VM, rollback, canary, or flake-lock policy.
- Write and observe each guard fixture failing before implementing its behaviour.

### Task 1: Accepted contract and decision records

**Files:** Create `docs/specs/0009-fedora-44-pre-alpha-baseline.md` and
`docs/adr/0015-fedora-44-pre-alpha-baseline.md`; update the specification/ADR
indexes and decision memory.

- [ ] Land the independently reviewed F44-only contract with its authorised
  `Accepted` status, rather than restoring the candidate's Draft/Proposed
  metadata.
- [ ] Record ADR 0015 partial supersession and split the Fedora-resolved / Ubuntu-open question.
- [ ] Append, rather than rewrite, the decision-memory correction.
- [ ] Verify every referenced document/path exists and docs contain no live F41 claim outside exact historical exceptions.

### Task 2: Baseline manifest and offline guard

**Files:** Create `packaging/fedora/baseline.toml`, `packaging/fedora/check-baseline.sh`, `packaging/fedora/test-check-baseline.sh`.

- [ ] Write fixtures that fail for EOL equality, post-EOL, an unknown field, and a pre-alpha F41 record.
- [ ] Run `packaging/fedora/test-check-baseline.sh` and observe the missing-guard failure.
- [ ] Implement the minimal strict TOML/date/lifecycle guard, then rerun all fixtures.
- [ ] Run `shellcheck packaging/fedora/check-baseline.sh packaging/fedora/test-check-baseline.sh`.

### Task 3: CI, RPM and live-document projection

**Files:** Modify `.github/workflows/distro.yml`, `packaging/fedora/helm.spec`, current target/install docs and examples named by SPEC 0009.

- [ ] Add a failing consistency inventory for F41/F43/unbounded-target claims and false evidence-level wording.
- [ ] Replace the one Fedora smoke lane with the admitted F44 digest and name it pinned-base/current-packages Cargo smoke.
- [ ] Use Fedora `river >= 0.4.0`; remove the `helm-river` alternative and false unavailable statement without imposing `<0.5`.
- [ ] Reconcile current support documents, retaining only enumerated historical F41 references.
- [ ] Run guard, shellcheck, Cargo formatting/clippy/tests and inspect workflow syntax.

### Task 4: Review and integration

- [ ] Run the full offline workspace suite and all local Fedora guard fixtures.
- [ ] Obtain adversarial spec/code review, address findings, and rerun scoped checks.
- [ ] Commit, push, open a PR and merge only after remote required checks pass.
