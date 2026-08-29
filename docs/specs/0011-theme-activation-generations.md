# SPEC 0011 — Immutable theme activation generations

- **Status:** Accepted (2026-08-29)
- **Milestone:** M1
- **Decision:** [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Issue:** [#131](https://github.com/Pipeliner/realms-de/issues/131)

## Purpose

Make each launcher decision refer to one sealed, digest-bound theme generation,
not a mutable output directory.

## Behaviour

`generation-id` is a collision-resistant opaque 128-bit lowercase-hex identifier. Helm creates
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

## Generation manifest v1 (L4)

Each sealed generation directory contains exactly the regular files named by
`output` records plus three reserved regular files: `manifest`, `receipt` and
`seal`.
Every directory entry is either one of those files or a directory that is a
non-empty proper prefix of at least one `output` path. No empty, unlisted,
special, staging, or symlink directory/file is valid.
`manifest` is UTF-8 and uses this byte-exact grammar (every line ends in one
LF; no CR, blank line, comment, duplicate field, or extra record is allowed):

```text
helm-generation-manifest-v1\n
generation <generation-id>\n
palette-sha256 <lowercase-64-hex>\n
catalogue-sha256 <lowercase-64-hex>\n
templates-sha256 <lowercase-64-hex>\n
renderer-sha256 <lowercase-64-hex>\n
launch-profile-sha256 <lowercase-64-hex>\n
receipt-sha256 <lowercase-64-hex>\n
output <normalized-relative-path> <lowercase-64-hex>\n
... one or more output records, strictly lexicographically sorted by path
```

The header and seven singleton records appear once in the listed order. An
`output` record follows them, names a non-empty path matching
`[A-Za-z0-9._/-]+`, and contains the SHA-256 of exactly that regular file's
bytes. It has neither a leading/trailing slash nor `//`; every component is
non-empty and not `.` or `..`. Output records are strictly sorted by the
unsigned lexicographic order of their ASCII path bytes. Paths cannot be
`manifest`, `seal`, `activation.lock`, `.staging-<generation-id>`, a duplicate,
or a prefix collision. A generation identifier must equal its containing final
directory name and match `[a-f0-9]{32}`; a producer retries with fresh random
128-bit values if a final directory already exists.

Each singleton digest is lowercase SHA-256 hex. It is calculated over these
byte-exact preimages, captured before rendering: `palette` is the raw palette
file bytes; `catalogue` is the UTF-8 sequence of all template records sorted by
the unsigned lexicographic order of their UTF-8 template-id bytes, each record
being `<decimal-byte-length>:<id-bytes><decimal-byte-length>:<target-bytes><decimal-byte-length>:<reload-kind-bytes>\n`;
`templates` is the same sorted record sequence but each record is
`<decimal-byte-length>:<id-bytes><decimal-byte-length>:<template-body-bytes>\n`;
`renderer` is UTF-8 `helm-theme-renderer-v1\n` followed by records for every
render option sorted by the unsigned lexicographic order of its UTF-8 name
bytes, each
`<decimal-byte-length>:<name-bytes><decimal-byte-length>:<value-bytes>\n`;
`launch-profile` is the raw selected launch-profile bytes. Every decimal length
is ASCII without leading zeroes except `0`, and is a byte count rather than a
character count. Template IDs, targets, reload kinds and renderer option names
must be valid UTF-8 and contain neither NUL nor CR/LF; an input that violates
that rule is refused before a receipt is formed. Template IDs and renderer
option names must each be unique; duplicates are refused before sorting.
Template bodies, option values, palette and launch-profile inputs use their
exact raw bytes.

`receipt` is UTF-8 with this byte-exact grammar: header
`helm-generation-receipt-v1\n`, then `generation <generation-id>\n`, then the
five input-digest records in the same order and spelling as `manifest`. It has
no other record or byte. Its generation identifier and five digest values must
equal those in `manifest`; `receipt-sha256` is the SHA-256 of its complete byte
sequence. The writer fsyncs `receipt`, then `manifest`, then `seal`, and only
then fsyncs the sealed generation directory and performs the pointer commit. A
consumer verifies this association but need not reconstruct the producer input
preimages.

`seal` is exactly the lowercase SHA-256 hex digest of the complete `manifest`
byte sequence followed by one LF. A manifest whose seal, grammar, identity,
tree membership or output digest is invalid is malformed and cannot be
selected.

The generation root and each directory/file component are opened
descriptor-relatively with `O_NOFOLLOW`; opening a final file also requires a
regular file descriptor. Validation retains the opened root/parents while it
reads each listed output. This protects Helm from pathname races and symlink
redirection, but does not expand ADR 0017's same-UID threat model.

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
