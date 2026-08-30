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

Registry construction receives a private capability derived from an already
validated `GenerationStore`. The capability clones the store's `leases/` and
`activation.lock` descriptors and shares its original in-process mutex. It
does not contain a path reconstructed from activation state. Restart
bootstrap must open and revalidate the generated store first; the registry can
then reconcile launch-record lease names only below that descriptor-held root.
All reconciliation holds `lifecycle.lock` before taking the capability's
shared generation lock, matching transfer and generic-GC serialization.
Each `GenerationSelection` carries the exact leases-directory and
activation-lock device/inode identities from its originating store. Both
`prepare_launch` and adoption revalidate and compare both identities with the
registry capability before any mutation; selections from another store are
never accepted by byte or path coincidence. A consumed rejected selection is
destructor-disarmed without touching its foreign process lease; its originating
store retains cleanup authority.

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
The cleanup read is bounded at 4097 bytes so records exceeding the 4096-byte
contract are retained without an unbounded allocation. Cleanup uses the same
cooperative Helm-writer proof-to-unlink boundary as retirement: pre-/at-proof
swaps retain, while hostile same-UID post-proof mutation is outside M2 and no
unlink-by-fd property is claimed. Cleanup itself acquires the selection's saved
originating-store mutex and shared activation lock for that proof/unlink span.

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

Reconciliation uses a whole-inventory three-phase protocol. It first validates
all record/lease relations and retirement forms and gathers every passive
ownership observation. Fatal or uncertain evidence freezes the pass before
controllers or durable mutation. It then runs only exact gate-closed abort
controllers; those bounded abort attempts are permitted before the final
global controller result is known, but no record/lease mutation occurs unless
all results prove empty. Only the final phase first applies a read-only
classified safe registry-temporary and transfer-staging normalization plan and
then persists transitions and retirements. A safe temporary or valid stage
never changes when an unrelated fatal/passive/controller classification freezes
the pass. Inventory entry and byte limits are enforced incrementally before
another name/descriptor/payload is retained in memory. Missing leases are state-sensitive: `preparing` is an abort and
record-collection row, `adopted`/`running` is fatal, and `terminal` is a
post-release collection retry.

Direct `terminal/failed`, `lease-kind process`, `exec-open no`,
`direct-drained no` with an absent lease is a separate producer-reachable
pre-exec crash row: a fresh exact gate-closed abort/empty proof permits launch
collection without a lifecycle drain witness. Other direct terminal/absent
rows retain the lifecycle witness rule.

The first phase always scans the complete lease directory, even when no staging
name exists. It rejects unknown/non-UTF-8 names, unsafe or malformed
unreferenced entries, orphan lifecycle leases, and duplicate launch references
to one lease before any observation or controller call. Valid unreferenced
process leases remain generic-GC evidence and are outside registry authority.
Lease retirement requires its sole exact durable `terminal` record. Launch
retirement requires the referenced lease to be absent. Either reversed ordering
is non-producer-reachable and freezes before adapters.

Lease and terminal-record deletion use fixed recoverable retirement siblings,
not revalidate-then-unlink. A no-replace rename to the retirement name is the
linearization point; the moved inode is post-validated and only that name may
be unlinked. Same-name replacements and all malformed/duplicate retirement
states are retained in the detecting pass. A later fresh pass may retire
semantically identical canonical evidence only after repeating the whole proof.
A crash with only an exact retirement form resumes the same proof and cleanup.
Stronger permanent inode preservation needs durable provenance and is deferred.
Transfer-staging failure is an explicit frozen error,
even with an otherwise empty registry.

Preparation reserves a launch id across both canonical and fixed
`.launch-retire-<id>` forms. Either collision rejects before mutation.

M2 assumes conforming same-UID Helm writers respect the lock order and reserved
retirement namespace. Atomic retirement and post-move proof cover races before
or at the move. Hostile same-UID mutation after retirement proof is explicitly
outside the account-compromise boundary; the design claims no unlink-by-fd
property that Linux cannot provide.

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
- exact final-gap lease and launch swaps, retirement crash recovery, whole-pass
  ownership uncertainty in both enumeration orders, state-sensitive absent
  leases, direct-detachment terminal retention, and empty-registry staging
  failure reporting;
- restart reconciliation through the descriptor-derived lease capability,
  proving that activation records cannot redirect cleanup to a config path.
- cross-store selection rejection at prepare and adopt; direct pre-exec
  terminal/missing restart collection; valid staging plus unrelated fatal
  evidence causing zero staging mutation; bounded selection cleanup with a
  pre-proof swap retained; and canonical/retired launch-id collision rejection.

The first implementation PR must stop at a testable registry core with fake
ownership adapters.  It must not claim real D-Bus activation, profile launch,
or user-manager functionality until #133/#135 and their executable evidence
are complete.

## Non-goals

- No public wire protocol or history store.
- No placeholder `helm-session` crate without the red fixtures above.
- No cleanup policy that trades uncertain direct evidence for quota recovery.
