# Helmctl Theme CLI Conformance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` task-by-task. Steps use checkbox syntax.

**Goal:** Close the accepted SPEC 0006 conformance gaps found by whole-branch review before shipping `helmctl`.

**Architecture:** Keep `helmctl` in-process and session-independent. Split palette selection from the apply-only seeding helper; give the CLI a top-level `--json` mode with deterministic serializers; keep negative command outcomes distinct from operational I/O failures.

**Tech Stack:** Rust 1.85, Clap derive, serde_json, helm-core Palette, helm-theme.

**Spec:** `docs/specs/0006-helm-ctl.md` §§2 and 4; `docs/specs/0002-theme-pipeline.md`; `docs/specs/0011-theme-activation-generations.md`.

## Global constraints

- `theme lint` reads `--palette`, then the user palette, then `SHIPPED_PALETTE`; it never seeds or writes configuration.
- `--json` produces exactly one JSON object on stdout and no stderr for defined theme outcomes.
- Exit 1 means a completed negative lint/diff result; exit 6 means apply/diff operational error or `OutcomeAmbiguous`.
- Do not add session IPC, reload, retry, recovery, daemon behavior, aliases, or a new public theme API.

### Task 1: Read-only lint selection and error class

**Files:** `crates/helm-ctl/src/main.rs`, `crates/helm-ctl/tests/theme_cli.rs`

- [ ] Write a subprocess regression using an empty config root: default lint succeeds using the shipped palette and a no-follow inventory remains byte-for-byte unchanged; run it red.
- [ ] Add a read-only loader that reads `root/helm/palette.toml` only if it is a regular available user file, otherwise parses `helm_theme::SHIPPED_PALETTE`; retain explicit `--palette` loading.
- [ ] Add a refusal fixture for diff and assert exit 6 while a changed diff remains exit 1.
- [ ] Run `cargo test -p helm-ctl lint_ diff_`, then commit `fix: make helmctl lint read-only`.

### Task 2: JSON mode and apply/diff operational mapping

**Files:** `crates/helm-ctl/src/main.rs`, `crates/helm-ctl/tests/theme_cli.rs`

- [ ] Write failing parser/output tests for `helmctl theme apply --json`, `lint --json`, and `diff --json`; test exact one-object stdout and empty stderr for each defined theme result.
- [ ] Add a global `--json` argument and deterministic structured renderers. Apply outcomes must match SPEC 0006 field order; lint/diff output shapes must be defined by the existing accepted spec or, if absent, add an explicit candidate spec clarification before claiming support.
- [ ] Map all `helm_theme::apply` and `helm_theme::diff` `Err` values to exit 6 with safe diagnostics. Keep explicit negative lint/diff outcomes at exit 1.
- [ ] Run focused tests, fmt, clippy, and commit `feat: add helmctl theme json output`.

### Task 3: Reconcile evidence and re-gate

**Files:** task reports, `docs/specs/0006-helm-ctl.md` only if a missing lint/diff JSON schema needs an explicit candidate clarification.

- [ ] Verify every newly claimed JSON object is normatively specified; do not invent a wire format from implementation convenience.
- [ ] Run workspace fmt, clippy, tests, release build, Fedora projections, and `git diff --check 51361bd..HEAD`.
- [ ] Obtain independent task and whole-branch review before pushing a revised PR.
