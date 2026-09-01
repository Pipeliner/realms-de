# Bounded Transfer Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make lifecycle transfer recovery enforce its accepted 4096-entry bound under normal FD limits without weakening descriptor-backed deletion authority.

**Architecture:** Classify the entire inventory into parsed records and ordered staging names without retaining every descriptor.  For each staging target, rescan and retain only that target/stage pair through final revalidation and unlink.  This follows ADR 0020 and retains the current cooperative-writer / post-proof hostile-mutation boundary.

**Tech Stack:** Rust 2021, `rustix` descriptor-relative filesystem APIs, existing in-file lifecycle fixtures.

**Spec:** `docs/specs/0012-activation-launch-lifecycle.md`; `docs/adr/0020-bounded-transfer-recovery-descriptors.md`

## Global Constraints

- Keep the 4096-entry and 16-MiB limits; never solve this by raising process limits.
- Do not use FD-free inode identity as unlink authority.
- Validate every entry in every recovery scan before unlinking the selected staging entry.
- Tests must demonstrate the red low-FD failure and secure replacement retention before implementation.

---

### Task 1: Encode the low-FD and recovery-normalization regressions

**Files:**
- Modify: `crates/helm-theme/src/generation/lifecycle.rs`
- Test: `crates/helm-theme/src/generation/lifecycle.rs` `generation::lifecycle::tests`

**Interfaces:**
- Produces a test-only low-FD child-process helper that runs the existing
  4097-entry classifier fixture with `RLIMIT_NOFILE=256`.
- Produces a recovery normalization replacement fixture that proves a replaced
  staging path is retained.

- [ ] **Step 1: Make the existing classifier fixture fail under a 256-FD process limit**

Run:

```bash
ulimit -n 256
cargo test -p helm-theme generation::lifecycle::tests::transfer_stage_classifier_rejects_over_bound_inventory -- --exact
```

Expected: FAIL with `Too many open files`, proving the regression is capacity
exhaustion rather than a malformed-inventory assertion.

- [ ] **Step 2: Add a child-process low-FD regression and a recovery replacement fixture**

The child must run the real 4097-entry classifier setup at limit 256 and assert
the result contains `bounds`.  The recovery fixture must replace the selected
staging pathname between classification and final revalidation and assert that
the replacement remains present after recovery errors.

- [ ] **Step 3: Verify red**

Run the new focused tests.  Expected: the low-FD test reports `EMFILE` before
the implementation changes; the replacement fixture exposes any authority
that lacks a still-held selected descriptor.

### Task 2: Retain descriptors only for the selected recovery pair

**Files:**
- Modify: `crates/helm-theme/src/generation.rs:214-263,2389-2465`
- Test: `crates/helm-theme/src/generation/lifecycle.rs`

**Interfaces:**
- `classify_lease_transfer_staging_locked` returns ordered parsed records and
  staging target names without retaining inventory FDs.
- A private selected-pair full scan returns only two
  `ValidatedLeaseInventoryEntry` descriptors after validating the complete
  inventory and exact inverse relationship.
- `LeaseTransferRecoveryPlan::normalize` performs a selected-pair full scan,
  final descriptor/path revalidation, fsync/unlink/fsync, and repeats in lexical
  order.

- [ ] **Step 1: Replace all-entry descriptor maps with record-only inventory state**

Keep canonical and staging parsed records in `BTreeMap<String, ParsedLeaseRecord>`.
Preserve existing bounds, no-follow, ownership, canonical-payload, unknown-name,
and exact-pair checks while dropping each unrelated descriptor immediately.

- [ ] **Step 2: Implement the selected-pair full scan**

For a requested target, scan every entry and use the existing validated reader.
Drop unrelated descriptors immediately; retain only the target and its staging
sibling.  At EOF require both entries and the exact process/lifecycle inverse
pair.  Do not use cached device/inode values.

- [ ] **Step 3: Normalize one proven pair at a time**

For each lexically ordered target, invoke the selected-pair scan, revalidate
both pathnames with its held descriptors, fsync the leases directory, unlink
only the staging name, and fsync again.  An error stops later pairs.

- [ ] **Step 4: Verify green**

Run:

```bash
cargo test -p helm-theme generation::lifecycle::tests::transfer_stage_classifier_rejects_over_bound_inventory -- --exact
cargo test -p helm-theme generation::lifecycle::tests -- --nocapture
cargo test -p helm-theme
```

Then run the native package driver that originally failed, and push only after
the PR’s remote package workflow has passed.
