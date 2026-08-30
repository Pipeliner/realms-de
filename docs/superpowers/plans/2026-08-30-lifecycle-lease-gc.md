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
- Keeps: `GenerationStore::garbage_collect()` as the only generic-GC API; session reconciliation owns lifecycle release and unlinks a validated lifecycle lease before GC can collect its generation.

- [ ] **Step 1: Add failing GC fixtures**

Use a stale supervisor PID in a valid lifecycle-v1 fixture and add two further sealed generations so collection would be observable. Assert that `garbage_collect()` reports zero reclaimed lifecycle leases and zero reclaimed generations, and the lifecycle lease plus its generation remain. In the same fixture add a stale process-v1 lease and assert it also remains. Add a separate process-only fixture to prove the existing stale process reclaim behavior remains unchanged when no lifecycle evidence exists.

- [ ] **Step 2: Run the focused red test**

Run: `cargo test -p helm-theme g6_gc_retains_lifecycle_lease -- --nocapture`

Expected: FAIL because GC treats every parsed lease as a process lease.

- [ ] **Step 3: Implement conservative GC dispatch**

Have `reclaim_stale_leases_locked` dispatch on `ParsedLeaseRecord`. Apply `LeaseRecord::liveness()` only to `Process`; add every lifecycle generation to the protected set and set uncertainty. Do not unlink queued stale process leases until the entire inventory finishes with no uncertainty. Never unlink a lifecycle record in the generic path. Preserve process-v1 stale reclaim only for an all-process, fully validated inventory; any lifecycle anomaly prevents every lease unlink and every generation deletion.

- [ ] **Step 4: Add explicit inspector-result tests**

Assert valid lifecycle, malformed lifecycle, cross-kind lifecycle, and unreadable lifecycle fixtures each retain every lease and every generation, including an otherwise stale process-v1 lease. Assert the process-only stale fixture still reclaims only its stale process lease.

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
