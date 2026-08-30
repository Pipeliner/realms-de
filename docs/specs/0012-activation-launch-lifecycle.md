# SPEC 0012 — Activation launch lifecycle

- **Status:** Draft — candidate for acceptance; implementation is blocked until
  acceptance and session-conformance claims are blocked on the live ADR 0011 /
  SPEC 0005 publication guards scheduled by #60; public typed-launch claims
  are also blocked on the Draft SPEC 0006/#117 amendment
- **Milestone:** M1
- **Decisions:** [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0017](../adr/0017-immutable-theme-activation-generations.md),
  [ADR 0018](../adr/0018-m1-desktop-launch-is-fresh-exec-only.md)
- **Issue:** [#132](https://github.com/Pipeliner/realms-de/issues/132)
- **Supersedes / Superseded by:** Supplies the M1 activation-ownership and
  teardown clauses that Draft SPECs 0003 and 0005 previously left unresolved;
  it does not accept either draft as a whole.

## Purpose

A profile launch must never outlive the evidence that identifies its immutable
generation, and a session restart or logout must never duplicate or accidentally
kill that launch. This specification joins SPEC 0011's sealed-generation lease
to one durable launch owner, a persistent lifecycle record, systemd scope or
direct process-group ownership, conservative reconciliation, and the session
entry's bounded teardown.

## Scope

**In:** the per-UID session claim; the lifetime owner of a generation-selected
profile launch; transferable generation leases; persistent activation records;
the pre-exec adoption order; systemd-scope and no-systemd ownership; WM and user
manager restart reconciliation; logout, lingering and concurrent-session
behaviour; target stop propagation; and the semantic snapshot boundary after a
lifecycle server restart.

**Out:**

- Sealed-tree construction, selection, manifest validation, process-lease
  creation, and generation deletion remain [SPEC 0011](0011-theme-activation-generations.md).
- River existing-window replay, ledger reconstruction, projection and readiness
  remain Draft [SPEC 0003](0003-helm-session.md) M2. In particular this spec does
  not answer its river replay open question.
- Desktop-id resolution, immutable desktop preflight, tokenized `Exec`,
  unconditional `DBusActivatable=true` refusal, child-only launch bindings and
  raw/fuzzel bypass truth remain Draft
  [SPEC 0013](0013-truthful-desktop-exec-launch.md)/[#133](https://github.com/Pipeliner/realms-de/issues/133).
  This lifecycle admits only the resulting immutable fresh-Exec plan. It has no
  existing-owner concept for plain `Exec`; D-Bus entries are refused without an
  owner query before any launch-local lifecycle or generation side effect.
- Session-entry systemd-user and D-Bus publication is already required by
  Accepted ADR 0011 and specified by Draft SPEC 0005; #60 schedules that work
  and #117 coordinates the public integration. This spec owns only the durable
  per-UID claim prerequisite and lifecycle ordering. #133 may specify
  compare-and-restore/cleanup and unsupported launch modes; it cannot defer,
  weaken or replace ADR 0011's mandatory dual import.
- Exact profile assets, target packages, consumer-specific argument additions,
  environment bindings and consumer behaviour remain
  [#135](https://github.com/Pipeliner/realms-de/issues/135). A launch prepared
  here accepts only SPEC 0013's immutable admitted Exec plan and may use only
  its held desktop snapshot, sealed generation, persistent state and
  authenticated transferred descriptors.
- Locker and idle defaults remain SPEC 0005 OQ-1. No clause here selects a
  locker or an idle policy.
- Public request/response/event DTOs, idempotency keys, frame limits, history
  retention, cursors and replay spelling remain Draft SPEC 0006/#117. Section 8
  constrains only fresh-snapshot semantics.

## Behaviour

### 1. Ownership vocabulary and invariants

A **session helper** is session infrastructure such as `helm-wm` or `helm-bar`.
On the systemd path it is a member of `helm-session.target` and stops with that
target. On the no-systemd path the entry owns an identity-recorded bounded
supervisor for the equivalent helper. A **launcher helper** is any fixed session
UI that requests a profile launch; it is session infrastructure and must use the
same ownership mode. A **profile launch** is the separately owned user
application resulting from one immutable admitted Exec plan governed by SPEC
0013, whose exact consumer invocation and environment bindings are governed by
#135. It is never a member, wanted unit, or `PartOf=` dependent of
`helm-session.target`.

Every admitted profile launch has exactly one **lifetime owner**. The owner is
a small Helm supervisor created before selection, kept behind an exec gate, and
placed in the launch's scope or process group. After durable authorization it
starts at most one fresh, ownership-compatible profile application and remains
alive while any process Helm can attribute to that launch may dereference
generation content. The application process is not itself the lifetime owner:
it may fork, replace itself, or exit while another owned member remains.

Every launch satisfies all of these invariants:

1. One launch id identifies one session-scoped request id, one lifetime-owner
   identity, one generation id, one manifest SHA-256, one lease reference, and
   at most one executed profile. A request id maps to at most one launch id.
2. The generation is fully validated and its lease is durable before any
   profile code can execute. A later `current` switch cannot affect it.
3. A launcher-local selection releases on failure only until its lease has been
   durably transferred. Successful transfer consumes that release authority;
   dropping any launcher-side value afterwards cannot unlink the lease.
4. A transferred lifecycle lease is not reclaimed merely because its recorded
   owner PID died. Only this lifecycle contract may remove it, after it has
   disproved all attributed scope/process-group membership. Missing,
   inconsistent or unreadable evidence is uncertain and retains the lease.
5. A lifecycle record is evidence, never authority to signal, kill, adopt,
   replace or delete a process. Every such operation revalidates boot id, UID,
   PID start time and, for systemd, unit name, invocation id and cgroup
   membership immediately before acting.
6. Reconciliation advances durable state monotonically, never recreates a user
   application, never moves a live launch between ownership modes, and never
   opens a second exec gate for an existing launch id.

Anything a profile launch may need after logout is either inside its validated
sealed generation, in persistent state defined here, or already held through an
authenticated transferred descriptor. It may not depend on the control socket, entry/WM PID
files, ledger snapshot, an exec gate, or any other path below
`$XDG_RUNTIME_DIR/helm` after it begins running.

### 2. Persistent state and serialization

The activation root is `$XDG_STATE_HOME/helm/activation`. If
`XDG_STATE_HOME` is unset or empty, Helm uses `$HOME/.local/state/helm/activation`.
The selected base must be absolute; absent/non-absolute required inputs fail
before a session claim or launch. The layout is:

```text
activation/
  lifecycle.lock
  pidfd-health
  .pidfd-health-create-<service-incarnation>-<nonce>
  .pidfd-health-update-<sequence>-<nonce>
  pidfd-probe
  .pidfd-probe-create-<probe-id>-<nonce>
  .pidfd-probe-update-<probe-id>-<sequence>-<nonce>
  session
  .session-claim-<session-id>
  .session-update-<session-id>-<sequence>-<nonce>
  preparations/
    <preparation-id>
    .preparation-create-<preparation-id>-<nonce>
    .preparation-update-<preparation-id>-<sequence>-<nonce>
  launches/
    <launch-id>
    .launch-create-<launch-id>-<nonce>
    .launch-update-<launch-id>-<sequence>-<nonce>
```

`session` is absent when no Helm session claim exists. `session-id`,
`preparation-id` and `launch-id` are independently generated opaque 128-bit lowercase hexadecimal
values matching `[a-f0-9]{32}`. A generation lease reference has the same
grammar and names its existing record below SPEC 0011's `leases/` directory; it
is not a mutable generated path.
The request id has that same byte grammar but is supplied by the typed client
and is unique only within its exact session id.

Helm creates each owned directory descriptor-relatively at mode 0700 and the
persistent `lifecycle.lock` once at mode 0600. Every existing component must be
owned by the current UID and have exactly that type and mode. All traversal and
mutation is descriptor-relative with `O_NOFOLLOW`; symlinks, special files,
foreign ownership and unsafe modes fail closed. The lock pathname is never
deleted or replaced for recovery. Kernel lock release is writer recovery.

State-root initialization is itself a crash-recoverable inventory and finishes
before any record or lifecycle mutation. With a process umask that preserves
owner mode bits, an initializer opens the selected state base without following
links, then creates the owned `helm` and `activation` components one at a time
with `mkdirat` at exact mode 0700. After each successful creation it opens and
verifies the new directory. `EEXIST` is accepted only after the same no-follow
owner/type/mode verification. Whether newly created or found by verified
`EEXIST`, the initializer fsyncs the opened directory and its parent before any
dependent creation or mutation; an absent component is created or an unsafe
existing component fails closed and is never removed or repaired in place.

In the verified activation root, every initializer first attempts
`openat("lifecycle.lock", O_RDWR|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC, 0600)`.
On `EEXIST` it instead opens the existing pathname with
`O_RDWR|O_NOFOLLOW|O_CLOEXEC`. The resulting file must be a current-UID,
zero-length regular file with exact mode 0600. The initializer acquires its
exclusive advisory lock, revalidates that the pathname still names the opened
device/inode, fsyncs the file and activation root, and only then may inspect or
mutate lifecycle state. After a crash, recovery can observe only absence or one
valid published inode: absence is recreated, while a present inode is locked and
made durable before use. Concurrent initializers may race to create or acquire
it, but all lifecycle work serializes through that same revalidated inode.

While holding that lock, initialization creates/verifies `preparations/` and
`launches/` at exact mode 0700 by the same create, child-fsync and parent-fsync
protocol. Their only
recoverable crash states are absent or one valid directory; invalid metadata or
any other object at that name fails closed. The `lifecycle.lock` pathname is
never unlinked or replaced after its first atomic create.

Every record is a current-UID mode-0600 regular file, at most 4096 bytes,
encoded as canonical UTF-8 with one LF after every line, no CR, comments, blank
lines, duplicate fields or extra bytes. Unsigned decimal fields contain no sign
or leading zero except the value `0`. Boot ids use lowercase canonical UUID
text; SHA-256 values use 64 lowercase hexadecimal digits. A `nonce` has the
same 32-lowercase-hex grammar as an id. `probe-id` and
`service-incarnation` have the id grammar. The ten temporary-name forms shown
above are the complete reserved grammar. The id, sequence and nonce of a
temporary are classified only from its basename bytes; decoding its payload is
never part of temporary-name validation or a precondition for discard. If a
temporary is parsed only for diagnostics and happens to contain a complete
canonical payload, a payload id or sequence mismatch may be reported but does
not change discardability and never authorizes replay.

A temporary is a **discardable unpublished artifact** only when recovery holds
`lifecycle.lock`, its basename exactly matches one of those ten forms, no-follow
metadata proves it is a current-UID regular file with exact mode 0600, an
`O_NOFOLLOW` open proves its size is at most 4096 bytes, and the applicable
inventory below satisfies its final-record condition. These facts exclusively
decide discardability. Its payload may be empty, partial, non-UTF-8 or otherwise
noncanonical: no temporary is authority, and payload validity or decoded
id/sequence equality is not required for safe discard.
An entry using a reserved prefix but not the exact grammar, or with wrong
owner/mode/type, symlink/special type or size above 4096 bytes, is a
**malformed reserved entry** and fails closed. A malformed final `pidfd-health`,
`pidfd-probe`, `session` or `launches/<launch-id>` record likewise always fails closed; the discard rule
never applies to final names.

Initial session publication never writes the fixed final name in place. Under
`lifecycle.lock`, the claimant creates `.session-claim-<session-id>` with
`O_CREAT|O_EXCL|O_NOFOLLOW` at mode 0600, writes and fsyncs the complete
sequence-1 `claimed` record, verifies `session` is absent, then uses
`renameat2(RENAME_NOREPLACE)` to publish it as `session` and fsyncs the
activation root. `EEXIST` changes neither record; the claimant removes and
fsyncs only its own temporary and loses. A crash leaves exactly one of: no
entry, one incomplete or complete discardable claim temporary, or one complete
final `session` record. It can never expose an empty or partial final record.

Recovery holds `lifecycle.lock`, so no conforming producer can still be writing
a temporary. With no final `session`, it unlinks and root-fsyncs every
discardable claim temporary regardless of payload, then permits a fresh claim;
no compositor/global/helper side effect was allowed before final publication.
With a valid final `session`, that record is the sole claim and recovery removes
all discardable claim temporaries left before a losing `RENAME_NOREPLACE` or
cleanup crash. Multiple discardable temporaries are cleaned, not treated as a
conflict. Any malformed reserved entry or malformed final record is preserved
and fails closed. The fixed final name needs no filename id: its decoded
session id is checked against every canonical update and compare-and-replace
transition.

Initial launch publication uses the corresponding
`.launch-create-<launch-id>-<nonce>` file and no-replace rename to the final
`launches/<launch-id>` name. Existing-record transitions write and fsync the
appropriate update temporary, atomically replace the matching final record,
and fsync the parent directory. Recognized temporaries are artifacts, never
records. Adoption is forbidden until the create rename and parent fsync finish,
so any discardable create temporary can be unlinked and directory-fsynced under
the lock regardless of payload; a stale process lease is then handled by SPEC
0011, and profile code cannot have executed. An update temporary is never
replayed. When its matching final record is valid, recovery captures the final
record plus actual lease/ownership inventory, discards and fsyncs every
producer-shaped update artifact regardless of payload, then applies section
5's table to any already-committed external side effect such as lease transfer.
An update temporary without its matching valid final record, or any malformed
reserved entry, is not producer-reachable and fails closed.

Pidfd-health/probe temporaries use the same artifact rule with these exact
final conditions. A `.pidfd-health-create-*` temporary is discardable only
when fixed final `pidfd-health` is absent; a `.pidfd-health-update-*` temporary
requires one valid final health record. A `.pidfd-probe-create-*` temporary is
discardable only when fixed final `pidfd-probe` is absent; a
`.pidfd-probe-update-*` temporary requires one valid final probe record with
the same decoded probe id and the preceding sequence. When the required final
is valid, recovery discards/fsyncs every matching producer-shaped temporary
without replay. An update temporary without its required final, a create
temporary beside malformed final evidence, multiple probe finals, or any
malformed reserved form fails closed. Absence of final health is not inferred
as healthy: typed desktop admission remains closed until a fresh probing record
is durably published and the full protocol reaches healthy. A final probe
record is containment authority and is never discarded as a temporary; only
its exact external emptiness proof authorizes removal.

Discardable temporaries share the registry's 4096-entry and 16-MiB scan bounds;
recovery removes them before applying final-record admission quotas. Natural
crash artifacts therefore cannot accumulate across successful recovery or
permanently block a later claim/GC. Exceeding a bound or encountering a
malformed reserved entry stops destructive GC and preserves evidence.

All session claims, record transitions, reconciliation and registry GC take an
exclusive advisory lock on `lifecycle.lock`. A caller supplies the sequence it
observed; a transition succeeds only if it is the permitted successor and
increments that positive `u64` by exactly one. A stale or conflicting sequence
changes nothing. If both locks are needed, the only permitted order is
`lifecycle.lock` then SPEC 0011's `activation.lock`; code holding
`activation.lock` must not acquire `lifecycle.lock`.

The per-UID typed-desktop pidfd capability record has this byte order:

```text
helm-pidfd-health-v1
sequence <positive-u64>
state <probing|healthy|failed>
service-incarnation <lowercase-32-hex>
boot-id <canonical-uuid>
coordinator-pid <positive-u32>
coordinator-start-time <positive-u64>
probe <none|probe-id>
failure-token-sha256 <lowercase-64-hex>
failure-token <none|lowercase-64-hex>
affected-launch <none|launch-id>
failure-phase <none|startup-probe|initial-poll|timeout-signal|result-wait|final-reap>
supervisor-pid <u32>
supervisor-start-time <u64>
```

Initial no-replace publication uses
`.pidfd-health-create-<service-incarnation>-<nonce>`; later transitions use the
update temporary. Both follow the same write/fsync/rename/root-fsync rules.
`probing` and `healthy` require `failure-token none`, `affected-launch none`,
`failure-phase none` and zero supervisor fields. `probing` names the one
reserved probe id; `healthy` requires `probe none`. `healthy` is valid only after
the same incarnation's full SPEC 0013 probe and probe-containment cleanup.
`failed/startup-probe` names the probe id, has no affected launch and has zero
supervisor fields. Every other `failed` record requires `probe none` and has the exact affected launch plus its positive lifetime-
supervisor identity and operation phase. In any failed record the raw
`failure-token` hashes to `failure-token-sha256`; only supervisors inheriting
that random 256-bit service-incarnation token can authenticate a monotonic
`healthy -> failed` transition. Publication holds `lifecycle.lock`, validates
the affected launch/lease/supervisor relation where present, increments the
sequence and fsyncs the activation root before the supervisor exits.
After that first commit, another failing supervisor from the same service/boot/
coordinator incarnation never updates the fixed record. Under the same lock it
validates the committed raw-token proof against its inherited token, preserves
its own legal launch tuple/lease, and may then close capabilities and exit
without `owner-drained`. A different incarnation, invalid proof or malformed
relation is not idempotent success and retains lock/owner authority fail-closed.

The health transition grammar is exhaustive. Absence may no-replace-publish
only sequence-1 `probing` for a new service incarnation. Within one incarnation
the only successors are `probing -> healthy`, `probing -> failed/startup-probe`
and `healthy -> failed/<actual-operation-phase>`. A different service
incarnation never unlinks the fixed health record: after proving the recorded
coordinator PID/start-time stale, it atomically replaces the valid old final
with sequence+1 `probing`, new coordinator identity, new probe id, new token
digest and all failure/raw-token fields cleared. Replacement from old `failed`
also requires its probe containment absent after exact cleanup and a complete
bounded launch scan in which every nonterminal typed launch present under that
failed service state—not only `affected-launch`—has reached terminal total
emptiness with its lease released. Any live/uncertain/malformed such ownership
blocks replacement. Replacement from old `probing`
requires the matching probe record reconciled/absent; probing with no probe is
never success and still requires a new full probe. Replacement from old
`healthy` requires its probe absent. A live/uncertain old coordinator, malformed
record or mismatched probe blocks replacement and typed desktop admission.
Only the new incarnation's complete probe may advance its probing record to
healthy.

The per-service probe containment record has this byte order:

```text
helm-pidfd-probe-containment-v1
probe <probe-id>
sequence <positive-u64>
state <reserved|contained|failed>
service-incarnation <lowercase-32-hex>
boot-id <canonical-uuid>
creator-pid <positive-u32>
creator-start-time <positive-u64>
owner-pid <u32>
owner-start-time <u64>
owner-kind <systemd-scope|process-group>
unit <none|helm-pidfd-probe-<probe-id>.scope>
unit-invocation <none|lowercase-32-hex>
process-group <u32>
cgroup <none|normalized-relative-cgroup-path>
cgroup-device <u64>
cgroup-inode <u64>
```

For `contained`/`failed`, systemd/direct field combinations and anti-reuse
validation are exactly the launch-owner rules below. A systemd record has a verified unit invocation,
cgroup path/device/inode and zero process group. A direct record has `none`/0
systemd fields and a process group equal to the probe supervisor PID. The
`reserved` record is created/fsynced from `.pidfd-probe-create-*` before owner
creation and matches the probing health record's probe/service incarnation and
exact creator PID/start-time. It has zero owner fields; the systemd unit is the
deterministic probe-id name while all incarnation/cgroup fields remain none/0,
and direct ownership has all containment fields none/0. The newly created inert
probe supervisor sets `PR_SET_PDEATHSIG=SIGKILL`, rechecks the creator, and
blocks on a pipe before placement or clone. Only after exact placement and
identity revalidation does the creator atomically update/fsync the record to
`contained` with positive owner identity and full systemd/direct evidence, then
open the barrier. The supervisor inherits that membership, so the disposable
child cannot run outside it. Failure is not a joint atomic commit:
`pidfd-health/failed` is the sole authority and is replaced/root-fsynced first;
only then is this record updated/root-fsynced to `failed` before the supervisor
closes its barrier/exits. The record remains
until the external startup reconciler proves the exact old owner stale and the
entire recorded cgroup/group empty without reuse; only then may it unlink/fsync
the record. Wrong boot, name/invocation, PID/start-time, cgroup device/inode,
group membership or permission uncertainty authorizes no signal or removal.
Every restart reconciles this fixed record before creating a new probe or
opening typed desktop admission. A `reserved` record with a proven-stale exact
creator is safe to remove only after the deterministic unit name is absent in
systemd mode; no probe child could have been created and PDEATHSIG killed any
pre-publication supervisor. Reuse, a live/uncertain creator or a present unit
fails closed. Direct reserved recovery signals no unknown PID/group and relies
only on stale creator plus the barrier/PDEATH guarantee.

The session record has this byte order:

```text
helm-activation-session-v1
session <session-id>
sequence <positive-u64>
state <claimed|preparing|active|admission-frozen|helpers-stopped|cleanup-delegated|closed>
owner-uid <u32>
boot-id <canonical-uuid>
entry-pid <positive-u32>
entry-start-time <positive-u64>
compositor-pid <u32>
compositor-start-time <u64>
helper-mode <none|systemd|direct>
wm-pid <u32>
wm-start-time <u64>
wm-process-group <u32>
bar-pid <u32>
bar-start-time <u64>
bar-process-group <u32>
```

Both compositor fields are `0` in `claimed` and remain `0` through teardown
only when failure occurred before compositor publication; otherwise both are
positive and identify the exact compositor process. `helper-mode none` requires
all six helper identity fields to be `0` and is valid only for `claimed` or its
early-failure teardown. `systemd` also uses zero direct-helper fields because
unit identities own them. `direct` permits zero pairs while `preparing` records
start progress, but `active` requires positive PID/start-time/process-group
triples for both bounded helper supervisors. Teardown preserves the last
identities until their death is proven. The entry identity is live before
`closed` unless a reconciler has taken over after proving that exact identity
stale.

Each `preparations/<preparation-id>` record is the authoritative durable
pre-gate reservation and has this byte order:

```text
helm-exec-preparation-v1
preparation <preparation-id>
sequence <positive-u64>
state <coordinator-owned|supervisor-received>
session <session-id>
request <lowercase-32-hex>
desktop <desktop-file-id>
boot-id <canonical-uuid>
deadline-ns <positive-u64-CLOCK_MONOTONIC-nanoseconds>
coordinator-incarnation <lowercase-32-hex>
supervisor-pid <u32>
supervisor-start-time <u64>
launch <launch-id>
lease <lease-id>
gate-token-sha256 <lowercase-64-hex>
```

Initial no-replace publication uses
`.preparation-create-<preparation-id>-<nonce>` and the same file/fsync/rename/
directory-fsync rules as launch creation. It reserves already-generated launch
and lease ids, starts with sequence 1, `coordinator-owned`, and zero supervisor
fields. After supervisor creation, one atomic update records both positive
supervisor identity fields before the reserved process lease may be created.
Every later update increments sequence by one. `deadline-ns` is meaningful
only with the same recorded boot id; another boot treats it expired. A
`coordinator-owned` record is one counted slot. `supervisor-received` is the
durable proof that the exact supervisor accepted the gate arm before the
absolute deadline and is no longer counted. The record is unlinked and its
directory fsynced only after the coordinator/reconciler has either observed
that received proof or completed the exact cancellation recovery row. Invalid
metadata/content, missing/mismatched reserved ids, uncertain supervisor
identity or an over-bound inventory fails closed and retains the corresponding
active and namespace credits.

The `preparations/` namespace has a separate hard bound of 512 directory
entries and 2 MiB of `st_size`, including `coordinator-owned` and
`supervisor-received` finals, `.preparation-create-*` and
`.preparation-update-*` temporaries, and every malformed or unsafe entry. Each
individual file remains limited to 4096 bytes. Inventory enumeration stops and
fails closed as soon as either bound is exceeded; malformed entries are never
discarded merely to make room. For operation accounting, each valid
`coordinator-owned` final consumes two entry/8192-byte credits—its final plus
the one possible atomic-update temporary—while each valid
`supervisor-received` final consumes one entry/4096-byte credit. A matching
single update temporary uses the owner's reserved second credit; every extra,
unmatched or malformed entry consumes its own actual entry/byte budget and
fails admission closed. Under the coordinator mutex and `lifecycle.lock`, new
admission first removes only safely discardable temporaries, computes both
physical totals and reserved credits, and reserves two free credits before
publishing the create temporary. Receipt CAS releases only the update credit;
receipt removal releases its final credit. Thus non-counting receipts remain
namespace-accounted, repeated successful batches cannot grow the inventory
without bound, and restart can reproduce the same accounting from disk.

Each `launches/<launch-id>` record has this byte order:

```text
helm-activation-launch-v1
launch <launch-id>
session <session-id>
request <lowercase-32-hex>
sequence <positive-u64>
state <preparing|adopted|authorized|starting|application-running|owner-drained|terminal>
result <none|exited|failed|lost>
owner-uid <u32>
boot-id <canonical-uuid>
desktop <desktop-file-id>
desktop-sha256 <lowercase-64-hex>
generation <generation-id>
manifest-sha256 <lowercase-64-hex>
catalogue-sha256 <lowercase-64-hex>
profile <profile-id>
profile-sha256 <lowercase-64-hex>
lease <lease-id>
lease-kind <process|lifecycle>
owner-pid <positive-u32>
owner-start-time <positive-u64>
owner-kind <systemd-scope|process-group>
unit <none|helm-launch-<launch-id>.scope>
unit-invocation <none|lowercase-32-hex>
process-group <u32>
cgroup <none|normalized-relative-cgroup-path>
cgroup-device <u64>
cgroup-inode <u64>
exec-open <no|yes>
exec-evidence <pending|positive|negative|lost>
```

`owner-drained` is written only by the still-live supervisor in either
ownership mode after it has proved every other attributed member absent and
before that supervisor exits. An external reconciler never synthesizes this
witness and may instead take one of the terminal bypasses below. It is not
total scope/group emptiness. `manifest-sha256` equals the digest contained in the validated generation's
`seal`, excluding its LF. Desktop/profile identifiers and digests are the exact
SPEC 0013 authenticated-plan values and never change. `lease-kind process` is written initially and changes
to `lifecycle` in the first durable record after transfer. A systemd record
uses the exact unit name shown, `process-group 0`, and records invocation/cgroup
fields as `none`/`0` only before verified adoption. A cgroup path is the
`ControlGroup` value with exactly one leading `/` removed. It is non-empty,
contains only ASCII components matching `[A-Za-z0-9_.:@%+-]+`, has no empty,
`.` or `..` component, and is resolved descriptor-relatively with no symlink
following below an opened `/sys/fs/cgroup`. Device and inode are captured from
that verified directory and are both positive. A direct record uses both unit
fields and `cgroup` as `none`, all cgroup numeric fields as `0`, and a process
group equal to the lifetime owner's PID. `exec-open yes` means authorization was durably
recorded before the unique gate token could be sent; it never returns to `no`.

The canonical state/result/authorization/evidence/lease relation is exhaustive:

| State | Result | `exec-open` | `exec-evidence` | Expected lease / route |
|---|---|---|---|---|
| `preparing` | `none` | `no` | `pending` | `process` in the producer; recovery may observe the explicitly listed transfer/absence crash windows |
| `adopted` | `none` | `no` | `pending` | `lifecycle` |
| `authorized` | `none` | `yes` | `pending` | `lifecycle` |
| `starting` | `none` | `yes` | `pending` | `lifecycle` |
| `application-running` | `none` | `yes` | `positive` | `lifecycle` |
| `owner-drained` | `exited` | `yes` | `positive` | `lifecycle`; live-supervisor wait result |
| `owner-drained` | `failed` | `yes` | `positive` or `negative` | `lifecycle`; post-proof signal or exact known no-Exec failure |
| `owner-drained` | `lost` | `yes` | `lost` | `lifecycle`; ambiguous live-supervisor evidence |
| `terminal` | `failed` | `no` | `negative` | expected `process` after a `preparing` bypass or `lifecycle` after a `preparing` transfer-window/`adopted` bypass; matching actual lease until release, then absent |
| `terminal` | `failed` | `yes` | `positive` or `negative` | expected `lifecycle`; preserves `owner-drained/failed` or is the exact `authorized`/known-no-Exec bypass; matching actual lease until release, then absent |
| `terminal` | `lost` | `yes` | `lost` | expected `lifecycle`; preserves `owner-drained/lost` or is the exact `starting`/`application-running` crash/pidfd-operation bypass; matching actual lease until release, then absent |
| `terminal` | `exited` | `yes` | `positive` | expected `lifecycle`; preserves only `owner-drained/exited`; matching actual lease until release, then absent |

Every other combination is malformed. In particular, `none` is legal only
through `application-running`, never at `owner-drained` or `terminal`;
non-`none` is legal only at those last two states. The split terminal rows are
the validator grammar: in particular `terminal/exited` preserves only a live
supervisor's `owner-drained/exited`, and an external bypass from `starting` or
`application-running` is `lost` because the crashed supervisor's application
result cannot be reconstructed.

`exec-evidence` is monotonic execution evidence, not admission mode. It changes
from `pending` exactly once: to `positive` only with SPEC 0013's full post-Exec
proof, to `negative` only when exact evidence proves no target Exec occurred,
or to `lost` when the proof/result becomes unknowable. Terminalization preserves
it except that an external crash bypass maps unresolved `pending` to the exact
`negative` or `lost` row above. It never returns to `pending`.

Final result mapping is exact and independent of application exit-code value:

| Observation | Result |
|---|---|
| Gate-closed cancellation/refusal, abort before `starting`, or the exact eight-byte `execveat` errno frame | `failed` |
| After the positive post-Exec proof, exclusive `waitid(P_PIDFD, ...)` reports `CLD_EXITED`, for exit status 0 or any nonzero status | `exited` |
| After the positive post-Exec proof, exclusive `waitid(P_PIDFD, ...)` reports `CLD_KILLED` or `CLD_DUMPED`, including supervisor teardown signal | `failed` |
| Any ambiguous pipe/pidfd/procfs observation, supervisor loss that discards the authoritative wait status, detachment, identity disagreement or unprovable membership/result | `lost` |

The `launches/` namespace admits at most 4096 total final-record plus temporary
entries and 16 MiB of their bytes; each file retains the 4096-byte limit.
Before refusing a new launch it removes safe unpublished artifacts and runs the
conservative collection rules below. Live or uncertain records are never
evicted to meet a quota. For the typed desktop relation, a terminal launch
record is retained until its matching session reaches durable `closed`; its
session/request/launch tuple is the bounded idempotency index. The lifecycle
lease is still released immediately after terminal emptiness proof. The
4096-record namespace is therefore also a per-session request-id budget:
exhaustion refuses a new typed request rather than evicting retry identity.
After the matching session is durably `closed`, or after its record has been
validly removed and is absent/replaced by a different session id, terminal
records are collectible; session-record removal is authorized only after close
and therefore preserves this proof. An old-session retry is refused and never
becomes a new launch. Longer history, if any, remains a
separate SPEC 0006/#117 promise.

### 3. Session claim and state machine

The session state machine is:

```text
absent -> claimed -> preparing -> active -> admission-frozen
       -> helpers-stopped -> cleanup-delegated -> closed -> absent
```

Evidence publication may retain `preparing` while its sequence increments to
record direct helper identities. It may not change any identity backwards or
open admission before the `active` transition.

Failure from any state before `closed` enters the same idempotent teardown at
`admission-frozen`; it never skips backwards. The atomically published claim
protocol in section 2 completes first. Only then may the entry start the
compositor. It durably records the compositor identity and selected helper mode
in `preparing` before any per-UID systemd/D-Bus mutation or helper start.
Therefore a `claimed` record with absent compositor/helper identities proves
that no global environment or helper mutation was authorized by this session.

After the durable per-UID claim and compositor record, the session entry
performs ADR 0011/SPEC 0005 publication into both the systemd user-manager and
D-Bus activation environment before starting `helm-session.target` or any
client. This spec owns only the claim prerequisite and lifecycle ordering; #60
schedules the already-required work, #132 owns this ordering, and #133 owns
only restoration/compare-and-restore and refused activation/owner modes. A
test-only no-op/fake proves lifecycle sequencing only. It is non-conforming
evidence for a real session, portal readiness, D-Bus behaviour, `doctor`, or
ADR 0011/SPEC 0005, and cannot authorize a session-conforming `active`
transition. There is no public D-Bus activation-environment readback. The
systemd path records conforming `active` only after exact systemd-manager
`Environment` readback, successful import command status, a bounded functional
D-Bus-activated portal witness with no pre-existing portal owner, and helper
verification. The freshly activated portal proves only that one bus-activated
service can select/connect to the current desktop; it does not expose or prove
every bus environment value. An already-owned portal is a weaker functional
doctor observation and cannot satisfy this session-entry import gate.

The NixOS acceptance fixture additionally installs a test-only
`org.helm.TestEnvironmentProbe1` D-Bus service on a fresh bus with no existing
owner. Its D-Bus service file starts a fixed sentinel as the first post-import
activation. One bounded `GetEnvironment(s nonce) -> (s nonce, a(ss) values)`
call (two-second monotonic deadline) returns the echoed nonce and exactly the
ADR 0011 import-list names/inherited UTF-8 values sorted by name; duplicates,
omissions, extras, mismatch, pre-existing
owner, timeout or non-exit refuse the fixture. After one reply the sentinel
exits. This proves what that sacrificial D-Bus-activated process inherited; it
is not a general API for reading the bus activation environment and is not a
production `doctor` capability.

The no-systemd path uses the same claim and compositor boundary, selects
`helper-mode direct`, and follows SPEC 0005's named degraded publication path.
ADR 0011/SPEC 0005, not SPEC 0013/#133, decide its publication requirement;
#133 may later govern safe restoration of values this session actually wrote.
A lifecycle-only fixture may use a fake boundary and exercise an internally
labelled `active` lifecycle state, but that state is expressly non-conforming
and cannot satisfy any session/portal/D-Bus/doctor acceptance gate.
It creates one bounded supervisor/process group for
`helm-wm` and then `helm-bar`, records each PID/start-time/group before opening
that helper's gate, and applies SPEC 0005's one-second delay and five-in-thirty
restart limits. The WM restarts after every unrequested clean or failed exit
while the session is active (the direct equivalent of `Restart=always`), except
that status 69 or 78 prevents restart. The bar restarts only after failure, not
after a clean exit. A WM restart-prevent status, refusal of the next WM start by
the five-in-thirty limit, or failure to establish WM readiness causes exactly
one transition into the session's idempotent admission-freeze and teardown
path; it never opens profile admission. A bar whose next start is refused by
the same limit remains the named degraded condition and does not tear down the
session.
Only after both supervisor identities are durable and the WM is ready does it
record `active` and open profile admission. A missing/failing bar remains the
named degraded condition from SPEC 0005, but its bounded supervisor identity is
still recorded so teardown can stop it. The direct entry is the only supervisor
owner; if it dies during `preparing` or `active`, reconciliation freezes
admission and tears down every recorded helper identity rather than recreating
one or advancing the session to `active`.

There is at most one non-closed Helm session claim per UID while the ADR 0011 /
SPEC 0005 publication step may have changed per-UID state. A second
login fails before compositor start, environment-boundary invocation, target
start or profile launch. It may remove a
prior record only by completing its state-specific teardown. For `claimed`, a
boot mismatch or stale entry PID/start-time alone authorizes the transition to
`admission-frozen`, with the canonically absent compositor/helpers carried
forward; `helpers-stopped` and cleanup delegation are then no-ops before
`closed`. For `preparing` or later, stale entry identity authorizes a reconciler
to freeze admission, but compositor and helper identities must each be
revalidated and stopped or proven dead by the mode-specific teardown before the
claim can close. Permission, parse, manager or `/proc` uncertainty preserves
the claim. A teardown or reconciler for session A may mutate only a record whose
session id and observed sequence still equal A; it cannot clear, close or adopt
a later claim.

This exclusion is a prerequisite for SPEC 0013's fresh child launch, not a
definition of how global environment values are published. ADR 0011/SPEC 0005
own the mandatory import; #133 owns only compare-and-restore/cleanup semantics
for values this session wrote. A valid session claim and session environment
predate a later launch;
refusing that launch creates no new claim/publication and does not roll prior
session state back. No M1 code may treat this spec as authority for D-Bus
activation or existing-owner Qt behaviour.

### 4. Launch preparation, adoption and lease transfer

#### Common pre-exec order

For either ownership mode, a launch performs these boundaries in order:

0. Complete SPEC 0013's desktop admission preflight and retain its immutable
   desktop snapshot plus held executable and working-directory descriptors.
   `DBusActivatable=true` and every malformed/unsupported entry refuse here,
   before generation-store open/selection, lifetime-owner/gate creation,
   process/lifecycle lease or launch-record creation, scope/group creation,
   environment publication, target execution or application-directed bus
   traffic. A pre-existing valid session claim is independent prior state.
1. Before creating anything, the sole per-UID lifecycle coordinator samples
   one absolute `CLOCK_MONOTONIC` deadline ten seconds in the future. Under its
   mutex and `lifecycle.lock`, it requires the exact current service
   incarnation's `pidfd-health` to be `healthy`, validates the preparation
   inventory, counts only `coordinator-owned` records, and computes the full
   namespace credits. It atomically publishes/fsyncs one new record only if the
   active count is below 128 and two entry/8192-byte credits remain. That record binds the deadline,
   coordinator incarnation, session/request/desktop, already-generated
   launch/lease ids and gate-token digest before any channel or supervisor can
   exist; expiry wins equality. It then creates distinct plan, gate and
   gate-receipt acknowledgement channels and exactly one inert lifetime-owner
   supervisor. The supervisor remains parent-reapable, has a parent-death
   fail-safe while ownership is local, holds no user profile code, and cannot
   pass its gate. Before process-lease creation, the coordinator atomically
   updates/fsyncs the reservation with that exact supervisor PID/start-time.
   The absolute deadline is copied to the launcher and inherited by the
   supervisor; neither resamples it. It bounds all plan/gate preparation,
   armed or unarmed, through durable gate-arm receipt, acknowledgement and
   token delivery; the two-second armed limits remain additional tighter
   packet deadlines. Expiry cancels the request, closes the channels and makes
   the gate-closed supervisor exit even if the launcher stays live. Every
   following publication/transfer/authorization/arm/token boundary revalidates
   the durable record, the same healthy service incarnation and `now < deadline`
   under the coordinator and applicable lifecycle lock.
2. With `lifecycle.lock` held before SPEC 0011's shared `activation.lock`,
   resolve and fully validate `current`; copy and digest-validate SPEC 0013's
   bounded catalogue/profile bytes against the held desktop snapshot and
   opened generation, and create/fsync a process lease for the supervisor's
   exact PID/start-time/boot/UID identity. Release both locks. Profile parsing,
   evaluation, base-environment merge and bounded plan encoding happen outside
   both locks; no later step rereads mutable sources.
3. Deliver the complete bounded plan and its executable/cwd/generation
   descriptors exactly once over SPEC 0013's authenticated plan channel.
   Receipt does not authorize child creation. Under `lifecycle.lock`, rescan the bounded
   session/request index and atomically persist/fsync only the unique winner's
   sequence 1 `preparing` with `exec-open no` and
   `lease-kind process` plus exactly those identities. Only after this point may
   ownership adoption be requested. A concurrent loser closes its plan/owner
   and releases only its own process lease.
4. Establish the selected ownership mode and verify the exact owner PID is in
   it. For systemd, capture the verified cgroup path/device/inode as well as the
   invocation. Under the shared generation lock, atomically replace the process
   lease with a transferred lifecycle lease bound to the launch id and verified
   ownership evidence, and fsync the leases directory. Successful transfer
   consumes all launcher-side release authority.
5. Durably record `adopted`, `lease-kind lifecycle`, the verified ownership
   incarnation and cgroup evidence while `exec-open no`. Next atomically record
   `authorized/exec-open yes` and fsync it; only then may the launcher arm and
   eventually send the unique gate token. On valid gate-arm receipt before the
   shared deadline, the supervisor validates the related launch/lifecycle
   lease and, under `lifecycle.lock`, compare-and-swaps/fsyncs the preparation
   record to `supervisor-received`. It then sends the exact authenticated
   acknowledgement defined by SPEC 0013. The coordinator reopens and validates
   that durable receipt, unlinks/fsyncs it, retires its in-memory handle exactly
   once, and only then sends the token. Cancellation races on the same record
   state, so one side wins and no crash can leak or release the slot early.
   `authorized` is accepted permission, not an application-liveness claim.
6. On receiving the token, the supervisor acquires `lifecycle.lock`, records
   `starting`, fsyncs it, consumes the unique starting permit and creates exactly
   one application child with SPEC 0013's exact `clone3(CLONE_PIDFD)` and
   error-pipe protocol. Only
   zero-byte CLOEXEC EOF followed by a non-readable child pidfd and exact
   `/proc/<pid>/exe` device/inode match to the pinned ELF proves successful
   Exec and permits the durable `application-running` transition. EOF alone,
   a readable pidfd, pre-Exec death, identity mismatch, a failure frame,
   malformed/partial frame, timeout or child loss never records
   application-running and enters terminal cleanup. The supervisor never exec-replaces itself and remains lifetime
   owner/subreaper through the mode-specific drain sequence. Reconciliation
   never reopens a gate or repeats application-child creation.

The launcher-side `PreparedSelection` owns generation identity, held evidence,
the process lease, its sole release capability and a close-only gate endpoint,
but no gate-send/child-create capability.
Lifecycle transfer consumes it while atomically replacing the same lease and
returns a non-clonable `TransferredExecAuthorization` with the sole gate-send
capability and no process-lease release authority. Dropping the consumed value
cannot unlink the lifecycle lease. A supervisor-only, non-serializable
`StartingExecPermit` exists only after durable `starting` and permits exactly one
application-child creation.

The transferred lease is a versioned canonical record in SPEC 0011's leases
directory. Generation GC recognizes it as protecting its named generation but
never unlinks it based only on PID liveness. The activation registry is the
only release capability. A missing, malformed or mismatched associated launch
record makes the lease uncertain and protected, not stale.

It replaces the original lease at the same opaque filename with this byte
order, using SPEC 0011's no-follow atomic-replace and directory-fsync rules:

```text
helm-generation-lifecycle-lease-v1
generation <generation-id>
launch <launch-id>
pid <positive-u32>
start-time <positive-u64>
boot-id <canonical-uuid>
owner-uid <u32>
owner-kind <systemd-scope|process-group>
unit <none|helm-launch-<launch-id>.scope>
unit-invocation <none|lowercase-32-hex>
process-group <u32>
cgroup <none|normalized-relative-cgroup-path>
cgroup-device <u64>
cgroup-inode <u64>
```

All common fields equal the prior process lease and launch record. The
ownership-specific fields, including cgroup identity, obey the same
systemd/direct restrictions as the launch record. A crash before the atomic
replacement leaves the conservative process lease and a gate-closed owner; a
crash after it leaves the lifecycle lease. There is no filesystem state in
which the selected generation has no lease.

Failure before transfer keeps the gate closed and kills/reaps the inert owner.
Before final `preparing` publication it releases the process lease and creates
no launch record; after that publication it takes the explicit external
`terminal/failed` bypass with `lease-kind process` and then releases only that
process lease. If the launcher itself dies, the parent-death fail-safe
prevents pre-transfer exec and reconciliation performs the same cleanup.
The independent lifecycle reconciler scans every durable preparation record at
least once per second, including unpublished owner/process-lease state and
durable `preparing`, `adopted` or `authorized` launches with their actual lease.
On service start/restart admission remains frozen while, under
`lifecycle.lock`, it validates all bounded records and reserved identities,
reconstructs the active count and the separate namespace-credit accounting
from `coordinator-owned` records, `supervisor-received` receipts and every
temporary/unsafe entry, safely retires valid receipts, and cancels expired
records through the exact recovery row. Malformed, uncertain,
over-128-active, over-512-entry or over-2-MiB inventory fails closed; only a
complete valid scan permits admission to reopen.
Admission's atomic durable reservation therefore permits at most 128 combined
preparations per UID and refuses before creating a channel, supervisor, lease
or launch at the limit.
The supervisor's ten-second epoch remains live through authenticated token
receipt and makes a live-but-wedged launcher lose its gate-closed owner. If
expiry occurs after durable receipt removal but before token receipt, the
absent-preparation `authorized` record takes the same gate-closed
`terminal/failed` recovery row. The periodic reconciler then applies the matching recovery
table row, including exact stale-identity/total-emptiness proof, terminalization
or retained uncertainty, lease handling and preparation-record removal. A late launcher
continuation must revalidate the same record, live owner and absolute deadline at
every publication/transfer/authorization boundary and therefore cannot revive
or advance it.
Failure or launcher death after transfer never creates a replacement: gate EOF
leaves profile code unexecuted, a durable authorization without `starting` is
aborted by reconciliation, and a `starting`/`application-running` owner is
reconciled in place. The
total record/lease recovery table in section 5 handles crashes between the
terminal record, lease removal and record collection.

#### systemd ownership

The systemd path creates exactly
`helm-launch-<launch-id>.scope` in `app.slice`, passing the inert owner PID for
adoption. It has no `Wants=`, `Requires=`, `BindsTo=` or `PartOf=` relationship
to `helm-session.target`. Before lease transfer Helm reads the unit's
`InvocationID` and `ControlGroup`, verifies the unit name and current invocation
match the reply, and verifies the exact owner PID's `/proc` cgroup membership.
It opens the normalized cgroup below `/sys/fs/cgroup` without following
symlinks, verifies membership there, and stores its path plus `st_dev`/`st_ino`
in both lifecycle lease and launch record. The lowercase 32-hex invocation id
and this cgroup identity together are the durable scope incarnation; a name or
path alone is never identity.

The lifetime owner stays in that scope until all other attributed members have
exited, then writes/fsyncs `owner-drained` with the final result and exits
without releasing the lease. Normal owner exit is expected only after that
condition, but lease release still waits for an external reconciler's
independent recursive-subtree proof;
an exit status alone is not emptiness. If the owner dies abnormally, the
transferred lease remains protected under the same rule. If the old unit has
unloaded, stale owner identity plus descriptor-
relative `ENOENT` for the recorded cgroup while no same-name unit exists is
emptiness proof. If the path still names the recorded device/inode, an empty
root `cgroup.procs` is **not** proof because descendants may occupy nested
cgroups. Helm instead opens `cgroup.events` descriptor-relatively with
`O_NOFOLLOW`, revalidates the cgroup path/device/inode before and after reading,
and requires cgroup v2's recursive `populated 0`.

The events parser accepts LF-terminated ASCII lines of exactly
`<lowercase-key> <unsigned-decimal>`, with one space, no CR/blank/duplicate key,
and requires exactly one `populated` value of `0` or `1`; additional unique
kernel keys with canonical unsigned-decimal values are permitted. Only
`populated 0` proves the complete recorded subtree empty. `populated 1`, a
missing/unsupported v2 events file, malformed/changed content, identity race,
changed inode, manager unavailability, unreadable hierarchy, or same-name unit
with a different invocation is live/collision/uncertainty: Helm does not
signal, adopt, stop, release or delete either object.

#### no-systemd ownership

When no usable user manager exists, the inert owner becomes the leader and
subreaper for a fresh process group whose id equals its PID. Helm verifies that
identity before lease transfer. The owner starts one direct child, tracks all
reparented descendants, and can prove only that every *other* attributed group
member has exited; its own membership means it never declares the whole group
empty. After that other-member proof it locks the exact record, writes/fsyncs
`owner-drained` with the final result, releases the lock and exits without
releasing the lifecycle lease. It performs no group-empty or lease-release
claim about itself.

An external lifecycle reconciler—never that supervisor—then revalidates the
same record/lease, proves the recorded supervisor PID/start-time stale and the
now-total process group absent under section 5's anti-reuse rules, writes/fsyncs
`terminal` with the preserved legal `owner-drained` result, unlinks/fsyncs the
lease, and collects the record in that order. If the supervisor died before
`owner-drained`, only section 5's explicit failed/lost terminal bypass applies.
A crash before `owner-drained`, after its fsync but before supervisor exit,
after exit but before `terminal`, or after each release boundary is handled by
the same table. Live supervisor, any other member, group-id reuse or uncertainty
retains the record and lease. Each wait, TERM and KILL phase is deadline-bounded.

Changing session/process group, double-forking beyond the tracked subreaper, or
otherwise detaching is unsupported. Detection reports `lost`/uncertain,
retains the current nonterminal record and lease, emits the named no-systemd
degradation, and returns without waiting forever; it may not write
`owner-drained` or a terminal result. Absence of the owner alone is never sufficient release proof for a
direct transferred lease. The no-systemd path makes no claim that an
application survives logout or user-manager loss equivalently to a systemd
scope; logout attempts to terminate every direct launch group within the
session deadline.

### 5. Launch state machine and reconciliation

The launch state machine is:

```text
preparing -> adopted -> authorized -> starting -> application-running
                                      |                     |
                                      +------> owner-drained <+
                                                   |
                                                   v
                                               terminal -> collectible

external terminal bypasses after total-emptiness proof only:
preparing | adopted | authorized -> terminal/failed
starting | application-running   -> terminal/lost
```

`authorized` durably permits the unique token but does not claim child creation or Exec.
`starting` says the one atomic child creation was attempted. `application-running` is permitted
only after SPEC 0013's EOF-plus-pidfd-plus-executable-identity proof. A normal
live supervisor in either ownership mode must cross `owner-drained` before it
exits; an external reconciler alone then advances to terminal after independent
total scope/group emptiness proof. If the supervisor is already dead before it
could write `owner-drained`, the reconciler may bypass directly to terminal
only after that same total-emptiness proof: `preparing`, `adopted` or
`authorized` maps to `failed`, while `starting` or `application-running` maps
to `lost`. These are terminal crash classifications, not fabricated drain
witnesses. Failure from any nonterminal state never moves backward and never
reopens a gate or repeats application-child creation.

`collectible` is a predicate, not a persisted backward transition: owner
emptiness is proven, `terminal` is fsynced, the actual process or lifecycle
lease is unlinked and the leases directory fsynced, and only then may the
record be removed and the launches directory fsynced. A typed record has the
additional matching-session-closed precondition from section 2. A crash after
terminal
but before lease removal retries removal; a crash after lease removal but
before record removal recognizes the terminal/missing-lease case below. If any
proof is uncertain or an operation fails, the remaining record/lease stays.
TERM refusal is bounded; it leaks safe evidence rather than authorizing
deletion.

Preparation reconciliation runs first. A `coordinator-owned` record is the
authoritative counted reservation until cancellation or the supervisor's
durable receipt CAS; a `supervisor-received` record is a non-counting receipt
that may be unlinked/fsynced after its exact launch/lease/supervisor relation is
validated. A launch through `authorized` may have the matching preparation in
either state during that bounded handoff. `starting` and all later states
require the preparation record absent; a leftover valid
`supervisor-received` record is a cleanup receipt, while a
`coordinator-owned` one there is fatal/uncertain. No reconciler sends an arm,
acknowledgement or gate token. The record's boot mismatch or
`now >= deadline-ns` selects cancellation, with equality expired; uncertainty
retains its active or namespace credits. The once-per-second scan includes
received receipts and every preparation temporary, not only active records.

Launch reconciliation then runs at activation-root open, before a restarted `helm-wm`
accepts a launch, and after user-manager reconnect. It opens the referenced
lease without following links and dispatches on durable record state plus the
lease's actual parsed format; `lease-kind` is an expected value whose mismatch
selects a recovery row rather than being guessed away:

| Durable record | Actual lease | Recovery |
|---|---|---|
| `preparing`, `lease-kind process` | matching process lease | The gate cannot have opened. Revalidate and terminate/reap the owner and any exact deterministic scope/group already created, prove emptiness, write `terminal/failed`, unlink/fsync the process lease, then retain typed evidence until session close. Live/uncertain or same-name/different-incarnation evidence is retained. |
| `preparing`, `lease-kind process` | matching lifecycle lease | Transfer committed before the adopted record. Use the lease's complete invocation/cgroup or group evidence to stop only that gate-closed ownership, prove it empty, write `terminal/failed` with `lease-kind lifecycle`, release/fsync the lifecycle lease, then retain it. It is never completed or executed. |
| `preparing` | lease absent | No profile was authorized. Revalidate and terminate any exact gate-closed owner/ownership; after proven total emptiness bypass `owner-drained`, write `terminal/failed`, and retain the record until session close. Uncertainty retains it. |
| `adopted` | matching lifecycle lease | No authorization was durable. Abort, terminate the exact ownership, prove total emptiness, bypass `owner-drained`, write `terminal/failed`, release the lease and retain the typed record. Reconciliation never arms the gate. |
| `authorized` | matching lifecycle lease | Arm/token delivery is ambiguous but no repeat is allowed. Terminate the exact ownership, prove total emptiness, bypass `owner-drained`, write `terminal/failed`, release the lease and retain the typed record; never resend or create a child. |
| `starting` or `application-running` | matching lifecycle lease | Reconcile the same live owner/scope/group in place and never create another application child. A live pidfd-capable supervisor obtains the exact result mapping, drains other members and records `owner-drained/<result>`. On SPEC 0013's unexpected actual pidfd-operation failure it instead preserves this legal nonterminal tuple, closes launch capabilities and exits without `owner-drained`. Only after that supervisor is proven stale may the external reconciler terminate the exact complete scope/group; only proven total emptiness permits the explicit bypass `terminal/lost`. The reconciler never invents `owner-drained`; any live member or membership/identity uncertainty retains the tuple and lease. |
| `owner-drained` | matching lifecycle lease | Require its legal non-`none` result and preserve it. Never release while the recorded supervisor is live. After it is proven stale and the recorded cgroup subtree or direct group is proven totally empty without reuse, an external reconciler writes `terminal/<same-result>` and releases/fsyncs the lease; typed record collection waits for session close. Any other member or uncertainty retains both. |
| `terminal`, `lease-kind process` | matching process lease | Re-prove the gate-closed owner/ownership empty and unlink/fsync the process lease. Collect only when the session-retention precondition permits. |
| `terminal`, `lease-kind lifecycle` | matching lifecycle lease | Re-prove complete ownership empty and unlink/fsync the lifecycle lease. Collect only when session retention permits. |
| `terminal` | lease absent | Re-prove recorded ownership empty, retain typed idempotency evidence until session close, then remove/fsync the record. Absence is not lease uncertainty in this one state. |
| `adopted`, `authorized`, `starting`, `application-running` or `owner-drained` | process lease, absent lease, malformed lease, or identity mismatch | This inventory is not producer-reachable and might expose an unprotected execution. Report fatal/uncertain; do not signal, adopt, release or delete. |
| any state / expected `lease-kind` | every canonical or malformed actual lease format, absence, identity relation or cross-kind combination not matched by an earlier row | Exhaustive fatal/uncertain default. This includes `preparing` with expected lifecycle, terminal cross-kind leases, and malformed or identity-mismatched Preparing/Terminal evidence. It authorizes no signal, adoption, release, record deletion or generation deletion. |

For every row, a matching live systemd scope means unit name, invocation,
recorded cgroup path/device/inode, owner identity and current membership all
agree. An unloaded old scope is empty only under section 4's durable-cgroup
proof. A direct supervisor may prove only that no *other* attributed member
remains and then record `owner-drained`; only an external reconciler may prove
the supervisor stale and the total group absent using PID/start-time, UID,
boot, group and tracked-descendant checks. Detachment, permission failure
or disagreement retains the evidence. A PID, unit name or session id that has
been reused never authorizes action; stale identity may prove death but never
identifies a replacement owner. Whenever evidence is provably terminal, the
table reaches collection and cannot consume the 4096-record quota forever.

A WM crash/restart reconciles these activation records independently of its
runtime ledger snapshot. It does not restart a profile application. Whether
river replays existing windows and how SPEC 0003 rebuilds rectangles remain its
unresolved M2 question; neither outcome changes the ownership record.

After user-manager reexec/restart, Helm re-queries unit properties rather than
trusting cached object paths. It may restore target-owned session helpers only
while the same session claim, entry and compositor are live and `active`. It
never recreates a profile application to make a missing scope look healthy and
never migrates a live systemd record onto the direct path.

Malformed record names/content, wrong type/owner/mode, symlinks, stale-name
collisions and uncertain liveness authorize no signal, adoption, record
deletion or lease release. Read-only reconciliation of independent valid
records may continue, but a destructive registry-GC pass stops; valid unrelated
state is left intact.

### 6. Unit graph and teardown

Every Helm session helper wanted by `helm-session.target` also declares
`PartOf=helm-session.target` so stopping the target really propagates a stop.
It retains any required relationship to `graphical-session.target`.
`helm-bar.service` is ordered `After=helm-wm.service`, so inverse stop ordering
stops the bar first. `helm-wm.service` uses `Restart=always`: while the same
compositor and active session target exist, an unsolicited clean exit is not a
logout and must not leave river unmanaged. User-requested logout exits river;
the entry freezes admission and stops the target, which prevents the WM restart.
The existing start limit, restart-prevent exit statuses and exactly-once abort
unit remain in force.

While this specification is Draft, the shipped WM/bar unit sources still name
only `PartOf=graphical-session.target` and are explicit non-conforming/red
evidence for A12. Functional unit changes require this spec to become Accepted
and the executable unit-graph test to be observed failing first. `INSTALL.md`
must describe the shipped behavior, not this candidate, until then.

Teardown is idempotent and ordered:

1. Durably transition the matching session to `admission-frozen`; reject all
   later launch requests for it.
2. Stop helpers according to the recorded mode. For `systemd`, stop
   `helm-session.target` and verify every target-owned helper inactive; do not
   stop, kill or attach any `helm-launch-*.scope` profile launch. For `direct`,
   disable both restart loops, send bounded TERM then KILL to the revalidated
   bar supervisor/group before the WM supervisor/group, reap them, and verify
   both recorded groups empty. For `none`, there was no helper start and the
   proof is immediate. Only the matching successful mode-specific proof may
   record `helpers-stopped`; uncertain identities leave teardown frozen for a
   later reconciler.
3. Reconcile profile launches. Systemd scopes remain independent and release
   only after exact emptiness proof. Because the direct path promises no logout
   survival, teardown sends bounded TERM/KILL to every revalidated direct
   lifetime-owner group, records terminal only after proven emptiness, and
   preserves uncertain records/leases. Lingering-on may leave systemd scope,
   record and lease live. Lingering-off may let logind kill them; the next
   reconciliation collects only after proving death.
4. Record `cleanup-delegated`, then perform SPEC 0005's compositor and
   runtime-tree cleanup. ADR 0011/SPEC 0005 own mandatory publication; only an
   Accepted #133 amendment may authorize compare-and-restore of manager/bus
   values this session wrote. Until then, cleanup makes no restoration claim.
   Removing `$XDG_RUNTIME_DIR/helm` cannot remove any asset
   promised to a surviving profile launch.
5. Record `closed`; collect that session's terminal typed records whose leases
   are already durably released, then remove the session record only after its
   close is durable and the caller still owns the same session id and sequence.
   Live/uncertain launch records remain and an old-session request id can never
   be admitted by a later session.

The whole entry teardown retains SPEC 0005's 15-second deadline. At expiry the
entry may return, but it may not forge a later state, delete an uncertain
record, release a live/uncertain lease, or claim cleanup completed. The durable
frozen or later record lets a subsequent reconciler resume. A source check is
insufficient: an executable unit-graph fixture must demonstrate actual stop
propagation and ordering before environment cleanup while an independent app
scope remains active.

### 7. Restart and concurrency outcomes

- **WM restart:** reconcile existing launch ids before accepting requests;
  preserve the same live owner/scope/generation and execute nothing again.
- **User-manager reexec/restart:** adopt the same verified unit invocation at
  most once; classify vanished/inconsistent state without duplicate execution;
  restart only session helpers for the still-live active claim.
- **Logout, lingering on:** target helpers stop, live app scopes and their
  records/leases remain until real exit.
- **Logout, lingering off:** logind may reap app scopes; later reconciliation
  proves stale identities before terminal transition and collection.
- **Concurrent same-UID login:** the first non-stale session record wins. The
  second mutates no global manager/bus state, starts no target/helper/profile,
  and cannot be cleared by the first session using a stale sequence.
- **No systemd:** direct groups receive bounded teardown and no survival
  equivalence promise. Uncertain detachment retains evidence and never hangs.

### 8. Deferred lifecycle status snapshots

This section is design input for #117 only and is not an M1 public surface.
There is no desktop-launch event or lifecycle subscription in the #133 typed
relation. Any future lifecycle-status subscription follows ADR 0004's
connection boundary. A new server process creates a fresh opaque 128-bit
incarnation and a sequence beginning at 1. After the mandatory connection
handshake and subscription, its first lifecycle frame is one full current
snapshot derived from durable records, tagged with that incarnation and
sequence. Later changes are monotonic within that incarnation.

Snapshot construction is all-or-nothing. While holding `lifecycle.lock`, the
server first removes only discardable unpublished artifacts, then scans the
complete bounded session and launch inventories and validates every final name,
record and reserved entry. Still holding that lock, and acquiring SPEC 0011's
`activation.lock` only after it when both are required, the server opens every
canonical launch record's actual referenced lease without following links and
cross-validates the record, actual lease kind and identity, and actual
systemd-scope/cgroup or direct-owner/group result through section 5's
classification. A nonterminal launch record is snapshot-valid only with a
present canonical lease of its recorded expected kind, matching identity and
consistent readable ownership evidence. A retained `terminal` record may have
the table-authorized absent lease after emptiness proof and is then emitted as
terminal idempotency evidence. Any other absent/malformed/cross-kind or
identity-mismatched lease, or missing/inconsistent/unreadable ownership
evidence, fails the complete snapshot; the record is never emitted as healthy.

If any final record or reserved entry is malformed, has unsafe metadata, an
inventory bound is exceeded, or any record/lease/ownership cross-validation
fails, the snapshot attempt fails closed: it emits neither a partial “full”
frame nor any later lifecycle delta for that subscription. It reports
fatal/uncertain snapshot failure through the eventual SPEC 0006 transport
contract and never silently omits the bad evidence. Internal read-only
reconciliation of independent valid records may continue as section 5 permits,
but its result is not a full/current public snapshot. A client can obtain a
snapshot only through a fresh successful scan after the malformed, over-bound
or cross-invalid inventory has been repaired externally.

EOF destroys the subscription. A client reconnects, handshakes and subscribes
again; the new server sends another current full snapshot. It never resurrects
the old connection, never mixes an old-incarnation delta into the new one, and
does not relaunch an application to reconstruct status. Durable records provide
current truth only under this spec. For the #133 typed launch relation, Draft
SPEC 0013 §8 and the Draft SPEC 0006/#117 amendment define request identity,
current-state query, reply states and the 65,536-byte frame cap; retry recovery
uses `GetDesktopLaunch`, not this deferred broad subscription. The broader
all-launch full snapshot remains blocked until #117 defines pagination/cursors
compatible with SPEC 0007's one-frame queue. It cannot be acceptance evidence
for the typed relation in its current unpaged form.

## Acceptance criteria

Each row is a Given/When/Then contract. Tests are added red-first only after
this spec is Accepted.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a launch selected on generation N, when N+1 becomes current and generation GC races launch or teardown, then the process reads only N and N is retained until the exact transferred lifetime ownership is proven empty. | |
| A2 | Given every legal/illegal launch tuple; 128 reservations and wedged receipts; 511/512/513 credits; every deadline equality; all ten temporary create/update crash points; service death before/after health probing, probe reservation, supervisor creation/PDEATH setup, placement, contained rename/root-fsync, health-failed rename/root-fsync, probe-failed rename/root-fsync, containment removal and healthy rename/root-fsync; every health/probe pair including probing-without-probe, failed+contained, probing+failed and healthy+probe; stale/live/reused creator/owner/unit/group/cgroup evidence in systemd/direct mode; concurrent reservation versus healthy-to-failed CAS; two and many already-running supervisors failing before/during/after the first CAS; coordinator death separately before/during/after the winner CAS/root-fsync and before/during/after a loser's read-only committed-freeze validation; the first named launch cleaned while later failing launches remain live, stale or uncertain; and actual initial-poll, timeout-signal, result-wait/final-reap failures, when recovery runs, then health is the sole failure commit authority and accepts only the exhaustive pair/transition grammar. The first supervisor commits; same-incarnation losers validate its token/incarnation without overwrite, preserve their own tuples/leases and exit without owner-drained, while invalid losers retain authority fail-closed. Probe reservation precedes owner, PDEATH/barrier contains pre-publication crashes, and no two-file atomicity or old-probing success is inferred. Failed/old probing/old healthy changes to a new sequence/incarnation only after exact stale-coordinator/probe cleanup and a complete bounded scan has reconciled every nonterminal typed ownership, not only the named launch; any live/uncertain later loser blocks reprobe. Failed health blocks only typed desktop admission with structured refusal while the ordinary control plane remains available. Limits hold, a racing prepared launch cancels, only external total emptiness writes terminal/lost, and no gate, child, unleased profile, duplicate application or quota leak results. | |
| A3 | Given systemd and direct children that exit zero, exit nonzero, die by signal, ignore TERM or leave uncertain membership, plus a supervisor crash in each pre-authorization, starting, application-running and owner-drained boundary, when teardown runs, then the exact result table maps both exit statuses to `exited`, signal to `failed` and uncertainty to `lost`; a live supervisor alone proves other-member absence and fsyncs owner-drained, while an external reconciler alone proves stale supervisor plus recursive cgroup or total direct-group absence and either preserves that result at terminal or takes only the specified `terminal/failed|lost` bypass. Every other state/result/lease combination is rejected, and live/reused/uncertain evidence remains under bounded waits. | |
| A4 | Given systemd-scope and direct-process-group typed launches when `helm-wm` crashes after each durable `preparing`, `adopted`, `authorized`, `starting`, `application-running`, `owner-drained` and accepted/refused `terminal` mutation before reply, when reconciliation and client reconnect complete, then same-key `GetDesktopLaunch`, scoped by the connection UID plus session/request ids, returns the total `DesktopLaunchCurrent` DTO with the same request/launch/desktop/generation/profile, exact current sequence/state/disposition/launch-mode/execution-evidence/result/refusal within one frame. It exposes no owner/scope/group field, repeats no owner/gate/child creation, and promises no desktop-launch event/subscription or bounded wait. The broader all-launch snapshot remains #117-only. | |
| A5 | Given user-manager reexec/restart with a live scope, an unloaded old scope whose recorded cgroup path is absent, the same recorded cgroup inode reporting recursive `populated 0` or `1`, an empty root `cgroup.procs` with a populated nested child cgroup, a reused cgroup path/inode, or a same-name/different-invocation scope, when reconciliation runs, then only matching invocation plus durable cgroup identity is adopted once, only stale-owner plus absent old cgroup or verified `cgroup.events` `populated 0` permits collection, nested/live/inconsistent/reused evidence causes no duplicate launch or lease release, and only target-owned helpers may restart while the same session claim remains active. | |
| A6 | Given a running target and independent profile scope, when logout stops `helm-session.target`, then an executable unit-graph fixture proves the bar stops before the WM and every helper stops before environment cleanup while the profile scope remains untouched. | |
| A7 | Given equivalent live launches with lingering off and on, when logout and later reconciliation run, then off permits logind cleanup followed only by proven-stale record/lease collection, while on preserves the live scope, record and lease until real exit. | |
| A8 | Given faults after temporary creation, during/after temporary write/fsync, before/after no-replace rename and parent fsync for session claim, launch create and record update, plus an early same-boot entry crash and concurrent same-UID claimants, when recovery holds the lifecycle lock, then discardability is decided only from exact basename grammar, current-UID no-follow exact-0600 regular-file metadata, the 4096-byte size bound and the applicable inventory's final-record condition; every artifact satisfying those conditions is discarded and fsynced without payload decoding or replay even when empty, partial, non-UTF-8 or noncanonical, every malformed final or malformed-reserved name/metadata fails closed, no partial final appears, natural crash temporaries cannot permanently block claim/GC or exhaust quota, exactly one healthy session claim wins before compositor/global-environment/helper/profile mutation, and a stale id/sequence cannot clear it. | |
| A9 | Given no usable systemd user manager, when the entry follows SPEC 0005's no-manager path, starts direct helpers, admits a profile, and the child exits zero/nonzero, dies by signal, detaches, or logout/crash occurs before/after owner-drained and supervisor exit, then helper teardown remains bar-before-WM; the exact result mapping is preserved, the live profile supervisor alone may write owner-drained and never calls its own group empty or releases its lease, while the external reconciler alone completes total-group proof and either preserves owner-drained's result or takes only the specified failed/lost terminal bypass before release. Detachment/reuse is degraded/uncertain, and no systemd survival equivalence is claimed. A fake publication fixture proves only ordering and is never session, portal, D-Bus, doctor or ADR 0011 evidence. | |
| A10 | Given wrong owner/mode/type, a symlink, malformed/reserved entry, stale boot/PID/start-time, reused PID, stale unit name with a new invocation, or a conflicting sequence, when reconciliation or GC runs, then it refuses signal/adoption/deletion for that evidence and leaves valid unrelated state intact. | |
| A11 | Given teardown work still live or uncertain at 15 seconds, when the entry deadline expires, then the entry may return but the durable state does not advance falsely, every affected record and generation lease remains, and runtime removal has deleted no promised application asset. | |
| A12 | Given the shipped source units and a live user-manager fixture, when restart, target stop and abort paths are exercised, then both views agree on `Restart=always`, `PartOf=helm-session.target`, inverse bar-before-WM stop order, profile-scope independence and exactly one abort execution after start-limit exhaustion. | |
| A13 | Given a desktop entry declares `DBusActivatable=true`, with no application owner, with an existing owner, and with an owner racing admission, when SPEC 0013 desktop preflight evaluates it, then every case refuses solely from the held key before generation-store open/selection, lifetime-owner/gate, process/lifecycle lease or record, systemd scope/group, launch-attributable environment publication, target exec or application-directed bus query/call; an already-valid session claim/environment is independent prior state. Given a plain `DBusActivatable=false`/absent Exec entry, no existing-owner probe or refusal occurs and the lifecycle may admit one fresh process. | |
| A14 | Given each safe crash point before/after owned-directory creation, child fsync, parent fsync, first `lifecycle.lock` exclusive creation, lock fsync, activation-root fsync and `launches/` creation, plus concurrent same-UID initializers and separate unsafe-existing-object fixtures, when initialization retries, then every safe inventory converges from absence to verified current-UID exact-0700 directories and one persistent current-UID zero-length exact-0600 lock inode, all contenders revalidate and acquire that same inode before lifecycle mutation, each created component's child and parent durability precedes dependent mutation, and every unsafe collision fails closed without unlinking, replacing or repairing it. | |
| A16 | Given no usable systemd user manager and separate direct-helper fixtures for an unrequested clean WM exit, failed WM or bar exit, clean bar exit, WM status 69 or 78, refusal of the next WM or bar start by the five-in-thirty limit, WM readiness failure, and entry death during `preparing` or `active`, when the direct supervisors or reconciler handle them, then WM clean/failure and bar failure use the one-second bounded restart policy, clean bar exit is not restarted, WM restart-prevent/limit/readiness failure enters idempotent admission-freeze and teardown exactly once, opens no new profile admission and never advances `preparing` to `active`, bar limit exhaustion remains degraded without tearing the session down, and entry death in either state freezes admission, recreates no helper, never resumes or advances `active`, and tears down only revalidated recorded groups in bar-before-WM order. | |
| A17 | Given separate fresh-bus real-session fixtures, when dual import completes before target/helper/client start, then exact manager readback proves the systemd values, the first test sentinel activation proves exactly its inherited D-Bus values/list/nonce and exits, and a separately fresh portal activation proves only its bounded functional result. When only D-Bus import is omitted, the sentinel exact-value fixture fails; the fresh portal fixture reports only its observed functional failure/success without diagnosing a raw missing variable; and a production doctor facing an already-owned functioning portal labels that observation weaker/inconclusive rather than claiming import success or failure. Fake/no-op publication and an existing owner are never exact D-Bus-environment evidence. | |

#### Deferred #117 design fixture — not an acceptance criterion

After #117 defines a paginated SPEC 0007-compatible subscription topology,
its tests should cover a complete bounded inventory containing malformed,
cross-kind and identity-inconsistent evidence, require all-or-nothing snapshot
failure, and require a fresh successful scan after repair. This former A15
concept is intentionally not part of SPEC 0012 acceptance and creates no M1
subscription or event promise.

## Budgets and limits

| Item | Limit |
|---|---|
| Entry teardown | 15 seconds total; expiry preserves uncertain state rather than extending the deadline |
| Desktop preparation | one absolute deadline sampled before durable reservation/channel/owner creation, ten seconds through authenticated gate-token receipt; expiry wins equality at ACK/removal/send/receive boundaries; independent two-second armed epochs are tighter limits and do not end the absolute epoch |
| Active preparations | 128 counted `coordinator-owned` preparations per UID, atomically reserved by the sole coordinator; `supervisor-received` is not active-counted but remains namespace-accounted; startup reconstruction and reconciliation scan at least once per second |
| Preparation namespace | 512 total physical/reserved entry credits and 2 MiB physical/reserved bytes, including finals, receipts, create/update temporaries and malformed/unsafe entries; 4096 bytes per file; a new operation reserves two entry/8192-byte credits |
| Launch namespace entries | 4096 total final records plus temporary artifacts per UID activation root |
| Launch namespace bytes | 4096 bytes per entry and 16 MiB total |
| Terminal lifecycle retention | lifecycle lease: 0 seconds after proven terminal emptiness; typed record/idempotency tuple: until matching session is durably closed |

No fixed sleep is a synchronization mechanism. Polls and TERM/KILL phases have
conditions and deadlines. Existing input/render budgets in ARCHITECTURE §4 and
SPEC 0003 are not changed by lifecycle work and may not be blocked on registry,
systemd, filesystem or process operations.

## Failure modes

| Failure | Guard |
|---|---|
| Generation GC wins the handoff race | Shared generation lock through durable process lease, pre-exec transfer to a non-PID-reclaimable lifecycle lease, and A1/A2 |
| Target stop leaves helpers running | Explicit `PartOf=helm-session.target`, inactive verification and executable A6/A12 |
| Launcher/WM/manager crash duplicates an application | One closed gate, monotonic record, incarnation validation and A2/A4/A5 |
| Supervisor counts its own membership as an empty group | Durable owner-drained, supervisor exit, then external total-emptiness proof in A2/A3/A9 |
| Logout kills a profile or drops its generation | Scope independence, freeze-before-stop, proof-before-release and A3/A6/A7/A11 |
| PID or unit name reuse targets an unrelated process | Boot/UID/start-time/invocation/cgroup revalidation and A10 |
| Runtime cleanup removes surviving assets | Persistent records plus sealed-generation-only dependencies and A7/A11 |
| A disconnected point-query client is treated as durable authority | Same-key current-state query and retained idempotency evidence in A2/A4 |
| No-systemd detachment becomes an unowned success | Explicit degraded failure, retained evidence and A9 |

## Open questions

None inside this M1 contract. The intentionally unresolved choices remain with
Draft SPEC 0013/#133 restoration/unsupported-mode seam, #135, ADR 0011 /
SPEC 0005 publication (scheduled by #60 and ordered here for #132),
SPEC 0005 OQ-1, SPEC 0003's river replay experiment, and SPEC 0006/#117's
broader paginated snapshot/history semantics beyond the typed current-state
amendment drafted here. Their resolution must not alter lease-before-exec,
proof-before-release, monotonic reconciliation, or the single-active-session
prerequisite without first amending this specification.
