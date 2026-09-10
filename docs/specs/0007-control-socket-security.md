# SPEC 0007 — Control-socket transport and security

- **Status:** Accepted (2026-08-28; endpoint feasibility correction 2026-09-10)
- **Milestone:** M2
- **Decisions:** [ADR 0004](../adr/0004-ndjson-control-socket.md)
- **Amends:** [SPEC 0001](0001-realm-core-contracts.md), [SPEC 0003](0003-realm-session.md), [SPEC 0006](0006-realm-ctl.md)

## Purpose and authority

The control socket can spawn processes and change the live desktop. It must be scriptable without making an absent runtime directory, a caller-selected path, or an unbounded local peer a control or liveness boundary.

This is the complete **Accepted** implementation specification for the
control-socket transport. Accepted SPEC 0003 owns the session integration and
readiness order; this specification owns the endpoint and transport mechanics.
Where historical socket prose differs, this specification wins.

## Scope

**In:** runtime-path resolution, endpoint creation and reclaim, Linux peer admission, the connection state machine, framed I/O limits, client startup retry, and the test seams needed to verify those rules.

**Out:** a remote-control protocol, authorization between different local
users, and changes to the JSON request/response/event vocabulary. An admitted
same-euid process is trusted for every existing command; directory mode is
defence in depth, not per-command authorization. The endpoint capability slice
tracked by #218 is narrower still: it does not implement or redesign peer
credentials, Hello/framing, request dispatch, subscriptions, the client retry
schedule, `ClientEndpoint`, client realm reopens or attempt resolution, the
persistence worker, the outer event loop, or daemon assembly.

Same-euid processes are already trusted control peers and are also trusted not
to mutate Realm's runtime namespace concurrently with endpoint construction or
cleanup. Linux provides no atomic `statat`-and-`unlinkat`, and a pathname Unix
socket's VFS inode cannot be proved by comparing it with `fstat(socket_fd)`
because those descriptors name different kernel objects. Identity rechecks
reject replacements detectable before the final pathname operation; preserving
a replacement made by a trusted same-euid process in the final stat-to-unlink
or bind-to-stat gap is outside this threat model, not an implementation promise.

## Ownership, target, and API boundary

`realm-core::ipc` owns only portable wire values, `encode`, `decode`, and
`PROTOCOL_VERSION`. It performs no environment lookup, filesystem operation,
credential query, or socket operation. The legacy M0
`realm_core::ipc::socket_path()` helper is not part of the accepted M2 API and
is removed when the transport is introduced.

A new shared workspace library crate, `realm-control`, owns Linux endpoint
capabilities and, in the later #41 slice, the client/server transport. Both
`realm-session` and `realm-ctl` depend on it; `realm-ctl` must not depend on
`realm-session`, and portable `realm-core` must not acquire filesystem or socket
work. `realm-control` is Linux-only and must reject every non-Linux compilation
explicitly (for example with a target-gated `compile_error!`), rather than
silently weakening the contract or omitting functionality.

The complete `realm-control` boundary across #218 and #41 includes at least:

```rust
pub struct RuntimeDir(/* absolute display path, retained fd, daemon euid */);
pub struct RealmDir(/* retained validated realm fd */);
pub struct SocketEndpoint(/* canonical display path + retained capability */);
pub struct BoundControlEndpoint(/* private non-listening fd + ownership */);
pub struct ActiveControlListener(/* private listening fd + ownership */);
pub struct ClientEndpoint(/* retained runtime capability for retries */);
pub struct Client(/* connected #41 transport wrapper */);

pub enum IpcPathError {
    MissingRuntimeDir,
    UnsafeRuntimeDir,
    UnsafeRealmDirectory,
    UnsafeSocketEntry,
    EndpointInUse,
    Io(std::io::Error),
}

pub trait RuntimeDirResolver {
    fn resolve(&self) -> Result<RuntimeDir, IpcPathError>;
}

pub fn production_runtime_dir() -> Result<RuntimeDir, IpcPathError>;
pub fn test_runtime_dir(path: &Path) -> Result<RuntimeDir, IpcPathError>;

impl RuntimeDir {
    pub fn path(&self) -> &Path;
    pub fn prepare_server_endpoint(self) -> Result<SocketEndpoint, IpcPathError>;
    // #41, not #218.
    pub fn client_endpoint(self) -> ClientEndpoint;
}

impl RealmDir {
    pub fn as_fd(&self) -> BorrowedFd<'_>;
}

impl SocketEndpoint {
    pub fn path(&self) -> &Path;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn bind(self) -> Result<BoundControlEndpoint, IpcPathError>;
}

impl BoundControlEndpoint {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn activate(self) -> Result<ActiveControlListener, IpcPathError>;
}

impl ActiveControlListener {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
}

impl AsFd for ActiveControlListener { /* poll/accept only */ }

impl ClientEndpoint {
    // #41, not #218.
    pub fn connect(&self) -> Result<Client>;
}
```

Issue #218 implements only `RuntimeDir`, `RealmDir`, `SocketEndpoint`,
`BoundControlEndpoint`, and `ActiveControlListener`. `ClientEndpoint`,
`Client`, `RuntimeDir::client_endpoint`, client realm reopen/attempt resolution,
and retry behavior are delivered by #41.

`RuntimeDir` retains the absolute public display path, the securely resolved
runtime-directory fd, and the daemon effective uid captured for validation.
`RealmDir` retains its separately validated directory fd and exposes a borrowed
fd for later descriptor-relative `ledger.json` work. `SocketEndpoint` denotes
only the exact fixed `realm/ctl.sock` descendant: it retains the realm
capability and a canonical public display path, but it exposes no way to replace
that descendant. Bound and active wrappers expose borrows of their endpoint and
`RealmDir`; neither wrapper is `Clone`. `BoundControlEndpoint` deliberately
does not implement `AsFd`, so code outside the one-shot transition cannot call
`listen`. `ActiveControlListener` implements `AsFd` for `poll`/`accept` while
keeping its fd private. Its `AsFd` borrow is the listener only: the singleton
lock descriptor is private to the ownership wrappers and no accessor or trait
implementation exposes it. `Client` is the later #41 connected transport
wrapper; naming it here is not a claim that the endpoint-only slice implements
that transport.

`production_runtime_dir` alone reads `XDG_RUNTIME_DIR`. It rejects an absent,
relative, or non-directory value as `MissingRuntimeDir`; a caller-provided
runtime-path symlink, foreign owner, or any mode other than exactly `0700` is
`UnsafeRuntimeDir`. Missing owner bits are unsafe just like extra group/world
bits. `test_runtime_dir` accepts an explicit absolute temporary runtime
**directory**, applies exactly the same descriptor/type/owner/mode checks, and
never accepts a socket path. `/tmp`, `REALM_SOCKET`, and a CLI socket-path
override do not exist in production or the fixture API.

Caller-provided path resolution is descriptor-relative after the first
capability is obtained. Linux resolution uses `openat2` with
`RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, `O_DIRECTORY`, and `O_CLOEXEC`;
`ENOSYS`, an unavailable security flag, or any failure of that secure operation
fails closed. No fallback reopens the absolute display path. The server uses a
scoped process umask of `0077` around `mkdirat(..., 0700)` for `realm` and
restores the previous umask on success, every error return, and unwind. It then
opens `realm` with the same secure descriptor-relative resolution and requires
directory type, daemon-euid ownership, and mode exactly `0700`; an existing
object with any other type, owner, or mode is rejected unchanged and is never
chmodded. Because umask is process-global, all server endpoint preparation,
through successful bind and verification, must finish while the process is
single-threaded. Clients never create `realm`.

Linux provides neither `bindat` nor `connectat`. Every bind and connect,
including the stale probe and every shared-crate client attempt, therefore uses
a private address generated internally as
`/proc/self/fd/<realm-dir-fd>/ctl.sock`. Following this procfs fd bridge is the
one intentional magic-link traversal after the descriptor capability has been
validated; no caller can supply or alter it. Before server mutation, opening
`/proc/self/fd` alone is insufficient: the server must traverse the actual
`/proc/self/fd/<held-runtime-fd>` bridge and verify that the resulting directory
has the same device/inode, type, euid owner, and exact `0700` mode as the
retained `RuntimeDir`. A missing, inaccessible, mismatched, or otherwise
unusable actual bridge, or an internal socket address that does not fit
`sockaddr_un`, fails closed with no absolute-path fallback. The canonical public
`$XDG_RUNTIME_DIR/realm/ctl.sock` is retained only for display and external
clients such as `socat`; it must also fit Linux `sockaddr_un`, including its
terminating NUL, and reaches the same directory entry. Shared-crate clients do
not connect through that display path: they use their validated `RealmDir` and
the generated procfd bridge.

Server construction order is exact:

1. resolve and retain the runtime capability and effective uid;
2. before any filesystem mutation, traverse the actual
   `/proc/self/fd/<held-runtime-fd>` bridge, verify its directory identity and
   properties against the retained `RuntimeDir`, length-check the fixed
   canonical address, and length-check a procfd socket address using the widest
   possible Linux fd decimal representation; each address check includes the
   terminating NUL in `sockaddr_un`;
3. under scoped umask `0077`, attempt `mkdirat(..., "realm", 0700)`, accepting
   only absence/create or `EEXIST`, then restore the old umask;
4. securely open and validate `realm`, retaining `RealmDir`, then form the
   actual procfd address whose fit was guaranteed by step 2;
5. open the validated realm directory again as a private independent open-file
   description with `O_RDONLY | O_DIRECTORY | O_CLOEXEC` (or an exactly
   equivalent atomic close-on-exec directory open), verify `FD_CLOEXEC` with
   `fcntl(F_GETFD)`, acquire the singleton lock, and retain that fd privately;
6. only then inspect, probe, identity-recheck, and if authorized unlink an
   existing `ctl.sock`;
7. create the nonblocking close-on-exec socket fd;
8. under scoped umask `0177`, bind the procfd address, then restore the old
   umask; and
9. validate pathname properties, retain pathname identity, verify
   `getsockname` and pre-activation `SO_ACCEPTCONN`, then return the bound
   capability.

Steps 1–9 complete before the process starts any other thread. Step 2 failure
does not create `realm`, inspect or alter `ctl.sock`, or try another pathname.
Every failure closes capabilities already acquired, preserves any entry whose
identity mismatch was detected before the final operation, and returns without
falling back. The trusted same-euid final-gap limitation above still applies.

## Listener ownership and stale reclaim

Before examining `ctl.sock`, the server opens the already validated realm
directory again with `O_RDONLY | O_DIRECTORY | O_CLOEXEC` (or an exact
equivalent) to obtain a separate open-file description. It verifies
`FD_CLOEXEC` through `fcntl(F_GETFD)`, acquires `flock(LOCK_EX | LOCK_NB)`, and
retains that private descriptor through bound, active, and cleanup states. No
public borrow exposes the lock fd. Lock contention returns
`IpcPathError::EndpointInUse` without a socket probe, stat, unlink, bind, or
other mutation. A duplicate fd would share the same open-file description and
therefore the same `flock`; a separate open is required so singleton ownership
has its own lifetime, independent of temporary or borrowed copies of the realm
resolution fd. Any open, flag-verification, or non-contention lock failure
fails closed. No launch path may clear `FD_CLOEXEC`: after a successful `exec`,
a launched client must hold no copy of the singleton lock and therefore cannot
extend server ownership beyond the daemon's lifetime.

Only after holding that singleton lock may an existing `ctl.sock` be examined.
It must no-follow-stat as a socket owned by the daemon euid with mode exactly
`0600`; every other entry, including a symlink, is rejected unchanged. The
server takes one stale probe with a fresh
`AF_UNIX/SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC` fd and closes it after that
attempt. Linux results are total:

| Probe result | Decision |
|---|---|
| immediate `ECONNREFUSED` | stale candidate |
| immediate success | preserve entry and fail (`EndpointInUse`) |
| `EAGAIN` or `EINPROGRESS` | poll `POLLOUT` for at most 100 ms, then inspect `SO_ERROR` |
| polled `SO_ERROR == ECONNREFUSED` | stale candidate |
| polled success, timeout, any other `SO_ERROR`, or poll failure | preserve entry and fail |
| unexpected `EALREADY`, or any other immediate error | preserve entry and fail |

A stale candidate is not yet authority to unlink. Immediately before
`unlinkat`, the server repeats the no-follow stat and requires the same
filesystem device/inode, socket type, euid owner, and exact `0600` mode observed
before probing. A mismatch detected by the second stat preserves the
replacement and fails. Only a candidate unchanged at that recheck is authorized
for reclaim; the subsequent stat-to-unlink gap is not atomic and relies on the
stated same-euid trust boundary.

With no entry remaining, `SocketEndpoint::bind(self)` creates one
`AF_UNIX/SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC` fd. Under a scoped `0177`
umask, restored on success, every error return, and unwind, it binds exactly the
internally generated procfd address. After bind it no-follow-stats the pathname
and requires socket type, daemon-euid ownership, and exact mode `0600`; that
pathname `st_dev/st_ino` becomes the retained cleanup identity. It must **not**
compare that identity with `fstat(socket_fd)`: a Unix socket fd refers to a
sockfs inode distinct from the pathname's VFS inode. Instead, `getsockname`
must equal the exact internally generated bind address and
`getsockopt(SO_ACCEPTCONN)` must be false. If the post-bind pathname stat syscall
fails, the server closes the fd, preserves the observed pathname, and returns
`IpcPathError::Io` with the original errno. Only a successfully read stat with
bad type, owner, or mode maps to `UnsafeSocketEntry`; the server closes the fd,
preserves the detected entry, and fails. A later `getsockname` or
`SO_ACCEPTCONN` failure closes the fd and removes only an identity that still
matches at cleanup recheck under the singleton lock. A trusted same-euid
mutation in the unavoidable bind-to-stat or final stat-to-unlink gap remains
outside the threat model.

The returned `BoundControlEndpoint` owns the private non-listening fd, the
singleton lock, and the retained pathname identity. Its consuming
`activate(self)` calls `listen(fd, 64)` exactly once, then requires
`SO_ACCEPTCONN` true before returning `ActiveControlListener`. Activation
failure is fatal, performs the same ownership-safe cleanup, and can never send
readiness. The type-consuming transition, private fd, and absence of `AsFd` on
the bound wrapper make a second activation unavailable through the public API.

Drop for either bound or active ownership closes the socket fd first, then,
while still holding the singleton lock, no-follow-stats and unlinks only a path
whose device/inode, socket type, daemon-euid owner, and `0600` mode all match the
retained identity. A replacement visible at that stat is preserved. The final
stat-to-unlink step is not atomic and relies on the stated same-euid trust
boundary. Drop cleanup is best effort; abrupt process death is recovered by the
locked stale-reclaim procedure, not by weakening identity checks.

## Linux admission and connection state machine

The M2 session transport is Linux-only. `realm-core` remains portable;
Linux-specific endpoint and transport code lives in `realm-control`.
Unsupported targets fail that crate's build explicitly rather than omitting
peer admission or substituting pathname permissions.

The listener and every accepted stream are nonblocking. Before reading any byte, the listener calls `getsockopt(SOL_SOCKET, SO_PEERCRED)`. Linux `SO_PEERCRED` identifies credentials fixed at connection time. A lookup error, absent credential, or uid different from the daemon effective uid closes the stream without a protocol reply. Production may not replace this check with a test fake.

An accepted connection is exactly one of these states:

| State | Entry | Permitted input | Output / exit |
|---|---|---|---|
| `Admitting` | `accept4` succeeded | none | obtain credentials; close on failure, otherwise enter `AwaitHello` |
| `AwaitHello` | admitted | one complete Hello before its 1 s monotonic deadline | queue Hello reply; matching version enters `Ready` after the reply is enabled for write, mismatch enters `CloseAfterReply` |
| `Ready` | matching Hello | one decoded ordinary request at a time | one ordinary response at a time; `Subscribe` enters `Subscriber` after its snapshot is queued |
| `Subscriber` | successful Subscribe | none | one partial state frame plus one replaceable latest-state frame; any subsequent input closes it |
| `CloseAfterReply` | terminal decodable protocol error or version mismatch | none | drain the one terminal reply for at most 100 ms, then close |
| `Closing` | EOF, a bound/deadline violation, or a close decision | none | close fd and release slot |

EOF closes the peer in every state. Deadline accounting uses `CLOCK_MONOTONIC`: the Hello deadline starts at successful `accept4`; a write-stall interval starts when a queued frame first gets `EAGAIN` or a short write and resets only after positive byte progress.

## Frames, errors, capacity, and liveness

A frame is its UTF-8 JSON payload plus exactly one terminating LF. Its total size is at most **65,536 bytes**, including that LF; 65,535 payload bytes plus LF is valid. A buffer that exceeds the limit before LF, a frame without valid UTF-8, or syntactically invalid JSON closes the peer without a reply. Valid JSON that cannot decode as `Request` (including an unknown request variant or invalid request fields) gets one `Response::Error` and `CloseAfterReply`.

| State | Frame class | Result |
|---|---|---|
| `AwaitHello` | matching `Hello` | queue `Response::Hello`; enter `Ready` after its write is enabled |
| `AwaitHello` | mismatched `Hello` | queue server `Response::Hello`; `CloseAfterReply` |
| `AwaitHello` | valid decodable non-Hello request | queue `Response::Error`; `CloseAfterReply` |
| `Ready` | ordinary request | process/queue its ordinary response in request order |
| `Ready` | duplicate `Hello` | queue `Response::Error`; `CloseAfterReply` |
| `Ready` | `Subscribe` | queue immediate `Event::State`; enter `Subscriber` |
| `Subscriber` | any complete frame | close without reply |
| any input state | invalid UTF-8, invalid JSON, oversized/unterminated frame | close without reply |

Each admitted connection consumes one of **64** slots from `accept4` until its fd is closed, including `Admitting`, `AwaitHello`, `CloseAfterReply`, and draining subscribers. The listener drains `accept4` until `EAGAIN` on every readability notification. It accepts a 65th fd and immediately closes it nonblocking without allocating a connection record, then continues draining; this prevents a full user-space limit from filling the kernel backlog.

For `AwaitHello` and `Ready`, input consists of at most one partial frame and one decoded-but-not-completed request. Output consists of at most one complete ordinary protocol frame, whether Hello, `Response`, or `Error`, plus its partial-write cursor. A pipelined frame that would create a second pending decoded request or response closes that peer. Responses remain in request order. `Subscribe`'s initial snapshot is subscriber output, not ordinary output; no state snapshot, error, or Hello frame is exempt from its relevant byte/message accounting.

Every write is nonblocking. Ordinary responses and subscriber state use a two-second no-progress stall limit. Terminal mismatch/ordering/invalid-request replies use the shorter 100 ms `CloseAfterReply` drain limit; delivery is best-effort, then EOF is mandatory. On shutdown, each subscriber may receive one `Event::Shutdown` only if it can be queued without displacing a partial frame; the process drains all such writes for at most 100 ms total and exits regardless. No reply or shutdown guarantee permits blocking the event loop.

## Client startup race (#41, not #218)

The reusable `ClientEndpoint` retains its validated `RuntimeDir` capability.
For every connection attempt it reopens and validates `realm`
descriptor-relatively from that retained fd, creates a fresh procfd bridge from
the resulting `RealmDir`, and connects through that bridge. A retry never
rereads `XDG_RUNTIME_DIR`, creates `realm`, or resolves/connects through the
canonical display path.

For a normal command, `realm-ctl` retries only `ENOENT` and `ECONNREFUSED`, with
five nonblocking attempts separated by 10, 20, 40, 80, and 160 ms (310 ms
maximum waiting). It performs the mandatory Hello on the successful connection.
Any other connect/path error fails immediately; exhausting the retry schedule
is exit code 3. `doctor` retains its existing no-session reporting behavior.

## Test seams and acceptance criteria

Transport construction receives three explicit dependencies:

```rust
pub trait Clock { fn now(&self) -> MonotonicInstant; }
pub trait PeerCredentialProvider {
    fn peer_uid(&self, stream: &UnixStream) -> std::io::Result<u32>;
}
pub trait ControlLoopHarness {
    fn record_manage_finish(&mut self);
    fn record_socket_write_attempt(&mut self, connection: ConnectionId);
}
```

Production uses `CLOCK_MONOTONIC` and a Linux `SO_PEERCRED` provider; test-only fakes may supply deterministic time and credential outcomes. The harness records the completion boundary from key event dispatch through `manage_finish`, before any socket write attempt. A stalled-peer test advances the fake clock without sleeping, proves eviction at the exact deadline, and proves `manage_finish` is recorded first. A separate Linux integration budget test measures the real monotonic interval with a full send buffer and fails if the observed key-to-`manage_finish` interval is 4 ms or more; it may not use a fake clock for that performance assertion.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given an absent, relative, non-directory, symlinked, foreign-owned, or not-exactly-`0700` runtime path (including modes missing owner bits), or any secure `openat2` failure, when resolved, then the specified path error is returned and no fallback or override is read | `realm_control::tests::runtime_capability_rejects_every_unsafe_input_and_openat2_failure` |
| A2 | Given a hostile ambient umask, when the server prepares a missing fixed descendant, then `realm` is exactly `0700` and the old umask is restored; given the same absence on a client, it does not create `realm` | `realm_control::tests::server_creates_realm_exactly_once_under_scoped_umask`; client half in #41: `realm_control::tests::client_never_creates_realm` |
| A2a | Given an actual retained runtime-fd bridge that is missing, inaccessible, mismatched, or otherwise unusable, or a canonical or worst-case procfd address that overflows Linux `sockaddr_un` including its terminating NUL, when server endpoint preparation begins, then it fails before creating `realm` or inspecting/mutating `ctl.sock` and never falls back to the canonical or another path | `realm_control::tests::actual_runtime_procfd_bridge_is_verified_before_mutation`, `realm_control::tests::missing_inaccessible_or_mismatched_procfd_fails_before_mutation`, `realm_control::tests::sockaddr_un_overflow_fails_before_mutation_without_fallback` |
| A3 | Given an unsafe realm object or unsafe existing `ctl.sock`, when an endpoint is prepared, then the object is rejected and remains byte/identity unchanged | `realm_control::tests::unsafe_realm_and_socket_entries_are_preserved` |
| A4 | Given a reachable listener or an unchanged refused stale socket, when another server prepares the endpoint, then the live entry is preserved and only the unchanged refused identity is reclaimed | `realm_control::tests::live_listener_is_preserved_and_verified_refusal_is_reclaimed` |
| A5 | Given immediate refusal, `EAGAIN`, `EINPROGRESS`, completion success/refusal/error, timeout, poll failure, or unexpected `EALREADY`, when the one-shot nonblocking stale probe runs, then only immediate or completed `ECONNREFUSED` is a stale candidate and every other result preserves the entry | `realm_control::tests::linux_stale_probe_completion_table_is_total` |
| A6 | Given a first server that holds a bound but non-listening endpoint, when a second binder starts, then it returns `EndpointInUse` without probing or unlinking `ctl.sock` | `realm_control::tests::singleton_lock_protects_the_prelisten_state` |
| A6a | Given a bound or active owner, then its singleton lock uses a private independent directory fd with `FD_CLOEXEC`; when a long-lived exec-launched client remains after that owner drops, the client retains no lock and a second binder can acquire ownership | `realm_control::tests::singleton_lock_fd_is_private_directory_cloexec_and_not_inherited_across_exec` |
| A7 | Given a successful bind, then the pathname is an euid-owned socket of exact mode `0600`, the fd has `NONBLOCK` and `CLOEXEC`, the canonical external path denotes the same entry, `getsockname` equals the generated procfd address, and `SO_ACCEPTCONN` is false; a post-bind stat syscall error preserves its errno and observed pathname | `realm_control::tests::bound_capability_has_exact_path_fd_and_address_properties`, `realm_control::tests::post_bind_stat_error_preserves_errno_and_pathname` |
| A8 | Given one bound capability, when it is consumed by activation, then `listen(..., 64)` occurs exactly once, `SO_ACCEPTCONN` is true, and a connection succeeds | `realm_control::tests::activation_is_consuming_one_shot_and_listens_with_backlog_64` |
| A9 | Given a bound or active owner and either its unchanged entry or a replacement visible at the cleanup identity stat, when it drops, then its socket fd closes first, only the matching identity is removed under the retained lock, and the detected replacement remains; the stated trusted same-euid final gap applies | `realm_control::tests::bound_drop_closes_and_removes_only_matching_identity`, `realm_control::tests::active_drop_preserves_detected_replacement` |
| A10 (#41) | Given a reusable client endpoint and an environment change between retries, when connection is retried, then the retained runtime capability is reused, `realm` is reopened and validated relative to it, and neither the environment nor canonical display path is reread | `realm_control::tests::client_retry_reuses_the_retained_capability` |
| A11 | Given a program that calls `activate` twice on one bound value, when compiled, then the second call fails because the first consumed the capability | `realm_control::compile_fail::bound_endpoint_cannot_activate_twice` |
| A12 | Given recovery is incomplete, when the endpoint is bound, then it is non-listening and readiness is absent; only after transition to `Live` may activation verify `SO_ACCEPTCONN` and precede `READY=1` | `realm_session::tests::readiness_follows_live_listener_activation` |
| A13 | Given foreign or missing credentials, when accepted, then no byte is read; a same-euid peer remains live | `control_socket::tests::rejects_foreign_uid_before_read`, `control_socket::tests::rejects_missing_peer_credentials_before_read` |
| A14 | Given every state/error-table case, when a frame is read, then its reply (if any), deadline, and close/continue result match the table | `control_socket::tests::protocol_state_machine_is_total` |
| A15 | Given 64 occupied slots or an excess frame/queue, when the limit is reached, then only that peer is closed and listener draining continues | `control_socket::tests::connection_and_queue_limits_preserve_admitted_peers` |
| A16 | Given a non-reading ordinary client, mismatch client, subscriber, or shutdown, when its applicable drain deadline expires, then it is evicted/exited without blocking | `control_socket::tests::all_write_classes_have_bounded_nonblocking_drain` |
| A17 | Given a stalled peer and a key event, when the loop runs, then `manage_finish` precedes every write and real Linux measurement remains below 4 ms | `control_socket::tests::stalled_peer_preserves_key_path`, `control_socket::tests::linux_key_path_budget_with_full_socket_buffer` |

## Failure modes

This prevents the runtime-fallback, pathname-race, unauthenticated-local-peer, protocol-version-skew, unbounded-pipeline, and wedged-subscriber failures recorded in ADR 0004. No failure permits a fallback endpoint, a best-effort path-security check, an unbounded queue, or a blocking write.

## Open questions

None. Remote or cross-user control requires a new ADR and specification.
