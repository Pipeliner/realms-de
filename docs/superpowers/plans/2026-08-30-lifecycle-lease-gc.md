# Lifecycle Lease and Generation GC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement SPEC 0011's lifecycle-aware lease discrimination so generic generation GC never reclaims a transferred lifecycle lease from PID liveness.

**Architecture:** Keep generation ownership in `helm-theme`. Add a typed lifecycle-lease record and a small injected inspection boundary; generic GC treats every present lifecycle lease as protected. The later `helm-session` M2 component owns the activation registry, lock ordering, systemd/direct proof, and supplies validation without creating a dependency from `helm-theme` back to session state.

**Tech Stack:** Rust 2021, rustix descriptor-relative filesystem operations, existing in-file `generation.rs` unit fixtures.

**Spec:** `docs/specs/0011-theme-activation-generations.md`, `docs/specs/0012-activation-launch-lifecycle.md`, `docs/adr/0017-immutable-theme-activation-generations.md`

## Global Constraints

- Preserve Rust 1.85 compatibility and existing no-follow descriptor-relative filesystem rules.
- Parse the lease discriminator before liveness; unknown, malformed, cross-kind, or unreadable lifecycle evidence retains every generation.
- The existing `garbage_collect()` API is conservative: it retains every present lifecycle lease; only the future session owner may unlink one after its own proof.
- Do not select profile argv/assets, D-Bus activation, existing-owner behavior, River replay, or public lifecycle wire semantics; #133 and #135 own those decisions.
- Direct lifecycle release is not a PID liveness decision: it requires the durable owner witness, exact-owner staleness, and recorded-group emptiness supplied by the session owner.

---

### Task 1: Type and parse both canonical lease grammars

**Files:**
- Modify: `crates/helm-theme/src/generation.rs:112-120,1832-1905`
- Test: `crates/helm-theme/src/generation.rs:2953-3025,3236-3310`

**Interfaces:**
- Produces: `enum ParsedLeaseRecord { Process(LeaseRecord), Lifecycle(LifecycleLeaseRecord) }`.
- Produces: `LifecycleLeaseRecord` containing the SPEC 0012 generation, launch, process identity, owner UID, owner kind, and scope/group identity fields.
- Consumes: existing `GenerationId`, `canonical_u32`, `canonical_u64`, `canonical_boot_id`, and `record_value` helpers.

- [ ] **Step 1: Add failing canonical-parser fixtures**

Add fixtures that write a valid `helm-generation-lifecycle-lease-v1` record, an unknown discriminator, a lifecycle record with a duplicated `launch` field, and a lifecycle record with a process-group/systemd cross-kind field combination. Assert that only the valid record parses as `ParsedLeaseRecord::Lifecycle` and every other form is rejected.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme lifecycle_lease_parser -- --nocapture`

Expected: FAIL because `ParsedLeaseRecord` and the lifecycle parser do not exist.

- [ ] **Step 3: Implement canonical discriminator-first parsing**

Replace the single-version branch in `LeaseRecord::parse` with a `ParsedLeaseRecord::parse(raw)` dispatcher. Preserve the existing process-v1 byte grammar exactly. Implement `LifecycleLeaseRecord::parse` in SPEC 0012 field order, require one LF-terminated UTF-8 record with no extra fields, and reject any owner-kind field combination not permitted by the contract.

- [ ] **Step 4: Run the focused parser tests**

Run: `cargo test -p helm-theme lifecycle_lease_parser -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs
git commit -m "feat: parse lifecycle generation leases"
```

### Task 2: Make generic GC explicitly protect lifecycle leases

**Files:**
- Modify: `crates/helm-theme/src/generation.rs:920-1080`
- Test: `crates/helm-theme/src/generation.rs:3236-3460`

**Interfaces:**
- Produces: `pub enum LifecycleLeaseInspection { Protected, Uncertain }`.
- Produces: `pub trait LifecycleLeaseInspector { fn inspect(&self, lease: &LifecycleLeaseRecord) -> LifecycleLeaseInspection; }`.
- Produces: `GenerationStore::garbage_collect_with_lifecycle_inspector(&self, inspector: &impl LifecycleLeaseInspector) -> Result<GenerationGcReport, String>`.
- Keeps: `GenerationStore::garbage_collect()` by routing lifecycle leases to `Uncertain`.

- [ ] **Step 1: Add failing GC fixtures**

Use a stale supervisor PID in a valid lifecycle-v1 fixture and add two further sealed generations so collection would be observable. Assert that `garbage_collect()` reports zero reclaimed lifecycle leases and zero reclaimed generations, and the lifecycle lease plus its generation remain. Add a process-v1 stale-PID fixture in the same test module and assert its current reclaim behavior remains unchanged.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme g6_gc_retains_lifecycle_lease -- --nocapture`

Expected: FAIL because GC treats every parsed lease as a process lease.

- [ ] **Step 3: Implement conservative GC dispatch**

Have `reclaim_stale_leases_locked` dispatch on `ParsedLeaseRecord`. Apply `LeaseRecord::liveness()` only to `Process`; add every lifecycle generation to the protected set and set uncertainty after calling the inspector. Never unlink a lifecycle record in the generic path. Preserve the existing process-v1 stale unlink ordering and the zero-generation-deletion behavior after any uncertainty.

- [ ] **Step 4: Add explicit inspector-result tests**

Assert `Protected` and `Uncertain` both retain the lease/generation. Assert a lifecycle lease stays protected even when the inspector reports `Protected`, demonstrating that generic GC never interprets inspection as unlink authority.

- [ ] **Step 5: Run focused and workspace tests**

Run: `cargo test -p helm-theme g6_gc -- --nocapture`

Expected: PASS.

Run: `cargo test --workspace --all-features --locked --quiet`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/helm-theme/src/generation.rs
git commit -m "fix: retain lifecycle leases during generation GC"
```

### Task 3: Transfer a selected process lease without drop-release authority

**Files:**
- Modify: `crates/helm-theme/src/generation.rs:93-104,773-825,1784-1825`
- Test: `crates/helm-theme/src/generation.rs:3754-3795`

**Interfaces:**
- Produces: `GenerationSelection::transfer_to_lifecycle(self, record: LifecycleLeaseRecord) -> Result<TransferredGenerationLease, String>`.
- Produces: `TransferredGenerationLease` exposing only the generation and opaque lease name, with no `Drop` unlink behavior.
- Consumes: the same opaque lease filename, `renameat`/atomic replacement and directory `fsync` rules already used by lease creation.

- [ ] **Step 1: Add a failing transfer crash-boundary test**

Select a paused target with `select_current_for_process`, construct a matching lifecycle record, transfer it, then drop the original selection. Assert the same lease filename still exists, begins with `helm-generation-lifecycle-lease-v1`, and generic GC retains it even when the original process PID is stale.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme selection_transfer_to_lifecycle -- --nocapture`

Expected: FAIL because the selection has no transfer operation and `Drop` unlinks its lease.

- [ ] **Step 3: Implement atomic same-name transfer**

Write and fsync a current-UID mode-0600 temporary in the existing leases directory, atomically replace the process lease at the exact opaque filename, fsync the directory, and mark the consumed `GenerationSelection` released only after the replacement is durable. Reject generation, process identity, and owner UID mismatch before mutation.

- [ ] **Step 4: Run the transfer and full checks**

Run: `cargo test -p helm-theme selection_transfer_to_lifecycle -- --nocapture`

Expected: PASS.

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs
git commit -m "feat: transfer generation leases to lifecycle ownership"
```

The lifecycle registry is deliberately outside this first slice. Once Tasks 1–3 merge, create a separate plan for SPEC 0012 A1–A16: activation-root initialization, canonical launch records, systemd recursive cgroup proof, direct two-phase witness proof, crash-matrix recovery, and Nix VM fixtures. Do not add a placeholder `helm-session` crate before those red fixtures exist.
