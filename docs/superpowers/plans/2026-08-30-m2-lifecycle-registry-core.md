# M2 Lifecycle Registry Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the descriptor-safe, crash-aware M2 registry core that creates canonical gate-closed launch records and atomically consumes a generation process lease into a lifecycle lease without an unleased or stale-drop state.

**Architecture:** Keep the registry as the real child module `helm_theme::generation::lifecycle`, not a placeholder `helm-session` crate: as a child it can use the parent's private `GenerationSelection` transfer/cleanup operations, while no other crate can call them.  The first core exposes no public launch/transfer API; it owns activation-root/record validation, `lifecycle.lock`, and internal registry-only transfer/reconciliation interfaces.  It uses sealed fake ownership adapters in unit tests; real user-manager/D-Bus/profile launching remain separate #133/#135 work.

**Tech Stack:** Rust 2021 / MSRV 1.85, `rustix` descriptor-relative filesystem APIs, existing `helm-theme` generation locking and in-file Linux fixtures.

**Spec:** `docs/specs/0012-activation-launch-lifecycle.md`, `docs/specs/0011-theme-activation-generations.md`, `docs/superpowers/specs/2026-08-30-m2-lifecycle-registry-design.md`

## Global Constraints

- Never create `crates/helm-session` until it has its own real M2 implementation and red fixtures.
- The lock order is `lifecycle.lock`, then SPEC 0011's `activation.lock`; reverse acquisition is forbidden.
- All activation paths are descriptor-relative, `O_NOFOLLOW`, current-UID, exact mode 0700 directories / 0600 regular files; unsafe collisions fail closed without repair.
- Every canonical record is UTF-8, LF-terminated, at most 4096 bytes, ordered, duplicate-free, and atomically renamed then directory-fsynced.
- Generic generation GC remains conservative and never releases lifecycle records; only this module releases a lifecycle lease after the accepted proof.
- A selection cleanup may unlink only a canonical matching process lease; lifecycle, malformed, missing, unreadable, unsafe, or mismatched records are retained.
- The first registry-core PR may use deterministic fake ownership evidence.  It must not claim actual systemd manager, D-Bus activation, profile argv, or public lifecycle protocol behaviour.

---

### Task 1: Make `GenerationSelection` cleanup discriminator- and identity-safe

**Files:**
- Modify: `crates/helm-theme/src/generation.rs:91-109,1816-1861,3050-3210`
- Test: `crates/helm-theme/src/generation.rs` `generation::tests`

**Interfaces:**
- Produces private `GenerationSelection::release_matching_process_lease(&mut self) -> Result<(), String>`.
- Produces private `GenerationSelection::disarm_after_transfer(&mut self)` usable only by `lifecycle`.
- Keeps public `release(self)` but makes it prove the on-disk record is the selection's exact process lease before unlinking.
- Extends `GenerationSelection` with its immutable `LeaseRecord`, validated manifest seal digest, cloned generated-root/`activation.lock` descriptor, and the same `Arc<Mutex<()>>` used by `GenerationStore`; this lets the later child transfer operation acquire the real shared generation lock after `RegistryLock` without reopening or bypassing the existing intra-process serialization.

- [ ] **Step 1: Write the failing stale-drop test**

In the existing generation test module, select a generation for the test PID, replace its lease-name bytes with a hand-written canonical `helm-generation-lifecycle-lease-v1` record using the same generation and owner identity, then drop the original selection. Assert the lease pathname still exists and parses as lifecycle evidence.

```rust
#[test]
fn selection_drop_never_unlinks_a_replaced_lifecycle_lease() {
    let root = tempfile::tempdir().unwrap();
    let store = seeded_store(root.path());
    let selection = store.select_current().unwrap();
    let name = selection.lease_name.clone();
    write_raw_lease_fixture(root.path(), &name, lifecycle_direct_fixture(selection.as_str()));
    drop(selection);
    assert!(root.path().join("leases").join(name).exists());
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p helm-theme selection_drop_never_unlinks_a_replaced_lifecycle_lease -- --nocapture`

Expected: FAIL because the existing destructor unlinks the opaque filename without inspecting its content.

- [ ] **Step 3: Implement matching-process-only cleanup**

Reopen `lease_name` from `lease_directory` through the existing no-follow regular-file reader. Parse it through `ParsedLeaseRecord`; unlink only `ParsedLeaseRecord::Process(record)` when every generation/PID/start-time/boot/UID field equals the selection's saved `LeaseRecord`. Treat `NOENT`, lifecycle, malformed/unreadable/unsafe, and identity mismatch as retained no-ops. Set `released` only after a matching unlink/`NOENT` has been fsynced or a retained nonmatching record has been observed.

```rust
match self.read_current_lease()? {
    Some(ParsedLeaseRecord::Process(record)) if record == self.process_identity => {
        unlinkat(&self.lease_directory, self.lease_name.as_str(), AtFlags::empty())?;
        fsync(&self.lease_directory)?;
    }
    _ => {}
}
```

- [ ] **Step 4: Verify green and preserve normal cleanup**

Run:

```bash
cargo test -p helm-theme selection_drop_never_unlinks_a_replaced_lifecycle_lease -- --nocapture
cargo test -p helm-theme g1_selected_old_generation_keeps_descriptor_pinned_bytes_after_new_commit -- --nocapture
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs
git commit -m "fix: guard generation selection lease cleanup"
```

### Task 2: Add canonical activation-root, session claim, inventory, and launch-record primitives

**Files:**
- Create: `crates/helm-theme/src/generation/lifecycle.rs`
- Modify: `crates/helm-theme/src/generation.rs`
- Test: `crates/helm-theme/src/generation/lifecycle.rs` `#[cfg(test)]`

**Interfaces:**
- Produces private `ActivationRegistry`, `SessionId([u8; 16])`, `LaunchId([u8; 16])`, `SessionClaim`, and `ActiveSessionCapability`.
- Produces private `SessionRecord`, `LaunchRecord`, `LaunchState`, `LaunchResult`, `RegistryLock`, and `RegistryTransfer`, plus `pub enum OwnershipMode { Direct, Systemd }`.
- Produces private `SessionClaimRequest { session: SessionId, entry_pid: u32 }`, `PrepareLaunch { launch: LaunchId, mode: OwnershipMode }`, and `VerifiedOwnership`.
- Produces private `ActivationRegistry::open(state_home: &Path) -> Result<Self, String>`, `claim_session(request: SessionClaimRequest) -> Result<SessionClaim, String>`, `open_active_session(session: SessionId, expected_sequence: u64) -> Result<ActiveSessionCapability, String>`, and `prepare(session: &ActiveSessionCapability, request: PrepareLaunch, selection: &GenerationSelection) -> Result<PreparedLaunch, String>`.
- A future public session-facing facade is out of this core PR.  It can be introduced only alongside a concrete verifier/launcher implementation; this PR never accepts caller-constructed ownership or session identity evidence.
- `open` scans the whole bounded inventory while holding `lifecycle.lock`: it discards only exact, safe, producer-shaped unpublished temporaries and fsyncs their parents; malformed/reserved/unsafe/over-bound entries are retained and make every destructive operation fail closed.  It never silently accepts a pre-existing partial inventory.

- [ ] **Step 1: Write failing initialization/parser tests**

Add tests for: absent absolute state home creates `helm/activation`, `launches`, and one zero-length mode-0600 `lifecycle.lock`; absent, empty, or relative state home is rejected without mutation; a symlink/wrong-mode lock is rejected without replacement; a claimed session independently revalidates the current UID, Linux boot ID, and exact `/proc` PID/start time before final publication and can be re-opened only through its exact canonical final record; valid direct and systemd `preparing` records parse; duplicate field, CR/no-final-LF, oversized record, wrong direct group, non-exact systemd scope name, and systemd non-`none` pre-adoption incarnation reject.  Add exact-name temporary fixtures proving empty/non-UTF8 producer-shaped artifacts are discarded, while a bad reserved name/type/mode or an entry/byte bound is retained and fails closed.

```rust
#[test]
fn preparing_direct_record_requires_owner_process_group() {
    assert!(LaunchRecord::parse(direct_preparing("process-group 0")).is_err());
}
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p helm-theme lifecycle::tests::preparing_direct_record_requires_owner_process_group -- --nocapture`

Expected: FAIL because `lifecycle` and `LaunchRecord` do not exist.

- [ ] **Step 3: Implement the minimal root, lock, and parser**

Reuse `GenerationRoot`-style descriptor helpers rather than path-based `std::fs` mutation. Create parent/child directory fsync boundaries exactly as SPEC0012 §2 requires. Implement no-replace session claim and launch-create publication plus exact temporary inventory/recovery before exposing any claim/launch operation. Write both canonical grammars in their stated field order. `open_active_session` only reopens and revalidates an already durable canonical `active` session record; this core does not fabricate that state or fake helper evidence. A later session-bootstrap tranche is its sole producer. `prepare` requires that sealed capability, copies generation/manifest/process-lease identity from the held selection, and writes a gate-closed final launch record. `preparing` stores the immutable selected mode: direct has owner PID process group and no unit/cgroup identity; systemd stores its deterministic scope name and zero/`none` incarnation fields.

- [ ] **Step 4: Verify green**

Run:

```bash
cargo test -p helm-theme lifecycle::tests -- --nocapture
cargo clippy -p helm-theme --all-targets --all-features -- -D warnings
```

Expected: PASS with no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs crates/helm-theme/src/generation/lifecycle.rs
git commit -m "feat: add lifecycle registry record core"
```

### Task 3: Implement linear process-to-lifecycle lease transfer

**Files:**
- Modify: `crates/helm-theme/src/generation.rs`
- Modify: `crates/helm-theme/src/generation/lifecycle.rs`
- Test: `crates/helm-theme/src/generation/lifecycle.rs` `#[cfg(test)]`

**Interfaces:**
- Produces private `ActivationRegistry::adopt_prepared(session: &ActiveSessionCapability, prepared: PreparedLaunch, selection: GenerationSelection, evidence: VerifiedOwnership) -> Result<AdoptedLaunch, String>`.
- Produces private `VerifiedOwnership::{Direct, Systemd}` only from the sealed child-module `OwnershipVerifier`; each variant carries the exact prepared launch id, owner PID/start-time/UID/boot identity, selected mode and deterministic unit/process group binding as well as its mode-specific proof. Production construction is intentionally absent from this core and test construction is confined to `#[cfg(test)]` fixtures; evidence for launch A must be rejected for launch B even when its invocation/cgroup spelling is otherwise canonical.
- Produces private parent-module `GenerationSelection::replace_with_lifecycle_locked(record: LifecycleLeaseRecord) -> Result<(), String>` and `GenerationSelection::disarm_after_transfer()`; the child module accesses them through ordinary parent-private visibility, and no public/crate-visible bridge or token exists.
- Consumes the exact `PreparedLaunch` final-record identity/sequence, an `ActiveSessionCapability` whose on-disk session id/sequence remains exact, and selection metadata; it fails without mutation if any session, launch, generation, manifest, process-lease, or expected-sequence field differs.
- Produces private `enum TransferCheckpoint { DuringStageWrite, BeforeReplace, BeforeExchange, AfterReplace, AfterLeaseDirectoryFsync, BeforeAdoptedRecord, BeforeSelectionDisarm }` and test-only `LeaseFilesystem` callbacks at each checkpoint. `DuringStageWrite` occurs after only a strict prefix has reached the still-unnamed `O_TMPFILE`, so a real-process exit there proves partial bytes never become a named artifact.

- [ ] **Step 1: Write red transfer-boundary fixtures**

Use an injected `LeaseFilesystem` checkpoint enum with `DuringStageWrite`, `BeforeReplace`, `BeforeExchange`, `AfterReplace`, `AfterLeaseDirectoryFsync`, `BeforeAdoptedRecord`, and `BeforeSelectionDisarm`. For each fault, assert: the generation always has either the exact process lease or exact lifecycle lease; pre-replace drop releases only the process lease; post-replace drop leaves lifecycle lease present; no `adopted` record precedes replacement+fsync. Add negative cases for wrong session claim, stale expected sequence, a second prepared launch id, absent/cross-kind/mismatched process lease, and a valid lifecycle lease whose record identity mismatches. Task 4 owns all reopen/recovery-row assertions.

Also simulate a real pre-rename process crash that bypasses Rust error/drop
cleanup at both sides of stage publication. A pre-link crash while the
lifecycle bytes are being prepared in an unnamed `O_TMPFILE` must leave only
the original process target and no named staging artifact. A post-link crash
must retain the original process target plus the exact fixed
`.lease-transfer-<lease-id>` staging artifact; reopen under `activation.lock`
must prove whole-inventory validation removes/fsyncs only its lifecycle staging
half before retry. Also inject a crash after `RENAME_EXCHANGE`: recovery must
recognize the exact lifecycle-target/process-stage pair as transferred, fsync
it, then remove/fsync only its displaced process half. Unsafe, missing,
same-kind or mismatched staging/target evidence must retain the entire inventory
and fail closed. Add a post-final-validation pathname-swap fixture: rollback is
allowed only when descriptor/content proof establishes the exact inverse pair
and restores the held source inode; otherwise retain an ambiguous result. This
staging recovery belongs to Task 3; Task 4 still owns only post-lease-fsync
record/actual-lease rows.

```rust
assert!(process_lease_exists || lifecycle_lease_exists);
assert_ne!(process_lease_exists, lifecycle_lease_exists);
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p helm-theme lifecycle::tests::transfer_never_leaves_an_unleased_generation -- --nocapture`

Expected: FAIL because no registry transfer operation exists.

- [ ] **Step 3: Implement private linear transfer**

Acquire one `RegistryLock`, revalidate the active session capability and `PreparedLaunch` sequence under it, then take the selection's cloned generated-root `activation.lock` shared lock through its original `Arc<Mutex<()>>`. Revalidate exact process lease/selection identity, generation, manifest and sealed ownership evidence. Atomically replace the same lease name with canonical lifecycle bytes and fsync the lease directory. The selection cleanup guard must parse/revalidate before every unlink, so an unwind before disarm is safe. Atomically replace/fsync the matching launch record as `adopted`, `lease-kind lifecycle`, with verified direct/systemd fields. Return an internal `AdoptedLaunch` with no raw lease-release method. Task 4 owns post-lease-fsync recovery assertions.

Before publishing the fixed transfer staging file, validate the complete lease
inventory and recover only SPEC 0011's two exact canonical paired states.
Prepare canonical lifecycle bytes in a mode-0600 unnamed `O_TMPFILE`, file
fsync and descriptor/content validate them, then atomically publish that exact
inode with one no-replace `linkat`: `AT_EMPTY_PATH` where permitted, otherwise
the descriptor-verified `/proc/self/fd/<held-fd>` source with
`AT_SYMLINK_FOLLOW`. `/proc` or link failure fails closed; no rename or second
named stage is allowed. Revalidate the published pathname's inode/device,
current UID, exact mode, bounded canonical content and equality to the held
bytes before fsyncing the leases directory.
Only after publication use `RENAME_EXCHANGE`, held source/staging descriptors,
post-exchange identity and content proof, directory fsync, then
displaced-process cleanup/fsync; plain overwriting rename is forbidden. Never
treat a staging payload by itself as an adopted lease. Generic GC applies the
same whole-inventory proof before any stale-lease or generation deletion.

- [ ] **Step 4: Verify green**

Run:

```bash
cargo test -p helm-theme lifecycle::tests::transfer_never_leaves_an_unleased_generation -- --nocapture
cargo test -p helm-theme lifecycle::tests::post_replace_drop_retains_lifecycle_lease -- --nocapture
cargo test -p helm-theme --lib --quiet
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs crates/helm-theme/src/generation/lifecycle.rs
git commit -m "feat: transfer generation leases through lifecycle registry"
```

### Task 4: Add conservative registry reconciliation with fake proof adapters

**Files:**
- Modify: `crates/helm-theme/src/generation/lifecycle.rs`
- Test: `crates/helm-theme/src/generation/lifecycle.rs` `#[cfg(test)]`

**Interfaces:**
- Produces sealed child-module `OwnershipInspector` with `inspect_systemd(&VerifiedSystemd) -> SystemdObservation` and `inspect_direct(&LaunchRecord) -> DirectObservation`.
- Produces sealed child-module `GateClosedController::{abort_unadopted_direct, abort_unadopted_systemd, abort_adopted_systemd}`; no external caller can manufacture it.  It receives the exact owner PID/start/UID/boot and deterministic direct group or systemd unit, and must return a fresh exact ownership observation after a bounded gate-closed abort/reap attempt.
- Produces private `ActivationRegistry::reconcile(inspector: &impl OwnershipInspector, controller: Option<&impl GateClosedController>) -> Result<ReconciliationReport, String>`.
- `SystemdObservation` distinguishes exact recursive-empty, exact-live, and every invocation/cgroup/path/device/inode/permission/parser uncertainty. `DirectObservation` independently carries `owner_written_witness`, exact-owner-stale, recorded-group-empty, live, and uncertainty; there is no generic `Empty` release token.

- [ ] **Step 1: Write red reconciliation tests**

Test exact rows: `preparing` plus matching process lease for direct and deterministic systemd ownership invokes only the matching gate-closed controller, proves emptiness, writes `terminal/failed`, then releases the process lease; `preparing` plus matching lifecycle systemd lease and `adopted` systemd rows use only their exact controllers; running systemd empty becomes `terminal/exited` without a controller call; direct `running` without owner-written `direct-drained yes` stays record+lease even if fake group is empty; terminal direct releases only with witness, exact owner stale, and exact recorded group empty; cross-kind/malformed/missing/mismatched lease, unavailable controller, or any observation uncertainty retains all state. Include post-lease-fsync/pre-adopt and terminal-fsync/pre-release/release-fsync/pre-record-removal fault fixtures; reopening must retry only the applicable row and prove final terminal record collection after lease-directory fsync.

```rust
assert_eq!(report.released, 0);
assert!(launch_record_exists(&root, launch));
assert!(lifecycle_lease_exists(&generated, lease));
```

- [ ] **Step 2: Verify red**

Run: `cargo test -p helm-theme lifecycle::tests::direct_owner_death_without_witness_is_retained -- --nocapture`

Expected: FAIL because reconciliation does not exist.

- [ ] **Step 3: Implement only accepted proof transitions**

Dispatch record/actual lease through SPEC0012 §5. Use `GateClosedController` only before profile execution: matching process-lease `preparing` direct/systemd ownership and matching lifecycle-lease `preparing`/`adopted` systemd rows. Never stop a running systemd profile scope; it may only collect after `OwnershipInspector` returns exact recursive-empty with matching incarnation. For direct, only an already durable `terminal/exited direct-drained yes` plus exact stale owner and exact recorded-group empty releases. Any malformed/cross-kind/mismatch/inspector uncertainty returns retained evidence without unlinking. After each permitted terminal record fsync, unlink/fsync the exact lease, then remove/fsync the exact terminal launch record; injected failures leave the matching retry state intact.

- [ ] **Step 4: Verify green and run full repository checks**

Run:

```bash
cargo test -p helm-theme lifecycle::tests -- --nocapture
cargo test --workspace --all-features --locked --quiet
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

Expected: every command exits 0.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation/lifecycle.rs
git commit -m "feat: reconcile lifecycle leases conservatively"
```

## Spec coverage review

- SPEC0012 §§2/4 record durability and lock order: Tasks 2–3.
- Linear transfer/no stale-drop/unleased fault boundaries: Tasks 1 and 3.
- SPEC0012 §5 authority/result/proof-before-release: Task 4.
- Direct owner-only drain witness and forced-logout retention: Task 4.
- Systemd recursive-proof interface without unauthorized profile stop: Task 4.
- Actual systemd D-Bus adapter, profile execution/gate, session helper lifecycle,
  and public status transport remain separate implementation plans; this plan
  does not claim them complete.

## Plan self-review

- No omitted implementation steps or deferred code instructions.
- Later task interfaces are defined by earlier task boundaries.
- Every mutation has a red test, focused green test, and a reviewable commit.
