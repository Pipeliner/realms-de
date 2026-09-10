# ADR 0020 — Transfer recovery bounds retained descriptors per pair

- **Status:** Accepted (2026-09-01)
- **Deciders:** realm maintainers, repo owner
- **Supersedes / Superseded by:** Supplements [ADR 0017](0017-immutable-theme-activation-generations.md) and accepted [SPEC 0012](../specs/0012-activation-launch-lifecycle.md).

## Context

The lifecycle transfer-recovery classifier admitted a 4096-entry inventory but
kept an `OwnedFd` for every canonical and staging entry until normalization.
On a normally bounded process this can exhaust `RLIMIT_NOFILE` before the
classifier reaches the required 4097th-entry bounds rejection.  PR #205's
native package test reproduced that failure as `EMFILE`.

Closing those descriptors and caching `(st_dev, st_ino)` is not safe.  A
pathname can be unlinked and its inode reused before recovery unlinks the
staging name; a cached numeric identity can then authorize deletion of the
replacement.  SPEC 0012 requires descriptor-backed exact-pair proof for
transfer staging.  The normalizer must also retain the existing cooperative
writer boundary: writers hold `lifecycle.lock` and hostile same-UID mutation
after a final pathname proof is outside the ordinary unlink-by-descriptor
guarantee.

## Decision

1. Initial transfer-staging classification validates the complete bounded
   inventory and retains only parsed records and deterministic target names,
   never one descriptor per entry.
2. Normalization processes targets in lexical order.  For each target it
   rescans the complete bounded inventory without mutation, validates every
   entry, and retains descriptors only for that canonical target and its
   `.lease-transfer-<target>` sibling.
3. It requires the exact process/lifecycle inverse pair, revalidates both
   pathnames against their still-open descriptors, fsyncs the directory,
   unlinks only that staging pathname, then fsyncs again before dropping the
   pair descriptors.
4. No FD-free device/inode cache may become deletion authority.  A later scan
   failure stops further normalization; under the established cooperative lock
   model each completed unlink had a complete fresh scan and exact held-pair
   proof.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Retain every inventory descriptor | Existing exact-inode proof already worked | Violates the declared bound under ordinary FD limits. |
| Raise `RLIMIT_NOFILE` or lower the 4096-entry limit | Small local change | Makes portability or the accepted logical capacity worse instead of fixing authority lifetime. |
| Cache `(st_dev, st_ino)` after classification | Constant FD use and one scan | Inode reuse makes a replacement numerically indistinguishable after the original FD closes. |
| Full scan per pair while holding that pair | Constant descriptor use and preserves exact selected objects through unlink | Recovery work is O(pairs × inventory), acceptable for rare crash reconciliation. |

## Consequences

### Good

- The 4096-entry / 16-MiB inventory contract is enforceable under low FD limits.
- Each destructive unlink remains backed by live descriptors for its exact pair.
- Deterministic target order and complete scans keep directory enumeration from changing authority.

### Bad

- Many crash-staging pairs require repeated full scans.
- A hostile mutation discovered only on a later scan cannot restore an earlier,
  already-proven unlink; that is the same post-proof hostile-same-UID boundary
  already recorded by SPEC 0012.

## Reversal

This changes `generation.rs` classifier/normalizer data ownership and its
lifecycle fixtures.  Reconsider if Linux gains a portable unlink-by-descriptor
primitive or recovery inventories become large enough that bounded repeated
scans are demonstrably unacceptable.

## Guard

`generation::lifecycle::tests::transfer_stage_classifier_rejects_over_bound_inventory`
must return the bounds error at `RLIMIT_NOFILE=256`, never `EMFILE`.  Lifecycle
fixtures must also prove recovery retains a pathname replacement and that a
4096-entry valid staging inventory normalizes under that limit.

## Needs a human

No further decision is required: the owner authorized this review path and did
not request a correction.
