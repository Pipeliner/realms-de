# SPEC 0011 — Immutable theme activation generations

- **Status:** Accepted (2026-08-29)
- **Milestone:** M1
- **Decision:** [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Issue:** [#131](https://github.com/Pipeliner/realms-de/issues/131)

## Purpose

Make each launcher decision refer to one sealed, digest-bound theme generation,
not a mutable output directory.

## Behaviour

`generation-id` is a collision-resistant opaque ASCII identifier. Helm creates
its owned generated root with mode 0700. A valid tree contains only normalized
relative output paths and a versioned canonical manifest with lexicographically
sorted paths, exact palette/catalogue/templates/rendering/launch-profile/output
digests, and a seal covering that manifest. It rejects symlinks, traversal,
duplicates, special files and external paths at publication and consumption.
All participants acquire the same descriptor-relative no-follow
`activation.lock` inode; writers use its exclusive advisory lock for staging,
seal, pointer replacement, recovery and GC, and launchers use its shared lock
through pointer resolution, manifest validation and fsynced lease creation.
Kernel release after process death is stale-writer recovery; a live lock is
never stolen or replaced by deleting its pathname. The launcher then passes the selected generation path explicitly
to target dependency evaluation.

No read or write follows symlinks; all operations are descriptor-relative below
Helm's owned generated subtree. `current` cannot name absent, staging, malformed
or digest-mismatched content. An apply that cannot finish before pointer commit
leaves `current` unchanged. A successful pointer commit is the only event that
may make a generation active for future launches. It must not signal or reload
an existing process. A direct launch outside a verified Helm profile is not
generation-selected.

## Acceptance criteria

| # | Given / When / Then |
|---|---|
| G1 | Given a selected old generation, when a newer generation is committed, then a launch using the old selection reads only old manifest-listed bytes. |
| G2 | Given interruption before pointer commit, when recovery runs, then `current` still names the previous valid sealed generation and staging is not launchable. |
| G3 | Given interruption after tree seal but before pointer replacement, when recovery runs, then the sealed tree is retained or safely collectible and `current` remains valid. |
| G4 | Given concurrent applies, when both complete, then every committed pointer refers to one complete manifest/tree pair and neither mixes palette/catalogue/target inputs. |
| G5 | Given a malformed, absent or digest-mismatched manifest, when a launcher resolves `current`, then it refuses before spawning a target. |
| G6 | Given a live lease, when GC runs, then its generation is retained; given a stale PID, boot-ID or start-time lease, then GC reclaims only the lease and unleased old generation candidates; on uncertain liveness or a malformed sealed tree, it retains rather than deletes. |
| G7 | Given rollback to a retained generation, when it commits, then future launches select it and already-running launches remain unchanged. |
| G8 | Given a corrupt pointer, missing tree, or digest mismatch, when recovery or launch runs, then it fails closed and never selects an arbitrary/newest generation. |
| G9 | Given concurrent applies, when both are accepted, then they serialize from input capture through pointer durability and each receipt identifies its committed generation. |

## Boundaries

This specification does not make arbitrary user configuration Helm-owned, does
not promise a live process upgrade, and does not settle systemd/no-systemd
teardown, scope adoption, D-Bus ownership, or target package sources. Those are
#117, #132, #133 and #134. #22 supplies the broader apply/reload transaction.
