# Immutable Theme Generations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task. Steps use checkbox syntax.

**Goal:** Replace mutable theme publication with validated sealed generations.

**Architecture:** `helm-theme` gains a generation module which owns descriptor-relative tree construction, canonical manifest validation, the single advisory lock, pointer commit/recovery, leases and conservative GC. Existing direct-target reload fan-out is removed from the generation path.

**Tech Stack:** Rust 1.85, `rustix`, SHA-256 crate already selected after dependency review, tempfile unit fixtures.

**Spec:** `docs/specs/0011-theme-activation-generations.md`

## Global Constraints

- All generated content stays below Helm's 0700 owned subtree.
- No symlink-following; descriptor-relative operations only.
- `current` changes only after a sealed fsynced tree; corrupt state fails closed.
- Existing processes are never signalled/reloaded by a generation pointer switch.

### Task 1: Generation record and manifest validation

**Files:** create `crates/helm-theme/src/generation.rs`; modify `lib.rs`; test `generation.rs`.

- [ ] Write tests rejecting traversal, duplicate paths, symlinks, special files, malformed canonical manifests, and mismatched output digests.
- [ ] Run `cargo test -p helm-theme generation::tests` and observe the missing-module failure.
- [ ] Implement `GenerationId`, canonical `Manifest`, descriptor-relative validation, and SHA-256 verification.
- [ ] Re-run `cargo test -p helm-theme generation::tests` until green; commit `feat: validate sealed theme generations`.

### Task 2: Locked publication and recovery

**Files:** modify `generation.rs`, `theme.rs`; test `generation.rs`.

- [ ] Write G2/G3/G4/G8/G9 tests using injected commit checkpoints and two writers.
- [ ] Run the focused tests and observe failure.
- [ ] Implement one persistent no-follow lock inode, operation-scoped root control validation, staging/fsync/seal/publication-order/pointer ordering, and fail-closed recovery.
- [ ] Run `cargo test -p helm-theme generation::tests`; commit `feat: publish theme generations atomically`.

### Task 3: Pointer selection, leases and GC

**Files:** modify `generation.rs`; test `generation.rs`.

- [ ] Write G1/G5/G6/G7 tests for old-generation selection, invalid pointer refusal, stale PID/boot/start-time leases, explicit publication-sequence retention independent of IDs/mtimes, malformed or missing publication order, protected retention and rollback.
- [ ] Run focused tests and observe failure.
- [ ] Implement shared selection independent of publication-order validity, fsynced leases-before-exec, rollback and conservative GC retaining active plus the two greatest publication-sequence unleased generations, with stale-lease cleanup but zero generation deletion when order is unsafe, missing or malformed.
- [ ] Run `cargo test -p helm-theme generation::tests`; commit `feat: manage theme generation leases`.

### Task 4: Integrate and prove no legacy reload leakage

**Files:** modify `theme.rs`, `template.rs`, `lib.rs`; test `theme.rs`.

- [ ] Write a test proving a generation pointer switch emits no `Reloader` call and direct mutable target publication is unavailable on the generation path.
- [ ] Run it red, integrate `apply_with_inner` with generation publication, then run it green.
- [ ] Run `cargo test -p helm-theme`, `cargo test --workspace --all-features --locked`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo fmt --check`; commit `feat: activate themes by sealed generation`.

## Self-review

Tasks 1–4 cover G1–G9 respectively; no automatic deletion on uncertain liveness; #132/#133 boundaries remain unimplemented. The plan contains no cross-process reload or user-config ownership expansion.
