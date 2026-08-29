# Fedora Projection Guard Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task.

**Goal:** Make SPEC 0009's Fedora baseline/projection checks load-bearing in ordinary PR CI and unable to accept unsupported or RPM-metadata bypasses.

**Architecture:** Keep the repository-local POSIX shell guards as the sole policy evaluator. Extend their existing disposable-root fixture harness before changing guard logic. The workflow only invokes the already deterministic checks; it adds no runner, schedule, network probe, or publishing service.

**Tech Stack:** POSIX `sh`, `grep`, `sed`, `awk`, GitHub Actions YAML.

**Spec:** `docs/specs/0009-fedora-44-pre-alpha-baseline.md` §§1–5, acceptance A1–A3 and A5–A10; `docs/adr/0015-fedora-44-pre-alpha-baseline.md`.

## Global Constraints

- Fedora 44 is the only pre-alpha baseline; no implicit future-release support.
- `unsupported` admits no current Fedora support claim or required Fedora lane.
- Fedora's RPM dependency is exactly `river >= 0.4.0`; do not invent a `< 0.5` ceiling or retain a `helm-river` alternative.
- Current-document discovery may not be bypassed by adding an unlisted file; historical exceptions must be exact path-and-line allowlist entries.
- The checks remain local/offline and POSIX-shell compatible; no KVM, self-hosted runner, scheduled network canary, package publication, or Fedora 41 fixture.

### Task 1: Add regression fixtures before guard changes

**Files:**
- Modify: `packaging/fedora/test-check-projections.sh`

- [ ] Add fixtures which currently pass but must fail: `unsupported` with a renamed Fedora lane plus current F44 claim, a `river < 0.5.0` ceiling, a second River requirement/alternative, a new unlisted current-support document, and removal of each ordinary-CI invocation.
- [ ] Run `./packaging/fedora/test-check-projections.sh`; observe the new fixtures fail because the existing guard has no state-aware discovery/metadata validation/CI invocation checks.

### Task 2: Make projection validation state-aware and complete

**Files:**
- Modify: `packaging/fedora/check-projections.sh`
- Test: `packaging/fedora/test-check-projections.sh`

- [ ] In `pre-alpha`, require one exact F44 Cargo-smoke lane, matching pinned image, precise documentation evidence wording, and exactly the supported active RPM dependency form.
- [ ] In `unsupported`, reject every live Fedora lane/container/current support statement.
- [ ] Discover matching support claims across the supplied root, admitting historical matches only through complete path-and-line allowlist entries.
- [ ] Avoid short-circuit pipelines that emit broken-pipe diagnostics; run the fixture suite and observe all cases pass.

### Task 3: Make policy gates load-bearing in normal PR CI

**Files:**
- Modify: `.github/workflows/distro.yml`
- Test: `packaging/fedora/test-check-projections.sh`

- [ ] Add a normal GitHub-hosted PR/push job that runs `check-baseline.sh --date "$(date -u +%F)"`, `check-projections.sh`, and both fixture suites.
- [ ] Have the projection fixture suite assert those exact invocations so deleting one fails deterministically.
- [ ] Run the projection and baseline fixture suites, shell syntax checks, ShellCheck, formatting, workspace tests, and inspect the workflow diff for scope compliance.
