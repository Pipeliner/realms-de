# M2 lifecycle registry design

**Status:** Candidate implementation refinement for accepted SPEC 0012

## Purpose

Implement the smallest safe M2 component that converts an already durable
generation process lease into a lifecycle lease without ever creating an
unleased owner or allowing launcher-side drop cleanup to remove the transferred
lease.  The component owns only durable registry records, linear transfer, and
conservative reconciliation.  It does not choose profile argv/assets, D-Bus
activation, existing-owner handling, River replay, or public lifecycle DTOs.

## Boundaries

The registry is the sole owner of:

- descriptor-relative activation-root initialization and `lifecycle.lock`;
- canonical launch-record parsing/publication under that lock;
- consuming a `GenerationSelection` into one lifecycle lease;
- terminal/recovery classification and proof-before-release.

`helm-theme` remains the sealed-generation and generic-GC owner.  Generic GC
only retains lifecycle evidence; it does not query, create, replace, or release
registry leases.  Systemd and direct process observations live behind adapters;
their accepted proofs in SPEC 0012 remain the authority.

## Linear transfer contract

`prepare_launch` first persists a gate-closed `preparing` record.  The registry
then receives the unique process selection and verified ownership evidence.  A
single private transfer operation:

1. takes `lifecycle.lock`, then SPEC 0011's shared generation lock;
2. revalidates the process lease, `preparing` record, generation, manifest,
   owner identity and ownership evidence;
3. atomically replaces the same lease name with the lifecycle grammar and
   fsyncs the lease directory;
4. consumes/disarms the source selection only after that fsync; and
5. persists `adopted` before authorization can open the owner gate.

There is no public transfer capability, raw lifecycle-lease replacement, or
post-transfer `GenerationSelection::release`.  A failed pre-replace operation
leaves the source selection armed with its process lease.  Any post-replace
failure is recovered from the record/lease table and is never repaired by
dropping the old selection.  The private handoff guard changes the selection's
drop state only after the replacement fsync, so an error or unwind on either
side of that boundary cannot unlink the same-named transferred lease.  Its
cleanup first reopens and parses the opaque filename and unlinks only an exact
matching process lease; lifecycle, malformed, unreadable or mismatched content
is conservatively retained.  That discriminator-and-identity guard closes the
replacement→fsync→disarm unwind window.

The selected ownership mode is already canonical in `preparing`: direct uses
the exact owner PID as process group and no unit/cgroup evidence; systemd uses
the deterministic scope name and zero/`none` incarnation fields.  Only the
verified systemd invocation/cgroup fields are filled during `adopted`; the
selected mode and every identity common to the process lease are immutable.

## Terminal authority

The registry's terminal result is internal only.  Systemd reconciliation writes
`failed` only after a permitted gate-closed pre-exec abort plus recursive proof,
and writes `exited` after a running independent scope reaches recursive
emptiness.  It has no authority to stop a running profile scope.  The direct
lifetime owner alone writes `exited` and `direct-drained yes` after
reaping descendants.  A detected direct detachment may become retained
`terminal/lost`; owner death, forced termination, or an absent witness remains
nonterminal and retained.  External teardown may request cooperative direct
completion, but it cannot forge a witness, terminal transition, lease release,
or collection after killing the owner.

## Verification obligations

Red-first tests must cover:

- every owner-create/process-lease/preparing/adoption/transfer/adopted/exec/
  running/terminal/release/collection crash boundary;
- source-selection drop before and after the lifecycle replacement, proving no
  unleased state and no post-transfer unlink by stale release authority;
- canonical record fields and transition-sequence/authority checks;
- systemd unit/invocation/cgroup recursive-population proof and identity races;
- direct normal self-drain, forced logout, owner death, and detachment retention;
- cross-kind, missing, malformed and identity-mismatched record/lease recovery.

The first implementation PR must stop at a testable registry core with fake
ownership adapters.  It must not claim real D-Bus activation, profile launch,
or user-manager functionality until #133/#135 and their executable evidence
are complete.

## Non-goals

- No public wire protocol or history store.
- No placeholder `helm-session` crate without the red fixtures above.
- No cleanup policy that trades uncertain direct evidence for quota recovery.
