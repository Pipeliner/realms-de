# SPEC 0012 — Activation launch lifecycle

- **Status:** Accepted (2026-08-30)
- **Milestone:** M2
- **Decisions:** [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Issue:** [#132](https://github.com/Pipeliner/realms-de/issues/132),
  [#168](https://github.com/Pipeliner/realms-de/issues/168)
- **Supersedes / Superseded by:** Supplies the M2 activation-ownership and
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
- Exact environment publication/restoration, `Exec` versus `DBusActivatable`,
  existing-owner Qt behaviour, and D-Bus ownership remain
  [#133](https://github.com/Pipeliner/realms-de/issues/133). This spec requires a
  session claim before global mutation but does not invent D-Bus readback or
  compare-and-swap. M2 admission is deliberately narrower: it accepts only a
  fresh consuming process which can be placed below the lifetime owner. A
  D-Bus activation or existing-owner result is refused before selection,
  process creation, lease creation or record creation until #133 either maps it
  to provable ownership or amends this contract.
- Exact profile assets, target packages, argument vectors and consumer
  behaviour remain [#135](https://github.com/Pipeliner/realms-de/issues/135).
  A launch prepared here treats those inputs as opaque and may use only its
  sealed generation, persistent state, and inherited descriptors.
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
application resulting from that request. It is never a member, wanted unit, or
`PartOf=` dependent of `helm-session.target`.

Every admitted profile launch has exactly one **lifetime owner**. The owner is
a small Helm supervisor created before selection, kept behind an exec gate, and
placed in the launch's scope or process group. After durable authorization it
starts at most one fresh, ownership-compatible profile application and remains
alive while any process Helm can attribute to that launch may dereference
generation content. The application process is not itself the lifetime owner:
it may fork, replace itself, or exit while another owned member remains.

Every launch satisfies all of these invariants:

1. One launch id identifies one lifetime-owner identity, one generation id,
   one manifest SHA-256, one lease reference, and at most one executed profile.
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
inherited descriptor. It may not depend on the control socket, entry/WM PID
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
  session
  .session-claim-<session-id>
  .session-update-<session-id>-<sequence>-<nonce>
  launches/
    <launch-id>
    .launch-create-<launch-id>-<nonce>
    .launch-update-<launch-id>-<sequence>-<nonce>
```

`session` is absent when no Helm session claim exists. `session-id` and
`launch-id` are independently generated opaque 128-bit lowercase hexadecimal
values matching `[a-f0-9]{32}`. A generation lease reference has the same
grammar and names its existing record below SPEC 0011's `leases/` directory; it
is not a mutable generated path.

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

While holding that lock, initialization creates/verifies `launches/` at exact
mode 0700 by the same create, child-fsync and parent-fsync protocol. Its only
recoverable crash states are absent or one valid directory; invalid metadata or
any other object at that name fails closed. The `lifecycle.lock` pathname is
never unlinked or replaced after its first atomic create.

Every record is a current-UID mode-0600 regular file, at most 4096 bytes,
encoded as canonical UTF-8 with one LF after every line, no CR, comments, blank
lines, duplicate fields or extra bytes. Unsigned decimal fields contain no sign
or leading zero except the value `0`. Boot ids use lowercase canonical UUID
text; SHA-256 values use 64 lowercase hexadecimal digits. A `nonce` has the
same 32-lowercase-hex grammar as an id. The four temporary-name forms shown
above are the complete reserved grammar. The id, sequence and nonce of a
temporary are classified only from its basename bytes; decoding its payload is
never part of temporary-name validation or a precondition for discard. If a
temporary is parsed only for diagnostics and happens to contain a complete
canonical payload, a payload id or sequence mismatch may be reported but does
not change discardability and never authorizes replay.

A temporary is a **discardable unpublished artifact** only when recovery holds
`lifecycle.lock`, its basename exactly matches one of those four forms, no-follow
metadata proves it is a current-UID regular file with exact mode 0600, an
`O_NOFOLLOW` open proves its size is at most 4096 bytes, and the applicable
inventory below satisfies its final-record condition. These facts exclusively
decide discardability. Its payload may be empty, partial, non-UTF-8 or otherwise
noncanonical: no temporary is authority, and payload validity or decoded
id/sequence equality is not required for safe discard.
An entry using a reserved prefix but not the exact grammar, or with wrong
owner/mode/type, symlink/special type or size above 4096 bytes, is a
**malformed reserved entry** and fails closed. A malformed final `session` or
`launches/<launch-id>` record likewise always fails closed; the discard rule
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

Each `launches/<launch-id>` record has this byte order:

```text
helm-activation-launch-v1
launch <launch-id>
session <session-id>
sequence <positive-u64>
state <preparing|adopted|running|terminal>
result <none|exited|failed|lost>
owner-uid <u32>
boot-id <canonical-uuid>
generation <generation-id>
manifest-sha256 <lowercase-64-hex>
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
direct-drained <no|yes>
```

`manifest-sha256` equals the digest contained in the validated generation's
`seal`, excluding its LF. `lease-kind process` is written initially and changes
to `lifecycle` in the first durable record after transfer. A systemd record
uses the exact unit name shown, `process-group 0`, and records invocation/cgroup
fields as `none`/`0` only before verified adoption. A cgroup path is the
`ControlGroup` value with exactly one leading `/` removed. It is non-empty,
contains only ASCII components matching `[A-Za-z0-9_.:@%+-]+`, has no empty,
`.` or `..` component, and is resolved descriptor-relatively with no symlink
following below an opened `/sys/fs/cgroup`. Device and inode are captured from
that verified directory and are both positive. A direct record uses both unit
fields and `cgroup` as `none`, all cgroup numeric fields as `0`, and a process
group equal to the lifetime owner's PID. `result` is `none` before `terminal`
and non-`none` at `terminal`. `exec-open yes` means authorization was durably
recorded before the unique gate token could be sent; it never returns to `no`.
`direct-drained` is always `no` for systemd scopes. For a direct group it starts
`no` and may become `yes` only in the terminal transition written by the still
live, revalidated lifetime owner after it has reaped every attributed descendant
other than itself. After that fsynced terminal transition, the owner exits. Only
then may a reconciler release the lease after it validates the witness, the
exact owner identity is stale, and the recorded group is empty. A reconciler may
never synthesize the witness from owner death, an empty observed root group, or
a stale PID.

The `launches/` namespace admits at most 4096 total final-record plus temporary
entries and 16 MiB of their bytes; each file retains the 4096-byte limit.
Before refusing a new launch it removes safe unpublished artifacts and runs the
conservative collection rules below. Live or uncertain records are never
evicted to meet a quota. This registry promises no historical retention: a terminal record is collectible
immediately (zero-second horizon) after its terminal transition is durable and
its lifecycle lease has been durably released. Public history, if any, is a
separate SPEC 0006/#117 store and promise.

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

The systemd path invokes an environment-publication boundary, starts
`helm-session.target`, and verifies the required units active. #133 alone owns
whether that boundary mutates systemd/D-Bus state and how it restores it; an M2
lifecycle fixture may supply a no-op/fake boundary and may not claim D-Bus
behaviour. After the boundary and helper verification, this state machine
records `active` and opens only ownership-compatible profile admission.

The no-systemd path uses the same claim and compositor boundary, selects
`helper-mode direct`, and invokes the externally owned environment-publication
boundary in no-manager mode. #133 alone decides whether that boundary performs
any D-Bus mutation and how it is restored; an M2 lifecycle fixture may use the
same no-op/fake boundary and may not claim D-Bus behaviour. Boundary absence
remains SPEC 0005's named degradation and does not prevent the direct lifecycle
from reaching `active`. It creates one bounded supervisor/process group for
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

There is at most one non-closed Helm session claim per UID while the externally
owned environment-publication boundary may have changed per-UID state. A second
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

This exclusion is a prerequisite for #133, not a definition of how global
environment values are restored. No M2 code may treat this spec as authority
for D-Bus activation or existing-owner Qt behaviour.

### 4. Launch preparation, adoption and lease transfer

#### Common pre-exec order

For either ownership mode, a launch performs these boundaries in order:

1. Create exactly one inert lifetime-owner process. It remains parent-reapable,
   has a parent-death fail-safe while ownership is local, holds no user profile
   code, and cannot pass its exec gate.
2. With `lifecycle.lock` held before SPEC 0011's shared `activation.lock`,
   resolve and fully validate `current`; create and fsync a process lease for
   the owner's exact PID/start-time/boot/UID identity. Capture generation id,
   manifest digest and the opaque lease reference from that selection.
3. Atomically persist and fsync sequence 1 `preparing` with `exec-open no` and
   `lease-kind process` plus exactly those identities. Only after this point may
   ownership adoption be requested.
4. Establish the selected ownership mode and verify the exact owner PID is in
   it. For systemd, capture the verified cgroup path/device/inode as well as the
   invocation. Under the shared generation lock, atomically replace the process
   lease with a transferred lifecycle lease bound to the launch id and verified
   ownership evidence, and fsync the leases directory. Successful transfer
   consumes all launcher-side release authority.
5. Durably record `adopted`, `lease-kind lifecycle`, the verified ownership
   incarnation and cgroup evidence while `exec-open no`. Next atomically advance
   that same state/sequence to `exec-open yes` and fsync it; only then may the
   launcher send the unique gate token. `yes` is durable authorization, not an
   inference that a volatile gate survived.
6. On receiving the token, the owner itself acquires `lifecycle.lock`, performs
   the expected-sequence transition to `running`, and fsyncs it. Only a
   successful `running` transition authorizes the owner to start the one fresh
   opaque profile prepared by #135. If EOF arrives instead, or the transition
   conflicts, it starts no application and exits. Thus a crash after gate send
   but before `running` cannot leave an executing application behind an
   `adopted/exec-open no` record, and reconciliation never needs to reopen a
   gate.

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

Failure before transfer keeps the gate closed, kills and reaps the inert owner,
durably records `terminal/failed` with `lease-kind process`, and then releases
only that process lease. If the launcher itself dies, the parent-death fail-safe
prevents pre-transfer exec and reconciliation performs the same cleanup.
Failure or launcher death after transfer never creates a replacement: gate EOF
leaves profile code unexecuted, a durable authorization without `running` is
aborted by reconciliation, and a `running` owner is reconciled in place. The
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
exited. Normal owner exit is expected only after that condition, but lease
release still waits for reconciliation's independent recursive-subtree proof;
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
reparented descendants, and declares normal completion only after every
attributed descendant other than itself is gone. It then writes/fsyncs the
terminal `direct-drained yes` witness and exits. A reconciler may release the
lifecycle lease only after validating that witness, the exact owner has exited,
and the recorded group is empty. Each wait, TERM and KILL phase is
deadline-bounded.

Changing session/process group, double-forking beyond the tracked subreaper, or
otherwise detaching is unsupported. Detection marks the record `lost`, retains
the lease, emits the named no-systemd degradation, and returns without waiting
forever. Absence of the owner alone is never sufficient release proof for a
direct transferred lease; an external reconciler cannot create a
`direct-drained` witness after the owner dies. The no-systemd path makes no claim that an
application survives logout or user-manager loss equivalently to a systemd
scope; logout attempts to terminate every direct launch group within the
session deadline.

### 5. Launch state machine and reconciliation

The launch state machine is:

```text
preparing -> adopted -> running -> terminal -> collectible
     \---------- failure from every nonterminal state ----------/
```

The one permitted same-state launch transition is
`adopted/exec-open no -> adopted/exec-open yes` with a sequence increment and
all other identity fields unchanged. It durably authorizes the unique token; it
is not a claim that the volatile gate or application already ran.

`collectible` is a predicate, not a persisted backward transition: owner
emptiness is proven, `terminal` is fsynced, the actual process or lifecycle
lease is unlinked and the leases directory fsynced, and only then may the
record be removed and the launches directory fsynced. A crash after terminal
but before lease removal retries removal; a crash after lease removal but
before record removal recognizes the terminal/missing-lease case below. If any
proof is uncertain or an operation fails, the remaining record/lease stays.
TERM refusal is bounded; it leaks safe evidence rather than authorizing
deletion.

Reconciliation runs at activation-root open, before a restarted `helm-wm`
accepts a launch, and after user-manager reconnect. It opens the referenced
lease without following links and dispatches on durable record state plus the
lease's actual parsed format; `lease-kind` is an expected value whose mismatch
selects a recovery row rather than being guessed away:

| Durable record | Actual lease | Recovery |
|---|---|---|
| `preparing`, `lease-kind process` | matching process lease | The gate cannot have opened. Revalidate and terminate/reap the owner and any exact deterministic scope/group already created, prove emptiness, write `terminal/failed`, unlink/fsync the process lease, then collect. Live/uncertain or same-name/different-incarnation evidence is retained. |
| `preparing`, `lease-kind process` | matching lifecycle lease | Transfer committed before the adopted record. For systemd, use the lease's complete invocation/cgroup evidence to stop only that gate-closed ownership, prove it empty, write `terminal/failed` with `lease-kind lifecycle`, release/fsync the lifecycle lease, then collect. For direct, the nonterminal record cannot contain the owner-written terminal `direct-drained yes` witness: retain the record and lifecycle lease as uncertain rather than infer release from owner/group observation. It is never completed or executed. |
| `preparing` | lease absent | No profile was authorized. Revalidate and terminate any exact gate-closed owner/ownership; after proven emptiness write terminal and collect the record. Uncertainty retains it. |
| `adopted`, either `exec-open` value | matching lifecycle lease | `exec-open no` was never authorized. `exec-open yes` may have sent the token, but the owner could not start the application without first durably reaching `running`. For systemd, under the lock abort the expected sequence, terminate the exact ownership, prove emptiness, write terminal, release the lease and collect. For direct, retain the nonterminal record and lifecycle lease as uncertain because no owner-written terminal `direct-drained yes` witness exists; reconciliation never reopens the gate. |
| `running` | matching lifecycle lease | Reconcile the same live owner/scope/group in place and never spawn. A systemd scope releases only after the section 4 complete-subtree proof. A direct group remains retained while `running`: only its still-live owner may write the terminal `direct-drained yes` witness after draining descendants; owner death or an external empty-group observation is uncertain and retains the lease. |
| `terminal`, `lease-kind process` | matching process lease | Re-prove the gate-closed owner/ownership empty, unlink/fsync the process lease, then collect. This is the crash after terminal and before untransferred release. |
| `terminal`, `lease-kind lifecycle` | matching lifecycle lease | For a systemd scope, re-prove complete ownership empty. For a direct group, validate `direct-drained yes`, prove the exact owner identity stale, then prove the recorded group empty. Only then unlink/fsync the lifecycle lease and collect; missing witness, live/reused owner, or nonempty/uncertain group retains it. |
| `terminal` | lease absent | For systemd, after re-proving recorded ownership empty, remove/fsync the terminal record. For direct, remove it only after validating `direct-drained yes`, exact owner staleness and recorded-group emptiness. This is the crash after lease release and before record collection; absence is not uncertainty only after the applicable proof. |
| `adopted` or `running` | process lease, absent lease, malformed lease, or identity mismatch | This inventory is not producer-reachable and might expose an unprotected execution. Report fatal/uncertain; do not signal, adopt, release or delete. |
| any state / expected `lease-kind` | every canonical or malformed actual lease format, absence, identity relation or cross-kind combination not matched by an earlier row | Exhaustive fatal/uncertain default. This includes `preparing` with expected lifecycle, terminal cross-kind leases, and malformed or identity-mismatched Preparing/Terminal evidence. It authorizes no signal, adoption, release, record deletion or generation deletion. |

For every row, a matching live systemd scope means unit name, invocation,
recorded cgroup path/device/inode, owner identity and current membership all
agree. An unloaded old scope is empty only under section 4's durable-cgroup
proof. A direct group is releasable only after its still-live owner wrote
`direct-drained yes` after its PID/start-time, UID, boot, group and
tracked-descendant checks showed no attributed descendant other than itself;
the reconciler then verifies that exact owner stale and the recorded group empty.
An external reconciler cannot establish or synthesize the owner witness.
Detachment, permission failure
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

The shipped WM/bar unit sources still name
only `PartOf=graphical-session.target` and are explicit non-conforming/red
evidence for A12. Functional unit changes require the executable unit-graph
test to be observed failing first. `INSTALL.md` must describe shipped behavior
until those changes land.

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
4. Record `cleanup-delegated`, then perform SPEC 0005's environment,
   compositor and runtime-tree cleanup subject to #133's eventual environment
   ownership/restoration contract. Removing `$XDG_RUNTIME_DIR/helm` cannot
   remove any asset promised to a surviving profile launch.
5. Record `closed`; remove the session record only after its close is durable
   and the caller still owns the same session id and sequence.

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

### 8. Lifecycle status snapshots

Any current or future lifecycle-status subscription follows ADR 0004's
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
classification. A surviving launch record is snapshot-valid only with a
present canonical lease of its recorded expected kind, matching identity, and
consistent readable ownership evidence. An absent or malformed actual lease,
a cross-kind or identity-mismatched lease, or missing, inconsistent, unreadable
or otherwise uncertain ownership evidence fails the complete snapshot; the
record is never emitted as a healthy current launch.

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
current truth only under this spec. Exact DTO names, request idempotency,
cursors, frame limits and any history/replay promise are deliberately delegated
to SPEC 0006/#117.

## Acceptance criteria

Each row is a Given/When/Then contract. Tests are added red-first before the
corresponding implementation.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a launch selected on generation N, when N+1 becomes current and generation GC races launch or teardown, then the process reads only N and N is retained until the exact transferred lifetime ownership is proven empty. In particular, if the supervisor PID is stale while an attributed scoped descendant survives, GC retains both lifecycle lease and generation. | |
| A2 | Given a fault at every owner-create, process-lease, preparing-record, scope/group adoption, lease-transfer-before-adopted-record, adopted-record, durable-exec-authorization, gate-send-before-running-record, running-record, terminal-record-before-lease-release, lease-release and record-collection boundary, when recovery runs, then the record-state/actual-lease table yields either no executed profile and eventual collection, exactly one reconciled owned launch, or retained direct lifecycle evidence where a nonterminal transfer crash lacks the owner-written `direct-drained` witness; there is never an unleased live profile, duplicate exec, or repeated-fault quota leak. | |
| A3 | Given a child or scope that exits, ignores TERM, leaves uncertain membership, or outlives a dead direct supervisor, when teardown reconciles it, then `terminal` is fsynced before cleanup, lease release follows proven emptiness, waits are bounded, and uncertainty preserves the record and lease. A direct lifecycle lease is releasable only after its still-live owner recorded `direct-drained yes`; later reconciliation never invents that witness. | |
| A4 | Given a live profile launch when `helm-wm` crashes and restarts, when activation reconciliation and a client reconnect complete, then the same PID/scope and generation are reported, no application is restarted, and the first status frame is a fresh current snapshot with no old-incarnation delta. | |
| A5 | Given user-manager reexec/restart with a live scope, an unloaded old scope whose recorded cgroup path is absent, the same recorded cgroup inode reporting recursive `populated 0` or `1`, an empty root `cgroup.procs` with a populated nested child cgroup, a reused cgroup path/inode, or a same-name/different-invocation scope, when reconciliation runs, then only matching invocation plus durable cgroup identity is adopted once, only stale-owner plus absent old cgroup or verified `cgroup.events` `populated 0` permits collection, nested/live/inconsistent/reused evidence causes no duplicate launch or lease release, and only target-owned helpers may restart while the same session claim remains active. | |
| A6 | Given a running target and independent profile scope, when logout stops `helm-session.target`, then an executable unit-graph fixture proves the bar stops before the WM and every helper stops before environment cleanup while the profile scope remains untouched. | |
| A7 | Given equivalent live launches with lingering off and on, when logout and later reconciliation run, then off permits logind cleanup followed only by proven-stale record/lease collection, while on preserves the live scope, record and lease until real exit. | |
| A8 | Given faults after temporary creation, during/after temporary write/fsync, before/after no-replace rename and parent fsync for session claim, launch create and record update, plus an early same-boot entry crash and concurrent same-UID claimants, when recovery holds the lifecycle lock, then discardability is decided only from exact basename grammar, current-UID no-follow exact-0600 regular-file metadata, the 4096-byte size bound and the applicable inventory's final-record condition; every artifact satisfying those conditions is discarded and fsynced without payload decoding or replay even when empty, partial, non-UTF-8 or noncanonical, every malformed final or malformed-reserved name/metadata fails closed, no partial final appears, natural crash temporaries cannot permanently block claim/GC or exhaust quota, exactly one healthy session claim wins before compositor/global-environment/helper/profile mutation, and a stale id/sequence cannot clear it. | |
| A9 | Given no usable systemd user manager, when the entry claims, records its compositor, invokes the externally owned environment-publication boundary in no-manager mode, starts/records/verifies bounded direct WM and bar supervisors, admits a profile, and then the profile exits, detaches, or receives logout, then the direct session reaches `active` without a target, an M2 fake boundary makes no D-Bus-behaviour claim, helper/profile teardown uses the recorded groups in bar-before-WM bounded order, the launch honors lease-before-exec and collects only after proven emptiness, detachment is degraded/uncertain, and no systemd-equivalent survival is claimed. | |
| A10 | Given wrong owner/mode/type, a symlink, malformed/reserved entry, stale boot/PID/start-time, reused PID, stale unit name with a new invocation, or a conflicting sequence, when reconciliation or GC runs, then it refuses signal/adoption/deletion for that evidence and leaves valid unrelated state intact. | |
| A11 | Given teardown work still live or uncertain at 15 seconds, when the entry deadline expires, then the entry may return but the durable state does not advance falsely, every affected record and generation lease remains, and runtime removal has deleted no promised application asset. | |
| A12 | Given the shipped source units and a live user-manager fixture, when restart, target stop and abort paths are exercised, then both views agree on `Restart=always`, `PartOf=helm-session.target`, inverse bar-before-WM stop order, profile-scope independence and exactly one abort execution after start-limit exhaustion. | |
| A13 | Given a profile resolves to `DBusActivatable` or an already-running owner, when M2 admission evaluates either mode, then it refuses before creating a lifetime owner, selecting a generation, creating a process/lifecycle lease or record, creating a systemd scope/group, or making any D-Bus activation call or other activation side effect. | |
| A14 | Given each safe crash point before/after owned-directory creation, child fsync, parent fsync, first `lifecycle.lock` exclusive creation, lock fsync, activation-root fsync and `launches/` creation, plus concurrent same-UID initializers and separate unsafe-existing-object fixtures, when initialization retries, then every safe inventory converges from absence to verified current-UID exact-0700 directories and one persistent current-UID zero-length exact-0600 lock inode, all contenders revalidate and acquire that same inode before lifecycle mutation, each created component's child and parent durability precedes dependent mutation, and every unsafe collision fails closed without unlinking, replacing or repairing it. | |
| A15 | Given valid independent records together with a malformed final record, malformed reserved entry, unsafe record metadata or an exceeded inventory bound, and separate byte-canonical launch-record fixtures whose actual referenced lease is absent, malformed, cross-kind or identity-mismatched or whose actual ownership evidence is inconsistent/unreadable, when a lifecycle client handshakes and subscribes, then under `lifecycle.lock` followed by SPEC 0011's `activation.lock` the server cross-validates every launch through section 5, emits neither an affected record as healthy nor any partial full snapshot or delta for that subscription, reports fatal/uncertain snapshot failure without silently omitting the evidence, leaves permitted internal read-only reconciliation distinct from public full/current truth, and emits a full snapshot only for a fresh subscription after a complete valid bounded record/lease/ownership scan. | |
| A16 | Given no usable systemd user manager and separate direct-helper fixtures for an unrequested clean WM exit, failed WM or bar exit, clean bar exit, WM status 69 or 78, refusal of the next WM or bar start by the five-in-thirty limit, WM readiness failure, and entry death during `preparing` or `active`, when the direct supervisors or reconciler handle them, then WM clean/failure and bar failure use the one-second bounded restart policy, clean bar exit is not restarted, WM restart-prevent/limit/readiness failure enters idempotent admission-freeze and teardown exactly once, opens no new profile admission and never advances `preparing` to `active`, bar limit exhaustion remains degraded without tearing the session down, and entry death in either state freezes admission, recreates no helper, never resumes or advances `active`, and tears down only revalidated recorded groups in bar-before-WM order. | |

## Budgets and limits

| Item | Limit |
|---|---|
| Entry teardown | 15 seconds total; expiry preserves uncertain state rather than extending the deadline |
| Launch namespace entries | 4096 total final records plus temporary artifacts per UID activation root |
| Launch namespace bytes | 4096 bytes per entry and 16 MiB total |
| Terminal lifecycle retention | 0 seconds after durable terminal state and durable lease release |

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
| Logout kills a profile or drops its generation | Scope independence, freeze-before-stop, proof-before-release and A3/A6/A7/A11 |
| PID or unit name reuse targets an unrelated process | Boot/UID/start-time/invocation/cgroup revalidation and A10 |
| Runtime cleanup removes surviving assets | Persistent records plus sealed-generation-only dependencies and A7/A11 |
| A dead socket subscription is treated as durable | Fresh incarnation/full snapshot rule and A4 |
| No-systemd detachment becomes an unowned success | Explicit degraded failure, retained evidence and A9 |

## Open questions

None inside this M2 contract. The intentionally unresolved choices remain with
#133, #135, SPEC 0005 OQ-1, SPEC 0003's river replay experiment, and SPEC
0006/#117's public wire and history semantics. Their resolution must not alter
lease-before-exec, proof-before-release, monotonic reconciliation, or the
single-active-session prerequisite without first amending this specification.
