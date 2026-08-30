# Lifecycle Lease and Generation GC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement SPEC 0011's lifecycle-aware lease discrimination so generic generation GC never reclaims a transferred lifecycle lease from PID liveness or deletes any lease after lifecycle evidence becomes uncertain.

**Architecture:** Keep generation ownership in `helm-theme`. Add a typed lifecycle-lease record and dispatch generic GC by its discriminator before liveness; every present lifecycle lease pins its generation and freezes all lease and generation deletion for that pass. The later `helm-session` M2 component owns the activation registry, lock ordering, adoption, atomic transfer, and systemd/direct proof without creating a dependency from `helm-theme` back to session state.

**Tech Stack:** Rust 2021, rustix descriptor-relative filesystem operations, existing in-file `generation.rs` unit fixtures.

**Spec:** `docs/specs/0011-theme-activation-generations.md`, `docs/specs/0012-activation-launch-lifecycle.md`, `docs/adr/0017-immutable-theme-activation-generations.md`

## Global Constraints

- Preserve Rust 1.85 compatibility and existing no-follow descriptor-relative filesystem rules.
- Parse the lease discriminator before liveness; unknown, malformed, cross-kind, or unreadable lifecycle evidence retains every generation.
- The existing `garbage_collect()` API is conservative: every lifecycle parse/registry anomaly retains all leases and generations for that pass; only the future session owner may unlink a lifecycle lease after its own proof.
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

Add fixtures that write a valid `helm-generation-lifecycle-lease-v1` record, an unknown discriminator, a lifecycle record with a duplicated `launch` field, and each process-group/systemd cross-kind field combination. Add `pid 0`, `start-time 0`, direct `process-group` not equal to `pid`, systemd nonzero process group, invalid systemd unit name, invalid invocation ID, and direct non-none cgroup fixtures. Assert that only the valid record parses as `ParsedLeaseRecord::Lifecycle` and every other form is rejected.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme lifecycle_lease_parser -- --nocapture`

Expected: FAIL because `ParsedLeaseRecord` and the lifecycle parser do not exist.

- [ ] **Step 3: Implement canonical discriminator-first parsing**

Replace the single-version branch in `LeaseRecord::parse` with a `ParsedLeaseRecord::parse(raw)` dispatcher. Preserve the existing process-v1 byte grammar exactly. Implement `LifecycleLeaseRecord::parse` in SPEC 0012 field order, require one LF-terminated UTF-8 record with no extra fields, require positive lifecycle PID/start-time, and reject any owner-kind field combination not permitted by the contract. For systemd require `process-group 0`, exact `helm-launch-<launch-id>.scope`, lowercase 32-hex invocation, and non-none normalized cgroup with positive device/inode. For direct require `process-group == pid`, unit/cgroup fields `none`, and every cgroup number `0`.

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
- Keeps: `GenerationStore::garbage_collect()` as the only generic-GC API; session reconciliation owns lifecycle release and unlinks a validated lifecycle lease before GC can collect its generation.

- [ ] **Step 1: Add failing GC fixtures**

Publish at least four sealed generations, then combine a stale process-v1 lease with each of: a valid lifecycle-v1 lease, unknown discriminator, malformed lifecycle header, duplicated-field lifecycle body, cross-kind lifecycle body, and unreadable/wrong-type lease evidence. Assert every mixed inventory reports zero reclaimed leases/generations, retains both lease paths, and retains every generation. Add a separate process-only fixture to prove the existing stale process reclaim behavior remains unchanged when no lifecycle or malformed evidence exists.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme g6_gc_retains_lifecycle_lease -- --nocapture`

Expected: FAIL because GC treats every parsed lease as a process lease.

- [ ] **Step 3: Implement conservative GC dispatch**

Represent a complete scan as a pass-wide `LeaseInventory` with provisional stale process names, protected generations, and a `deletion_barrier`. Dispatch each entry through `ParsedLeaseRecord` before liveness. Apply `LeaseRecord::liveness()` only to `Process`; a valid lifecycle record inserts its generation into protected generations and sets the barrier because only the future M2 registry can validate/release it. Any unknown, malformed, unreadable, cross-kind, unsafe, or identity-unavailable entry also sets the barrier. Do not unlink provisional stale process names until the entire inventory finishes with no barrier. Never unlink a lifecycle record in generic GC. Preserve process-v1 stale reclaim only for an all-process, fully validated inventory; any lifecycle evidence or anomaly prevents every lease unlink and every generation deletion.

- [ ] **Step 4: Add explicit inspector-result tests**

Assert valid lifecycle, malformed lifecycle, cross-kind lifecycle, unknown discriminator, and unreadable lifecycle fixtures each retain every lease and every generation, including an otherwise stale process-v1 lease. Assert the process-only stale fixture still reclaims only its stale process lease. Name this generic behavior frozen pending M2 registry validation; do not claim the `helm-theme` slice validates registry evidence.

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

The lifecycle registry and atomic transfer are deliberately outside this first slice. Create their separate plan before implementation: the registry must issue an unforgeable transfer capability only after fsynced `preparing` state, verified scope/group adoption, and all common-field equality checks. Its red fixtures must cover consumed-selection drop ordering, write/rename/fsync crash boundaries with no lease-less state, systemd descendants, and direct witness proof across SPEC 0012 A1–A3. Do not add a placeholder `helm-session` crate before those red fixtures exist.
