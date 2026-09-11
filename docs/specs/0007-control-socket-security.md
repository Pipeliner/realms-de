# SPEC 0007 — Control-socket transport and security

- **Status:** Accepted (2026-08-28; endpoint feasibility correction 2026-09-10;
  transport-liveness correction 2026-09-11)
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

`realm-core::ipc` owns only portable wire values, bounded `encode`, `decode`,
and `PROTOCOL_VERSION`. `encode` must enforce the complete-frame bound while
serializing rather than allocate an oversized value first. It performs no
environment lookup, filesystem operation, credential query, or socket
operation. The legacy M0
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
pub struct ControlServer(/* listener + admitted transports + stable identities */);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64); // private opaque value, never a raw fd
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlToken(u64); // private opaque poll-registration identity
pub struct PollInterest<'a> {
    pub token: ControlToken,
    pub fd: BorrowedFd<'a>,
    pub readable: bool,
    pub writable: bool,
}
pub struct ReadyEvent {
    pub token: ControlToken,
    pub readable: bool,
    pub writable: bool,
}
pub struct ClientEndpoint(/* retained runtime capability for retries */);
pub struct Client(/* connected #41 transport wrapper */);
pub struct Subscription(/* connected #41 event iterator */);

pub enum ControlAction {
    Request { connection: ConnectionId, request: Request },
}

pub enum ControlError {
    StaleConnection { connection: ConnectionId },
    ShuttingDown,
    OutboundFrameTooLarge { connections: Vec<ConnectionId> }, // at most 64, ascending
    PeerIo { connection: ConnectionId, source: std::io::Error },
    ListenerIo(std::io::Error),
    ResourceExhausted(std::io::Error),
}

pub enum ClientPhase {
    Connect,
    HelloWrite,
    HelloRead,
    RequestWrite,
    ResponseRead,
    SubscribeWrite,
    InitialState,
    SubscriptionEvent,
}

pub enum ClientError {
    MissingRealm,
    Refused,
    Path(IpcPathError),
    VersionMismatch { client: u32, server: u32 },
    Timeout { phase: ClientPhase },
    FrameTooLarge { phase: ClientPhase },
    InvalidRequest,
    UnexpectedResponse { phase: ClientPhase },
    MalformedResponse { phase: ClientPhase },
    Eof { phase: ClientPhase },
    Io { phase: ClientPhase, source: std::io::Error },
}

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
    // Consumes the only public listener capability.
    pub fn into_server(self, now: Instant) -> ControlServer;
}

impl ControlServer {
    pub fn poll_interests(&self) -> impl Iterator<Item = PollInterest<'_>>;
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn service_one(
        &mut self,
        now: Instant,
        ready: ReadyEvent,
    ) -> Result<Option<ControlAction>, ControlError>;
    pub fn expire(&mut self, now: Instant) -> Result<(), ControlError>;
    pub fn complete_request(
        &mut self,
        now: Instant,
        connection: ConnectionId,
        response: Response,
    ) -> Result<(), ControlError>;
    pub fn complete_subscribe(
        &mut self,
        now: Instant,
        connection: ConnectionId,
        state: RealmState,
    ) -> Result<(), ControlError>;
    pub fn publish_state(
        &mut self,
        now: Instant,
        state: RealmState,
    ) -> Result<(), ControlError>;
    pub fn begin_shutdown(&mut self, now: Instant);
    pub fn is_shutdown_complete(&self) -> bool;
}

impl ClientEndpoint {
    // Exactly one descriptor-relative connection attempt; #41, not #218.
    pub fn connect(&self, client: &str) -> Result<Client, ClientError>;
}

impl Iterator for Subscription {
    type Item = Result<Event, ClientError>;
}
```

Issue #218 implements only `RuntimeDir`, `RealmDir`, `SocketEndpoint`,
`BoundControlEndpoint`, and `ActiveControlListener`. `ClientEndpoint`,
`Client`, `RuntimeDir::client_endpoint`, client realm reopen/attempt resolution,
and retry behavior are delivered by #41.

These signatures are the public conceptual boundary; concrete borrowed
iterator and error-source spelling may follow ordinary Rust conventions while
preserving every stated ownership and classification. `ActiveControlListener`
does not implement public `AsFd`: it is consumed into `ControlServer`.
`poll_interests` is the only server poll-registration boundary. Each returned
interest couples a borrowed fd to a stable `ControlToken`; the caller copies
only token and readiness into `ReadyEvent`, then drops every borrowed interest
before calling `service_one`. A raw fd is never a connection identity.

`RuntimeDir` retains the absolute public display path, the securely resolved
runtime-directory fd, and the daemon effective uid captured for validation.
`RealmDir` retains its separately validated directory fd and exposes a borrowed
fd for later descriptor-relative `ledger.json` work. `SocketEndpoint` denotes
only the exact fixed `realm/ctl.sock` descendant: it retains the realm
capability and a canonical public display path, but it exposes no way to replace
that descendant. Bound and active wrappers expose borrows of their endpoint and
`RealmDir`; neither wrapper is `Clone`. `BoundControlEndpoint` deliberately
does not implement `AsFd`, so code outside the one-shot transition cannot call
`listen`. `ActiveControlListener` keeps its listening fd private and exposes no
public fd borrow; the singleton lock descriptor is likewise private to the
ownership wrappers and no accessor or trait implementation exposes it.
`Client`, `Subscription`, and `ControlServer` are the later #41 transport
wrappers; naming them here is not a claim that the endpoint-only slice
implements that transport.

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
The active wrapper is a second short-lived consuming capability: it exposes no
public `AsFd` and transfers its listener, lock, endpoint, and cleanup identity
into `ControlServer`.

Drop for bound, active, or server ownership closes the socket fd first, then,
while still holding the singleton lock, no-follow-stats and unlinks only a
path whose device/inode, socket type, daemon-euid owner, and `0600` mode all
match the retained identity. A replacement visible at that stat is preserved.
The final stat-to-unlink step is not atomic and relies on the stated same-euid
trust boundary. Drop cleanup is best effort; abrupt process death is recovered
by the locked stale-reclaim procedure, not by weakening identity checks.

## Linux admission and connection state machine

The M2 session transport is Linux-only. `realm-core` remains portable;
Linux-specific endpoint and transport code lives in `realm-control`.
Unsupported targets fail that crate's build explicitly rather than omitting
peer admission or substituting pathname permissions.

The listener and every accepted stream are nonblocking. `accept4` atomically
sets `SOCK_NONBLOCK | SOCK_CLOEXEC`. Immediately after a successful accept and
before any receive, production obtains Linux `SO_PEERCRED` and admits only a
uid equal to the daemon effective uid. A lookup error, absent credential, or
different uid closes the stream without a protocol reply. Production uses the
safe APIs of the workspace-locked `rustix` 1.1.4; there is no production
credential-provider trait or injectable credential path. Tests may inject
credential outcomes only through a `#[cfg(test)]` constructor.

An accepted connection is exactly one of these states:

| State | Entry | Permitted input | Output / exit |
|---|---|---|---|
| `Admitting` | `accept4` succeeded | none | obtain credentials; close on failure, otherwise enter `AwaitHello` |
| `AwaitHello` | admitted | one complete Hello before its hard 1 s deadline | queue the server Hello; matching version enters `SendingHello`, mismatch enters `CloseAfterReply` |
| `SendingHello` | matching Hello decoded | at most one already-buffered post-Hello request or partial prefix is retained but not dispatched | send the complete Hello reply, then enter `Ready` and expose or finish the retained request |
| `Ready` | matching Hello reply completely sent | one decoded non-Hello request at a time | expose one `ControlAction::Request`; its one ordinary response drains before another request is admitted, or successful `Subscribe` enters `Subscriber` |
| `Subscriber` | `complete_subscribe` queued the initial snapshot | no positive input byte; clean read-half EOF is allowed | one current frame cursor plus one replaceable latest state |
| `CloseAfterReply` | terminal decodable protocol error or version mismatch | none | drain the one terminal reply for at most 100 ms, then close |
| `Closing` | EOF, a bound/deadline violation, or a close decision | none | close fd and release slot |

`std::time::Instant` is the monotonic type. Every server mutation accepts an
explicit `now: Instant`; there is no production `Clock` trait in
`realm-control`. Tests construct and advance chosen `Instant` values. The #38
combined loop captures one `Instant` per turn and supplies that value to all
server calls in the turn.

Read-side EOF is a half-close, not an unconditional full close. If EOF follows
complete authorized frames, the server stops reading, finishes the already
authorized Hello/request/subscription transition, and drains its bounded
output. Matching Hello plus one request followed by EOF therefore yields the
Hello reply and ordinary response in that order. Matching Hello plus Subscribe
followed by EOF remains a subscriber and may receive events. Matching Hello
alone may drain its Hello reply and then close. EOF before Hello or with an
incomplete frame closes immediately. A subscriber closes immediately on any
positive input byte, including the first byte of a forbidden frame, while a
clean read-half EOF is allowed.

## Frames, errors, capacity, and liveness

A frame is its UTF-8 JSON payload plus exactly one terminating LF. Its total
size is at most **65,536 bytes**, including that LF; 65,535 payload bytes plus
LF is valid. A 65,536-byte prefix without LF is already impossible to complete
validly and closes immediately; the implementation does not wait for byte
65,537. Invalid UTF-8 or syntactically invalid JSON closes the peer without a
reply. Valid JSON that cannot decode as `Request` (including an unknown request
variant or invalid request fields) gets one bounded `Response::Error` and
`CloseAfterReply`.

The same 65,536-byte total-frame bound applies to every request, response, and
event. Encoding writes into a bounded sink and stops as soon as the next output
would exceed the bound; it must not first retain an unbounded oversized
allocation. `complete_request` and `complete_subscribe` encode for their one
peer; an oversized frame closes that peer and returns
`ControlError::OutboundFrameTooLarge` with a singleton id collection.

`publish_state` is a no-op without subscribers. Otherwise it bounded-encodes
the event exactly once before mutating any queue. If encoding is oversized, it
closes every current subscriber, leaves every non-subscriber unchanged, and
returns all affected `ConnectionId`s in stable ascending order. That collection
is bounded by the 64-connection cap. No oversized diagnostic echoes untrusted
content, blocks, or crashes the session.

| State | Frame class | Result |
|---|---|---|
| `AwaitHello` | matching `Hello` | queue `Response::Hello`; enter `SendingHello` and do not dispatch a retained request until the reply fully drains |
| `AwaitHello` | mismatched `Hello` | queue only the server `Response::Hello`; discard any pipelined bytes and enter `CloseAfterReply` |
| `AwaitHello` | valid decodable non-Hello request | queue `Response::Error`; `CloseAfterReply` |
| `SendingHello` | one immediately buffered non-Hello request | retain it; expose it as a `ControlAction::Request` only after Hello fully drains |
| `Ready` | ordinary request | expose one `ControlAction::Request`; `complete_request` queues its response in request order |
| `Ready` | duplicate `Hello` | queue `Response::Error`; `CloseAfterReply` |
| `Ready` | `Subscribe` | expose it as `ControlAction::Request`; `complete_subscribe` queues the authoritative current snapshot and enters `Subscriber` |
| `Subscriber` | any positive input byte | close without reply |
| any input state | invalid UTF-8, invalid JSON, oversized/unterminated frame | close without reply |

One receive may scan and decode at most the mandatory Hello plus one
immediately buffered post-Hello request. This is the only two-frame exception
and is required for a shell that writes both LF-terminated frames before
reading. The second request is retained behind `SendingHello`; it has no side
effect before the Hello response drains. A third complete startup frame, or
buffered bytes establishing an excess later pipeline while that retained
request is pending, closes the peer without exposing either post-Hello request.
After startup, any later ordinary pipeline beyond the one pending request
closes that peer without side effects from the excess request.

Each admitted connection consumes one of **64** slots from `accept4` until its
fd is closed, including `Admitting`, `AwaitHello`, `SendingHello`,
`CloseAfterReply`, and draining subscribers. `ControlServer::service_one`
handles exactly one ready token and performs at most one accept, one receive,
or one send syscall. Listener readiness therefore causes at most one
`accept4`, never a drain-to-`EAGAIN` loop. At capacity, that one accept still
occurs and the excess fd is immediately closed without allocating a connection
record; at most one excess fd is refused per listener service quantum.

`poll_interests` and `service_one` use deterministic arbitration:

- Before shutdown, the listener advertises readable interest. `AwaitHello` and
  a read-enabled `Ready` connection advertise readable interest.
- `SendingHello` and `CloseAfterReply` never advertise or service read
  readiness. A Ready connection also disables read interest while its emitted
  application action awaits completion or while ordinary output is queued;
  only the applicable writable interest is exposed for queued output.
- A read-open subscriber advertises readable interest and advertises writable
  interest whenever it has current output. It may therefore advertise both.
  When its copied `ReadyEvent` is simultaneously readable and writable,
  `service_one` performs the receive first. A positive forbidden byte closes
  it without a send; clean EOF removes its future read interest.
- At the start of `service_one`, before resolving readiness or performing any
  accept, receive, or send, the server applies the same bounded global expiry
  pass as `expire(now)` to every applicable `now >= deadline`. Due peers close
  in stable `ConnectionId` order, and a due shared shutdown deadline closes its
  remaining subscriber set and marks shutdown complete. If that pass closes the
  supplied token, the call performs no socket I/O; a token already stale before
  the pass returns its stable rejection without I/O.
- After that deadline gate, listener readable readiness selects one accept;
  subscriber readiness follows the read-before-write rule; eligible
  `AwaitHello`/`Ready` readable readiness selects one receive; and writable
  readiness for queued output selects one send. Readiness not enabled by the
  current state performs no socket I/O.

For `AwaitHello` and `Ready`, input consists of at most one partial frame and
one decoded-but-not-completed request, except for the bounded Hello-plus-one
startup receive above. Output consists of at most one ordinary protocol frame,
whether Hello, `Response`, or `Error`, plus its cursor. Responses remain in
request order. `Subscribe`'s initial snapshot is subscriber output, not
ordinary output; no state snapshot, error, or Hello is exempt from byte,
queue, or deadline accounting.

The 64-slot cap guarantees bounded memory, bounded work per service quantum,
no blocking syscall, eventual partial-frame eviction, and no mutation of an
already-admitted peer's state merely because another peer stalls or is refused.
It does not promise availability against 64 trusted same-euid peers holding all
slots, and it does not claim that such peers cannot delay admission of a new
peer.

### Server deadlines

- `AwaitHello` has a hard one-second deadline from successful `accept4`;
  expiry is exactly `now >= deadline`.
- In `Ready`, a partial frame has a hard two-second completion deadline from
  its first positive byte. Later bytes do not extend it. An empty Ready
  connection may remain idle indefinitely. A post-Hello partial prefix retained
  in `SendingHello` uses that same first-byte deadline; sending Hello does not
  postpone it.
- Ordinary output and subscriber output have a two-second no-progress deadline
  beginning when the frame is queued and resetting only after a positive send.
  `EAGAIN`, `EINTR`, and a zero-byte result do not reset it.
- `CloseAfterReply` has a hard 100 ms deadline from queue time and never resets
  on progress.
- `begin_shutdown` establishes one hard 100 ms deadline for the whole shutdown
  drain. Its first call fixes that deadline; later calls are idempotent and
  cannot move it. It never resets on progress.

The first write attempt occurs on the next applicable bounded control quantum.
The caller may not delay that quantum indefinitely after a frame is queued.
`next_deadline` includes all applicable deadlines, while `expire(now)` closes
every connection whose exact deadline has arrived without performing socket
I/O.

### Subscriber queue and shutdown

A subscriber owns one current output cursor and one replaceable latest-state
slot. The immediate initial snapshot installed by `complete_subscribe` is the
current cursor even at offset zero and is never replaced. Once any state A is
current, later B then C replace only the latest slot, so the delivered order is
A then C whether or not A's first byte has been written. When current completes,
the latest slot, if present, becomes current. This is coalescing, not an
unbounded queue.

Shutdown treats the initial snapshot specially. If it is still current and
unstarted at offset zero, preserve it and replace the latest slot, if any, with
`Event::Shutdown`; delivery can therefore be initial State followed by
Shutdown. If the initial snapshot is partial, discard latest, finish only the
partial initial frame, and queue no Shutdown. Once the initial snapshot has
completed, a later non-initial current frame at offset zero may be replaced by
Shutdown; a partial non-initial current frame is finished alone; and with no
current frame Shutdown is queued directly. In all cases any replaceable state
is discarded. After its last permitted frame drains, the server closes that
subscriber. The shared drain ends at its hard 100 ms deadline.

`Client::subscribe` does not succeed until it has received the complete initial
State. If the hard server shutdown deadline prevents that delivery, the call
returns its normal phase-specific `Timeout { phase: InitialState }` or
`Eof { phase: InitialState }`, according to what it observes; it never fabricates
a successful `Subscription`. After successful Subscribe, `Subscription` yields
Shutdown exactly once and then ends; clean EOF ends it without synthesizing
Shutdown.

### Total shutdown transition

The first `begin_shutdown(now)` call atomically fixes the hard deadline,
removes listener poll interest, stops admission, and closes every
non-subscriber. This includes `AwaitHello`, `SendingHello`, read-open or
read-half-closed `Ready`, queued ordinary output, and connections whose emitted
application request or Subscribe is still in flight. It emits no new
`ControlAction`; no later readiness can create a connection or action.

Subscribers alone follow the queue rules above. #38 services their remaining
readable/writable interests and calls `expire` using one captured `Instant` per
turn until `is_shutdown_complete()` is true. The predicate becomes true as
soon as every subscriber has drained/closed, or when `expire(now)` observes the
shared `now >= deadline`, closes all remaining subscribers without I/O, and
marks the drain complete. Repeating `begin_shutdown` is a no-op and
does not move the deadline or rebuild output.

After shutdown begins, `complete_request`, `complete_subscribe`, and
`publish_state` return stable `ControlError::ShuttingDown` without encoding,
queue mutation, or publication. A `ConnectionId` already disconnected or
unknown before shutdown may still return `StaleConnection`; ids closed by the
shutdown transition remain classified as `ShuttingDown` for the bounded
remaining server lifetime.

### Stable identity and completion boundary

`ConnectionId` and `ControlToken` are opaque stable identities and are never
raw fds. Closing and later reusing an fd cannot make an old token or completion
refer to the new peer. `poll_interests` exposes borrowed fds only long enough
to register them; all borrows are dropped before copied `ReadyEvent` values are
passed back to `service_one`.

Every decoded non-Hello `Request`, including `Subscribe`, is emitted unchanged
as `ControlAction::Request`. The transport is vocabulary-agnostic and performs
no session operation. The #38 authoritative adapter supplies ordinary
completion with `complete_request` or subscriber completion with
`complete_subscribe`. `Response::Error` supplied by the application is an
ordinary response, not a `CloseAfterReply` terminal protocol error. After any
application response drains, including `Response::Error`, a read-open peer
returns to `Ready`; a read-half-closed ordinary peer closes. A completion for a
disconnected or stale `ConnectionId` returns a stable rejection and cannot
touch any reused fd. The shutdown precedence above applies after
`begin_shutdown`.

### Syscall classification

Socket I/O uses safe `rustix` 1.1.4 operations; every send includes
`MSG_NOSIGNAL`. Results are classified completely:

| Operation/result | Classification |
|---|---|
| `accept4`: `EINTR`, `EAGAIN`/`EWOULDBLOCK`, or `ECONNABORTED` | End this quantum without a peer; a later readiness turn may retry |
| `accept4`: `EMFILE`, `ENFILE`, `ENOBUFS`, or `ENOMEM` | Fatal `ControlError`; the process cannot preserve bounded service under resource exhaustion |
| `accept4`: any other error | Fatal listener `ControlError` |
| `SO_PEERCRED`: any error, missing value, or wrong uid | Close only the accepted peer before receive |
| receive/send: `EINTR` or `EAGAIN`/`EWOULDBLOCK` | No progress; retain state and deadlines |
| receive: zero bytes | Apply the read-half EOF rules above |
| receive/send: `ECONNRESET`; send: `EPIPE` | Close only that peer |
| other connected-stream receive/send error | Close only that peer and return a stable peer-local diagnostic |

`ControlError::ListenerIo` and `ControlError::ResourceExhausted` are fatal.
`StaleConnection`, `OutboundFrameTooLarge`, and `PeerIo` are stable peer-local
results after the affected peer has already been isolated or closed;
`ShuttingDown` is the stable no-mutation rejection after the total shutdown
transition.

Global `poll` failures and the decision to restart the session remain #38
outer-loop responsibilities, not `realm-control` transport policy.

## Combined-loop ownership and ordering

Issue #41 owns this reusable transport, its deterministic one-token/one-I/O
quantum, its client, and transport-order evidence. Issue #38 owns the combined
backend/listener/connection/worker poll loop and the authoritative session
adapter. Each #38 turn first services pending backend repair or backend work,
then performs at most one control operation, then performs a zero-time backend
readiness check before it may perform another control operation. No token
bucket or audit-only rate limiter is added for MVP.

For user-requested shutdown, #38 calls `begin_shutdown` once, continues the
same backend-first bounded-loop ordering while driving subscriber interests and
`expire`, and invokes River `exit_session` only after
`is_shutdown_complete()` becomes true.

Issue #40 supplies the real River `manage_finish`; #65 owns the real combined
loop key-to-`manage_finish` performance assertion together with #38/#40. The
#41 transport half must prove bounded nonblocking work and ordering seams, but
cannot claim the real end-to-end A17 measurement in isolation.

For `GetState`, and for the initial state passed to `complete_subscribe`, the
#38 adapter uses the last visible `Session::state()` accepted for publication.
`SessionUpdate.state == None` is never published. Mutating completion and the
complete Request-to-Response mapping remain SPEC 0003/SPEC 0006 authority.

## Client startup race (#41, not #218)

The reusable `ClientEndpoint` retains its validated `RuntimeDir` capability.
`ClientEndpoint::connect(&self, client)` makes exactly one descriptor-relative
connection attempt and uses the explicit client name in Hello. It reopens and
validates `realm` from the retained runtime fd, creates a fresh procfd bridge
from that `RealmDir`, and connects through the bridge. It never rereads
`XDG_RUNTIME_DIR`, creates `realm`, or resolves/connects through the canonical
display path. An absent fixed `realm` descendant is `ClientError::MissingRealm`,
not `UnsafeRealmDirectory`.

Immediate connect success proceeds. `EINPROGRESS` or the Unix connect form of
`EAGAIN` polls for writability against a hard 100 ms completion deadline, then
uses `SO_ERROR` for the final classification. Poll `EINTR` may resume only
within the same deadline and does not repeat `connect`; timeout is terminal.
An immediate connect `EINTR`, `EALREADY`, poll failure, or any unlisted connect
result is terminal `Io { phase: Connect, .. }`; exactly one `connect` syscall is
made.
Final `ENOENT`/missing realm maps to retryable `MissingRealm`, and final
`ECONNREFUSED` maps to retryable `Refused`. Those are the only retryable client
errors. Every path-validation error, timeout, and other I/O result is terminal.

From connect completion, the complete Hello send and Hello response share one
hard two-second exchange deadline. Each later ordinary request send and
response receive shares a new hard two-second deadline. Subscribe send through
the complete initial `Event::State` likewise shares one new hard two-second
deadline. Client sends use `MSG_NOSIGNAL`, and all client frames use the same
65,536-byte bounded encoding/decoding rule as the server. Subscription idle
after its initial event is unbounded; after the first positive byte of any
event, that frame has a hard two-second completion deadline. During bounded
client exchanges, send/receive `EINTR` resumes within the unchanged deadline,
`EAGAIN`/`EWOULDBLOCK` waits for the required readiness within that deadline,
and `EPIPE`/`ECONNRESET` is terminal phase-specific `Io`.

`ClientError` plus `ClientPhase` preserves the distinctions needed by callers:
`MissingRealm`, `Refused`, path validation, version mismatch carrying both
versions, timeout phase, oversized frame, invalid client request,
unexpected/malformed response, EOF phase, and other I/O phase. A client-side
ordinary request rejects Hello and Subscribe as `InvalidRequest`; Subscribe has
its consuming method. Application `Response::Error` is returned as a normal
`Response`, not promoted to `ClientError`.

`realmctl`, not the single-attempt `ClientEndpoint`, owns startup retry. At
driver start it fixes absolute not-before offsets of 0, 10, 30, 70, 150, and
310 ms. Attempts never overlap and execute in order. After a retryable
completion, the driver sleeps only until the next absolute target when that
target is still in the future; if the preceding attempt completed late, the
next attempt starts immediately at the current time. It never subtracts time,
travels backward, or sleeps after the sixth failure. Thus immediate retryable
results start at exactly the six targets, while slow attempts may start later
but never earlier. Only `MissingRealm` and `Refused` advance the schedule. Its
retry driver has injected sleeper and time seams. Exhaustion is exit 3, version
mismatch is exit 4, and every other transport/path/I/O error is exit 6.
`doctor` retains its separate no-session behavior.

## Protocol-vocabulary boundary

The #41 transport decodes and emits non-Hello `Request` values but does not
implement their meaning. Accepted SPEC 0006 currently records protocol drift:
before the first production `realmctl` control command ships, its typed Error
kind, complete `OrbitLedger` fields, `GetHealth`/`Health` schema, and the
corresponding `PROTOCOL_VERSION` bump must land together. No production v1
server/client pair is declared final merely because transport tests exercise
the current enum.

The Hello request/response envelope (`cmd=hello` / `reply=hello`, version plus
client/session fields) is the permanently stable refusal envelope across
protocol versions. An incompatible version may add fields, but it may not make
that envelope undecodable by the other version; this is what preserves a useful
version-mismatch result.

## Test seams and acceptance criteria

Production server methods take `Instant`; fake time consists only of caller-
chosen `Instant` values. Credential outcome injection is confined to a
test-only constructor. Client retry tests use the `realmctl` sleeper/time seam.
The table assigns evidence to the issue that can actually produce it: #218 owns
A1-A9 (including their lettered refinements) and A11; #41 owns A2's client
half, A10, A13-A16, bounded transport-order/work evidence, and shell
interoperability; A12 is #38. The real combined-loop performance budget is
#38/#40/#65.

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
| A12 | Given recovery is incomplete, when the endpoint is bound, then it is non-listening and readiness is absent; only after transition to `Live` may activation verify `SO_ACCEPTCONN` and precede `READY=1` | #38 pending: `realm_session::tests::readiness_follows_live_listener_activation` |
| A13 (#41) | Given foreign, missing, or failed real Linux peer credentials, when one connection is accepted with atomic fd flags, then no receive occurs before `SO_PEERCRED`, only that peer closes on rejection, and the safe rustix path admits a same-euid peer | `realm_control::tests::linux_admission_checks_real_credentials_before_receive`, `realm_control::tests::rejected_credentials_close_before_receive_or_identity_allocation`, `realm_control::compile_fail::bound_endpoint_cannot_activate_twice`, `realm_control::control_socket_linux::public_client_and_server_interoperate_with_actual_same_euid_admission` |
| A13a (#41) | Given any listener or connected-stream syscall result in the classification table, when one ready token is serviced, then the specified retry, peer close, or fatal result occurs and sends use `MSG_NOSIGNAL` | `realm_control::tests::listener_errno_classification_and_one_accept_quantum_are_total`, `realm_control::tests::connected_errno_classification_is_peer_local_and_total`, `realm_control::tests::production_send_path_supplies_msg_nosignal` |
| A14 (#41) | Given every state/frame case, including `SendingHello`, mismatch pipelining, matching Hello plus one retained request, and an excess third or later pipelined frame, when bounded quanta run, then replies/actions are ordered and excess input has no side effect | `realm_control::tests::protocol_state_machine_is_total`, `realm_control::tests::hello_reply_drains_before_retained_request_dispatch`, `realm_control::control_socket_linux::matching_hello_and_get_state_survive_write_half_eof_in_order`, `realm_control::control_socket_linux::mismatched_hello_discards_a_pipelined_request_and_sends_only_hello` |
| A14a (#41) | Given no Hello by one second, a Ready partial frame whose first byte is two seconds old, or exactly 65,536 input bytes without LF, when `now >= deadline` or the impossible prefix arrives, then only that peer closes; later partial bytes do not slide the deadline | `realm_control::tests::hard_read_deadlines_and_impossible_full_prefix_close_exactly` |
| A14b (#41) | Given matching Hello plus one Request or Subscribe followed by read-half EOF, when the server completes it, then Hello precedes the response, an ordinary peer closes after its application response including Error, or the subscriber remains able to receive events; EOF before Hello, with a partial frame, or positive subscriber input closes | `realm_control::tests::half_close_preserves_authorized_two_frame_work_and_subscribers`, `realm_control::tests::read_half_closed_application_error_drains_then_closes`, `realm_control::control_socket_linux::matching_hello_and_get_state_survive_write_half_eof_in_order`, `realm_control::control_socket_linux::matching_hello_and_subscribe_survive_clean_read_half_eof` |
| A15 (#41) | Given listener readiness below or at the 64-slot cap, when `service_one` runs, then it performs at most one accept; at capacity it closes exactly one excess accepted fd without a record, and admitted peer state is unchanged | `realm_control::tests::one_ready_token_performs_at_most_one_socket_io`, `realm_control::tests::capacity_refusal_is_one_fd_per_quantum_and_allocates_nothing` |
| A15a (#41) | Given a connection closes and its numeric fd is reused, when stale readiness or completion arrives, then its stable token/`ConnectionId` is rejected without touching the new peer, and no public listener `AsFd` bypass exists | `realm_control::tests::stale_token_and_connection_id_cannot_target_reused_fd`, `realm_control::compile_fail::bound_endpoint_cannot_activate_twice` |
| A15b (#41) | Given an oversized response, initial state, or published event, when bounded encoding runs, then one-peer completion closes and reports its singleton id; publication with no subscribers performs no encode/mutation; and publication with subscribers encodes once before queue mutation, closes all and only subscribers, and returns their at-most-64 ids in ascending order without untrusted content | `realm_control::tests::outbound_frame_bound_is_streaming_peer_local_and_stable`, `realm_control::tests::oversized_publish_closes_sorted_subscriber_set_after_one_encode` |
| A15c (#41) | Given every state and simultaneous readiness combination, when interests are built and one token is serviced, then pending application/ordinary output and SendingHello/CloseAfterReply disable reads, a subscriber advertises both when applicable and services readable first, and irrelevant readiness performs no I/O | `realm_control::tests::poll_interest_and_ready_arbitration_is_total` |
| A16 (#41) | Given queued ordinary, subscriber, terminal, or shutdown output, when positive sends, `EAGAIN`, and exact deadlines occur, then `service_one` expires every applicable `now >= deadline` before socket I/O, only ordinary/subscriber progress resets its two-second deadline, hard 100 ms deadlines never reset, and the first applicable quantum attempts the write | `realm_control::tests::all_output_classes_obey_exact_nonblocking_deadlines`, `realm_control::tests::service_quantum_expires_before_socket_io`, `realm_control::control_socket_linux::a_full_send_buffer_returns_from_one_nonblocking_quantum` |
| A16a (#41) | Given initial State A and later states B/C, when output drains, then A remains current and A followed by C is sent; when shutdown starts, an unstarted initial A is preserved before Shutdown, a partial initial/current finishes alone, and only a later non-initial unstarted current may be replaced by Shutdown. If initial delivery misses the hard drain, the client Subscribe returns InitialState timeout/EOF rather than success | `realm_control::tests::subscriber_current_and_latest_coalesce_and_shutdown_exactly`, `realm_control::tests::shutdown_preserves_unstarted_initial_state_before_shutdown` |
| A16b (#41) | Given immediate connect, in-progress connect, every final `SO_ERROR`, or a client exchange/event deadline, when one named-client attempt runs, then phase-specific errors, the 100 ms connect bound, each two-second exchange bound, and the sole retryable MissingRealm/Refused classifications are exact | `realm_control::tests::client_error_preserves_all_phases_versions_and_exact_retryability`, `realm_control::tests::single_attempt_client_immediate_connect_sends_named_hello_on_cloexec_nonblocking_stream`, `realm_control::tests::single_attempt_client_in_progress_so_error_table_is_total_and_connects_once`, `realm_control::tests::single_attempt_client_connect_terminal_errors_and_hard_timeout_are_exact`, `realm_control::tests::client_hello_uses_one_hard_two_second_deadline_and_msg_nosignal`, `realm_control::tests::client_hello_response_errors_are_phase_specific_and_bounded`, `realm_control::tests::client_request_write_and_response_share_one_hard_deadline`, `realm_control::tests::client_subscribe_write_and_initial_state_share_one_hard_deadline`, `realm_control::tests::client_complete_buffered_event_does_not_slide_trailing_partial_deadline`, `realm_control::tests::client_subscription_idle_is_unbounded_but_partial_event_deadline_is_hard` |
| A16c (#41) | Given immediate retryable failures, when `realmctl` exhausts startup retry, then attempts start at absolute not-before offsets 0, 10, 30, 70, 150, and 310 ms with no final sleep; given a slow attempt finishing after its next target, the following attempt starts immediately but never overlaps or moves backward; exhaustion/version/other transport errors map to exits 3/4/6 | `realm_ctl::control_retry::immediate_retryable_failures_use_exact_absolute_schedule_without_a_final_sleep`, `realm_ctl::control_retry::late_retryable_attempts_start_immediately_when_their_targets_have_elapsed`, `realm_ctl::control_retry::terminal_error_stops_after_its_first_attempt`, `realm_ctl::control_retry::classifier_uses_the_specified_client_exit_codes` |
| A16d (#41) | Given a shell writes matching Hello and one request as two LF-terminated frames before reading, then it receives a decodable Hello followed by the request response and EOF does not discard either | CI-only remote evidence pending: `realm_control::control_socket_linux::shell_two_frame_interoperability_is_ordered` |
| A16e (#41 transport; #38 driver) | Given any server state, when shutdown begins once or repeatedly, then the first call fixes the deadline, listener/admission/actions stop, all non-subscribers close, later completions/publication reject as `ShuttingDown` without mutation while ids already stale may remain `StaleConnection`, subscribers obey the initial/current rules, and completion becomes true on empty subscribers or exact expiry; #38 drives interests/expiry before `exit_session` | `realm_control::tests::shutdown_transition_is_total_idempotent_and_completion_observable`; #38 pending: shutdown-driver test |
| A17 (#41 transport; #38/#40/#65 end to end) | Given adversarial ready control tokens, when the transport is serviced, then each quantum has bounded work, makes at most one socket I/O syscall, and exposes no write before its queued action completion; given the real combined loop, stalled peers, and a key event, #38 services backend/pending repair first and rechecks backend readiness between control operations, while #40/#65 prove real key-to-`manage_finish` remains below 4 ms | `realm_control::tests::one_ready_token_performs_at_most_one_socket_io`, `realm_control::tests::poll_interest_and_ready_arbitration_is_total`, `realm_control::control_socket_linux::a_full_send_buffer_returns_from_one_nonblocking_quantum`, `realm_control::control_socket_linux::a_healthy_peer_completes_beside_a_stalled_partial_frame`; #38/#40/#65 pending: combined-loop order and real key-to-`manage_finish` performance |

## Failure modes

This prevents the runtime-fallback, pathname-race, unauthenticated-local-peer, protocol-version-skew, unbounded-pipeline, and wedged-subscriber failures recorded in ADR 0004. No failure permits a fallback endpoint, a best-effort path-security check, an unbounded queue, or a blocking write.

## Open questions

None. Remote or cross-user control requires a new ADR and specification.
