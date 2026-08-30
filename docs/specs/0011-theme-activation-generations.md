# SPEC 0011 — Immutable theme activation generations

- **Status:** Accepted (2026-08-29)
- **Milestone:** M1
- **Decision:** [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Issue:** [#131](https://github.com/Pipeliner/realms-de/issues/131)
- **Contract reconciliation:** [#159](https://github.com/Pipeliner/realms-de/issues/159)
- **Implementation refinement:** publication-order record v1 (2026-08-29)

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

Two canonical lease kinds exist. A process lease uses the existing
`helm-generation-lease-v1` grammar and generic GC may unlink it only after
proving its PID, boot ID, or start time stale. Before exec, a process lease may
be atomically replaced at the same opaque lease name with the
`helm-generation-lifecycle-lease-v1` grammar from [SPEC 0012](0012-activation-launch-lifecycle.md).
GC parses the discriminator before assessing liveness. It never PID-reclaims a
valid lifecycle lease: it validates its launch reference against the lifecycle
registry and retains its generation until the lifecycle reconciler has durably
proved the complete recorded scope or process group empty and unlinked the
lease. A missing, malformed, cross-kind, identity-mismatched, or unreadable
lifecycle record/evidence is uncertain: GC unlinks neither lease kind and
deletes zero generations. For a direct process group, death of the lifetime
owner is likewise uncertain unless that still-live owner durably recorded its
descendant-drain witness before exit; only then may reconciliation validate the
exact owner stale and recorded group empty. A later reconciler must never invent
the witness.

The basename `.lease-transfer-<lease-id>` is reserved as a private unpublished
staging artifact for SPEC 0012's same-name process-to-lifecycle replacement;
`<lease-id>` is one canonical opaque lease identifier. It is never itself a
lease or release authority. A conforming producer creates at most that one
fixed staging name while holding the shared `activation.lock`, with
`O_CREAT|O_EXCL|O_NOFOLLOW`, current-UID mode 0600, bounded canonical lifecycle
bytes, and a file fsync before `RENAME_EXCHANGE` with `<lease-id>`. Plain
overwriting rename is forbidden. The producer holds descriptors for both the
exact staged lifecycle inode and exact source process inode, checks both paths,
performs the exchange, then proves the target names the held lifecycle inode
and staging names the held process inode before directory fsync. Only after
that fsync may it unlink the displaced process staging name and fsync again.

A crash can expose only two recoverable paired states with exact common
generation/PID/start-time/boot/UID identity: target process plus staging
lifecycle is **untransferred**, so recovery removes/fsyncs only the staging
artifact; target lifecycle plus staging process is **transferred**, so recovery
first fsyncs the exchange state and then removes/fsyncs only the displaced
process staging artifact. Before retrying a transfer or reclaiming any
lease/generation, Helm holds `activation.lock` and validates the whole lease
inventory without mutation. Each pair must use no-follow current-UID regular
exact-0600 files of at most 4096 bytes with canonical discriminators and exact
contents. Only after every inventory entry and every staging/target pair passes
may recovery normalize those two states and continue or retry. A missing target,
same-kind pair, malformed/cross-kind or identity-mismatched pair, unsafe
mode/owner/type, oversized payload, malformed reserved basename, or any other
inventory uncertainty retains all evidence and permits no staging, lease or
generation deletion.

If a pathname changes between pre-exchange validation and exchange, the
producer may exchange back only when post-exchange descriptor/content proof
shows the exact inverse pair and the rollback restores the held source process
inode at `<lease-id>`. It then validates that restored target and retains the
other evidence. If that proof or rollback is unavailable, it retains all paths
and returns an ambiguous fail-closed outcome; it never guesses, blindly unlinks,
or publishes an unchecked inode. Thus pre-replacement crashes retain the
original process lease without creating an unbounded or permanently blocking
producer artifact, while a committed exchange is tied to the checked inodes.

No read or write follows symlinks; all operations are descriptor-relative below
Helm's owned generated subtree. `current` cannot name absent, staging, malformed
or digest-mismatched content. An apply that cannot finish before pointer commit
leaves `current` unchanged. A successful pointer commit is the only event that
may make a generation active for future launches. It must not signal or reload
an existing process. A direct launch outside a verified Helm profile is not
generation-selected.

Pointer recovery recognizes only these producer-reachable journal inventories,
after validating the complete inventory before any mutation. For candidate `C`
and prior current `A`: `.current-C` contains `C` while `current` is `A` (or is
absent on a pristine apply) before replacement; `.current-C` contains `A` while
`current` is `C` after an existing-current exchange; `.absent-C` is empty while
`current` is `C` after a pristine rename; and `.committed-C` is empty for a
pristine committed activation or contains `A` for an existing-current committed
activation, in both cases while `current` is `C`. The sole permitted two-journal
inventory is pristine pre-rename `.current-C` containing `C` plus empty
`.absent-C`, with `current` absent. Every basename beginning `.current-`,
`.absent-`, or `.committed-` is reserved: invalid UTF-8, an invalid identifier
suffix, a non-regular or symlink entry, malformed or inconsistent content,
multiple/contradictory markers, or any other inventory fails closed in both
selection and recovery. Shared selection refuses every rollback-required
`.current-`/`.absent-` inventory; it may accept a valid `.committed-` cleanup
inventory only after validating `current`. Recovery never mutates any journal or
pointer until the whole inventory and every referenced generation validate.
When recovering a pristine post-rename `.absent-C` state, it first atomically
renames `current` to `.current-C` and fsyncs the generated root, producing the
permitted pristine two-journal inventory. It then removes `.absent-C` before
`.current-C` and fsyncs the root again. Thus process death at every recovery
boundary leaves either the permitted two-journal inventory, the permitted lone
`.current-C` inventory with no `current`, or the clean absent state.
After recovery reaches clean absence, valid sealed unpointed generation trees
are harmless orphans and do not prevent a later pristine publication. Before
accepting that publication, the writer validates every final entry below
`generations/` as a no-follow directory whose canonical manifest identity,
seal, receipt, membership and output digests match its valid generation-id
basename. It preserves those orphans and allocates a fresh non-colliding id.
Recovery first discards only recognized `.staging-<generation-id>` entries as
required above. Any malformed final name or tree, special entry, unrecognized
staging-like entry, or remaining pointer journal then keeps the absent-current
state fail closed rather than being ignored or guessed away.

GC recency is recorded explicitly rather than inferred from directory metadata
or the opaque generation identifier. The generated root contains the root-owned
regular file `publication-order`. Writers validate and update it under the same
exclusive `activation.lock`; GC reads it under that exclusive lock. A publisher
atomically replaces it from the reserved `.publication-order` staging sibling,
fsyncing the staged regular file and then the generated root after each rename.
It first reserves a unique sequence by durably advancing `next-sequence`; after
the staged tree is sealed and fsynced, it performs a second durable replacement
which adds the candidate mapping before the tree becomes final and before it may
create or commit any pointer journal for that generation. A crash after
reservation but before mapping leaves an allowed sequence gap. A crash after
mapping but before pointer commit leaves `current` unchanged and may leave a
historical mapping or a valid ordered unpointed orphan. A crash before either
order rename may leave `.publication-order`; publication may remove only that
recognized current-UID mode-0600 regular staging file under the exclusive lock,
then fsync the root before retrying. Selection ignores publication order and
continues to validate only the pointer transaction and selected sealed tree.

`publication-order` and `.publication-order` are reserved control names.
Operations which consume or mutate either name open it descriptor-relatively
with `O_NOFOLLOW` and require a current-UID regular file with mode 0600. Store
open and launcher selection intentionally do not parse or reject an existing
order entry: order state controls only publication and generation deletion, not
selection of an otherwise valid pointer and sealed tree. A publisher fails
closed without using or mutating a symlink, special file, foreign-owner entry,
unsafe-mode entry or malformed official record. GC first removes independently
provable stale leases, then treats any such entry, missing `publication-order`,
lingering staging file or malformed record as uncertain and deletes zero
generations.

When `publication-order` is absent, the first opener serializes control creation
with the exclusive persistent lock, creates it once with
`O_CREAT|O_EXCL|O_NOFOLLOW` at mode 0600, writes and fsyncs the canonical empty
record, and fsyncs the generated root before releasing the lock. Any existing
entry at that name, including an unsafe or malformed entry, is left untouched
and does not prevent the store from opening. Initialization never synthesizes
ordering for pre-existing final generations: if the record was lost and is
recreated after such generations, their missing mappings remain fail-closed.

## Publication order v1 (L4 refinement)

`publication-order` is canonical UTF-8 with this byte-exact grammar. Every line
ends in one LF; CR, blank lines, comments and extra records are forbidden:

```text
helm-generation-publication-order-v1\n
next-sequence <positive-decimal-publication-sequence>\n
generation <generation-id> <positive-decimal-publication-sequence>\n
... zero or more generation records, strictly sorted by generation-id
```

Each generation identifier has the same lowercase 128-bit grammar as `current`.
Sequences are positive `u64` decimal values containing only ASCII bytes `0` to
`9`, with no sign, whitespace or leading zeroes; they are unique within the
generation records and need not be contiguous. `next-sequence` is
also positive, appears exactly once immediately after the header, and is
strictly greater than every mapped sequence. Record ordering is only by the
unsigned byte order of generation identifiers; publication recency is defined
only by the numeric mapped sequence. Every final generation directory currently
present must have exactly one mapping. Historical mappings whose generation
directory has already been collected or whose interrupted attempt never made a
final tree are permitted and prevent identifier reuse. A new publisher reserves
the current `next-sequence`, refusing if it cannot advance that value, then
durably increments it before construction and adds its candidate mapping before
final-tree and pointer commit. Interrupted attempts may therefore leave sequence
gaps; neither generation IDs, directory iteration order, timestamps nor file
metadata may fill, reorder or interpret those gaps. Duplicate generation IDs,
duplicate mapped sequences, a mapped sequence greater than or equal to
`next-sequence`, noncanonical order or decimal encoding, missing mappings for
present final generations, or an unsafe/missing record makes generation recency
unprovable and causes fail-closed zero generation deletion.

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

## Theme-apply integration (L4 refinement)

The supported theme-apply path is generation-only.  It does not use
`OutputRoot`, direct target staging/rename, `Reloader`, `SystemReloader`, or
any process signal/command fan-out.  Its result reports the
`GenerationPublicationOutcome` directly: `Committed` and
`CommittedWithCleanupPending` identify the selected generation, while
`OutcomeAmbiguous` is not reported as an activated theme.  Existing mutable
target files are neither consulted for equality nor written by this path.

The CLI maps those outcomes without interpretation. `Committed(generation)` is
exit 0 and reports that generation as selected for future launches.
`CommittedWithCleanupPending { generation, cause }` is also exit 0 because the
pointer is durably committed; it reports the selected generation and a warning
that committed-journal cleanup remains pending. `OutcomeAmbiguous { candidate,
cause }` is exit 6, leaves stdout empty in human mode, and reports only that
activation is unconfirmed for the candidate; it must not call the candidate
selected, active, current, or successfully applied and must not run recovery or
retry automatically. SPEC 0006 fixes the exact human and JSON representations,
including safe escaping of the diagnostic cause. This result mapping neither
adds a live-upgrade mechanism nor preserves a control-socket wire contract.

Before publication, the apply boundary holds only safe input locators and opens
the configuration root.  It creates or opens `helm/generated` only
descriptor-relatively: each existing component is opened with `O_NOFOLLOW`,
the final generated root is current-UID mode 0700, and an absent `helm` or
`generated` component is created with `mkdirat` mode 0700 then reopened and
validated before use.  Every successful revalidation—whether this process
created the component, received `EEXIST`, or first observed it as
existing—fsyncs the configuration root after `helm` and the opened `helm`
directory after `generated`.  A symlink, special file, foreign owner, unsafe
final root mode, or failed revalidation refuses before any output write.
Path-based recursive creation is forbidden.

After `GenerationStore` has acquired its exclusive lock, its publication
callback copies and validates the input locators into one immutable snapshot:
the exact raw palette bytes, selected launch-profile bytes, complete
renderer-option map, and template catalogue fields.  It parses those snapshot
palette bytes, performs palette linting, derives colours, validates the
catalogue, and renders every output into memory.  The callback creates every
byte-exact preimage specified above, calculates its SHA-256, and passes only
the resulting five lowercase digests plus rendered `(normalized-target, bytes)`
pairs to `GenerationPublication`.  It never rereads a palette, template,
profile, option, or target path.  Thus concurrent applies serialize from input
capture through pointer durability, and every receipt describes one whole input
set.

The current built-in profile has no external launch-profile file.  Its selected
raw bytes are exactly `helm-theme-launch-profile-v1\nnone\n`; any later
profile selection supplies its own raw bytes instead.  The current renderer
option map is empty, so its renderer preimage is exactly
`helm-theme-renderer-v1\n`.  Empty is a value, not an omitted preimage.

For the catalogue preimage, `reload-kind-bytes` use this canonical UTF-8
encoding with no trailing newline: `none`; `helm-clients`; or
`signal:<decimal-process-byte-length>:<process-bytes><decimal-signal>`.  A
command uses `command:<decimal-argument-count>:` followed by one
`<decimal-byte-length>:<argument-bytes>` record for each argument in argument
order.  Decimal values are ASCII, canonical and unsigned; the process name and
all command arguments are valid UTF-8 without NUL, CR or LF.  Targets, template
IDs and option names meet the existing manifest character rules before receipt
formation.  The public apply seam accepts input locators/snapshot builders
(rather than a reserialized `Palette`) and has no reloader argument; test-only
fixtures may supply deterministic snapshot builders directly.

## Theme-diff integration (L4 refinement)

`theme diff` is a read-only comparison between one captured candidate input set
and the fully validated generation selected by `current`. It uses the same
snapshot, lint, derivation, catalogue validation, rendering, normalization, and
digest rules as apply. Before comparing any output, it resolves `current` and
validates the pointer transaction, generation identity, manifest, seal, receipt,
tree membership, and every listed output digest exactly as a launcher does.
Absent or invalid current state is an error, never an empty baseline.

The result is the lexicographically path-sorted union of candidate and current
normalized output paths, containing only `added`, `removed`, and
`byte-different` entries. `added` means the path exists only in the candidate;
`removed` means it exists only in the current manifest; `byte-different` means
both list the path and their exact bytes differ. Manifest, receipt, seal, and
other generation-control files are not outputs and never appear in the diff.

Diff does not create or initialize `helm/generated`, `activation.lock`,
`publication-order`, `generations/`, `leases/`, or any journal. It does not take
the exclusive writer lock, run recovery or GC, create a lease, stage or publish
a generation, replace `current`, write an output, or signal/notify a process.
If the generated root or required control state is absent, unsafe, malformed,
or inconsistent, diff reports that error without mutation. Concurrent pointer
change is handled by holding the existing shared activation lock through
current resolution, full validation, and comparison; diff never compares a
candidate with a partially validated or mixed generation.

## Acceptance criteria

| # | Given / When / Then |
|---|---|
| G1 | Given a selected old generation, when a newer generation is committed, then a launch using the old selection reads only old manifest-listed bytes. |
| G2 | Given interruption before pointer commit, when recovery runs, then `current` still names the previous valid sealed generation and staging is not launchable. |
| G3 | Given interruption after tree seal but before pointer replacement, when recovery runs, then the sealed tree is retained or safely collectible and `current` remains valid. |
| G4 | Given concurrent applies, when both complete, then every committed pointer refers to one complete manifest/tree pair and neither mixes palette/catalogue/target inputs. |
| G5 | Given a malformed, absent or digest-mismatched manifest, when a launcher resolves `current`, then it refuses before spawning a target. |
| G6 | Given a live process or lifecycle lease, when GC runs, then its generation is retained. Given a validated stale process lease, GC reclaims only that lease and unleased old generation candidates. A valid lifecycle lease is never PID-reclaimed: its generation remains pinned until SPEC 0012 reconciliation proves complete recorded ownership empty and releases it. A stale supervisor with a surviving scoped descendant, a dead direct owner without its prior descendant-drain witness plus exact-owner staleness and recorded-group emptiness, or missing/malformed/cross-kind/unreadable lifecycle evidence is uncertain and deletes zero generations. The two retained unleased candidates are those with the greatest valid publication sequences, independent of opaque IDs and filesystem timestamps. |
| G7 | Given rollback to a retained generation, when it commits, then future launches select it and already-running launches remain unchanged. |
| G8 | Given a corrupt pointer, missing tree, or digest mismatch, when recovery or launch runs, then it fails closed and never selects an arbitrary/newest generation. |
| G9 | Given concurrent applies, when both are accepted, then they serialize from input capture through pointer durability and each receipt identifies its committed generation. |
| G10 | Given a valid current generation and a candidate whose normalized output set differs, when `theme diff` runs, then it reports only lexicographically sorted `added`, `removed`, and `byte-different` output paths after fully validating current, and performs no control initialization, recovery, lease, GC, publication, output write, pointer switch, or reload. |
| G11 | Given a successful apply or rollback pointer commit, when existing processes continue running, then Helm sends no signal, command, or notification and only later launches may select the newly current generation. |
| G12 | Given `Committed`, `CommittedWithCleanupPending`, or `OutcomeAmbiguous`, when `helm ctl theme apply` reports the result, then the first two exit 0 and name the selected future-launch generation (with a cleanup warning for the second), while the ambiguous result exits 6, claims no activation, safely reports its candidate/cause, and performs no automatic recovery or retry. |

## Boundaries

This specification does not make arbitrary user configuration Helm-owned and
does not settle systemd/no-systemd teardown, scope adoption, D-Bus ownership,
or target package sources. [SPEC 0012](0012-activation-launch-lifecycle.md) is
the M2 lifecycle owner for #132/#168: it consumes a process selection into a
durable lifecycle lease before exec, forbids drop-based release after transfer,
and permits release only after verified scope/process-group emptiness. #117,
#133 and #134 retain their existing boundaries. A no-op apply optimization and compatibility with the
retired mutable apply result or any existing control-socket wire message are
not promised.

Live upgrade is also outside this specification. #22 may define only a future
generation-aware owned-process protocol which proves the generation selected by
each process before coordinating an upgrade. It cannot restore direct
signal/command fan-out, session notification, or any other reload as a side
effect of a `current` pointer switch, and it cannot make foreign or direct
launches generation-selected retroactively.
