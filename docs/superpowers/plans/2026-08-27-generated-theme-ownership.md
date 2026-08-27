# Generated theme ownership implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make theme application write only Helm-owned files while safely activating GTK themes on first run.

**Architecture:** `helm-theme` renders every shipped target below `helm/generated/`. A small activation model records the user-side GTK file and its exact import line; it creates that file only when absent and otherwise reports the required remedy without modifying it. The later `helmctl doctor` implementation (#23) consumes this model rather than reconstructing paths and wording.

**Tech Stack:** Rust 2021, existing `helm-theme` descriptor-relative writer, Rust 1.85.

**Spec:** `docs/specs/0002-theme-pipeline.md`, “Generated-file ownership and activation”.

## Global Constraints

- Helm owns only `$XDG_CONFIG_HOME/helm/generated/**` and may replace only files there.
- An existing user activation file is never modified, appended to, replaced, or deleted.
- A missing GTK activation file may contain only the documented import.
- Preserve #110 descriptor containment, render-before-write, reload-after-commit, and #22's multi-file boundary.
- Preserve Rust 1.85 and `#![forbid(unsafe_code)]`.

---

### Task 1: Establish ownership regressions and activation model

**Files:**
- Modify: `crates/helm-theme/src/template.rs`
- Modify: `crates/helm-theme/src/theme.rs`

**Interfaces:**
- Produces a public, data-only activation diagnostic suitable for #23.

- [ ] Add failing tests proving every shipped template target starts with `helm/generated/`.
- [ ] Add failing GTK tests for: absent user `gtk.css` gets exactly one import; an existing file remains byte-identical; the diagnostic exposes its exact import and generated target.
- [ ] Run focused tests and observe the present targets/absence of activation behavior fail.
- [ ] Define the minimal `Activation`/diagnostic data type and make GTK template metadata declare its user activation path and import.
- [ ] Commit the regression/model increment.

### Task 2: Apply owned files and first-run activation safely

**Files:**
- Modify: `crates/helm-theme/src/template.rs`
- Modify: `crates/helm-theme/src/theme.rs`

**Interfaces:**
- Consumes Task 1 activation metadata.
- Produces `apply_with` that writes generated targets and creates only absent activation files.

- [ ] Redirect all shipped targets to `helm/generated/<tool>/...` without changing template source or reload identity.
- [ ] After successful owned-output commits, create each absent GTK activation file with the exact declared import; never mutate existing files.
- [ ] Ensure an activation-create failure is returned without claiming a modified user file; do not widen #22 atomicity.
- [ ] Run Task 1 tests red-to-green and the full `helm-theme` crate suite.
- [ ] Commit the implementation.

### Task 3: Verify, document, and hand off doctor diagnostics

**Files:**
- Modify: `docs/specs/0002-theme-pipeline.md`
- Modify: `crates/helm-theme/src/theme.rs`

- [ ] Add acceptance rows mapping ownership and existing-file behavior to exact tests.
- [ ] Document the stable diagnostic wording/data contract that #23's doctor command must print.
- [ ] Run `cargo fmt --check`, `cargo test --workspace --all-features`, and `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Commit, push, and comment on #33 with verification and the explicit #23 doctor dependency.
