# SPEC 0007 — Control-socket transport and security

- **Status:** Accepted (2026-08-28)
- **Milestone:** M2
- **Decisions:** [ADR 0004](../adr/0004-ndjson-control-socket.md)
- **Amends:** [SPEC 0001](0001-helm-core-contracts.md), [SPEC 0003](0003-helm-session.md), [SPEC 0006](0006-helm-ctl.md)

## Purpose and authority

The control socket can spawn processes and change the live desktop. It must be scriptable without making an absent runtime directory, a caller-selected path, or an unbounded local peer a control or liveness boundary.

This is the complete **Accepted** implementation specification for the control-socket transport. SPEC 0003 remains Draft for unrelated river/session questions; its §7 is an integration constraint only. Where its historical socket prose differs from this specification, this specification wins. No M2 transport implementation may use a Draft requirement as a substitute for a decision here.

## Scope

**In:** runtime-path resolution, endpoint creation and reclaim, Linux peer admission, the connection state machine, framed I/O limits, client startup retry, and the test seams needed to verify those rules.

**Out:** a remote-control protocol, authorization between different local users, and changes to the JSON request/response/event vocabulary. An admitted same-euid process is trusted for every existing command; directory mode is defence in depth, not per-command authorization.

## Ownership, target, and API boundary

`helm-core::ipc` owns only portable wire types, `encode`, `decode`, and `PROTOCOL_VERSION`. It performs no environment lookup, filesystem operation, credential query, or socket operation. The legacy M0 `helm_core::ipc::socket_path()` helper is not part of the accepted M2 API and is removed when the transport is introduced.

The shared Linux transport support module used by `helm-session` and `helm-ctl` owns endpoint resolution. Its public boundary is:

```rust
pub struct RuntimeDir(/* validated directory descriptor */);
pub struct SocketEndpoint { pub path: PathBuf }

pub enum IpcPathError {
    MissingRuntimeDir,
    UnsafeRuntimeDir,
    UnsafeHelmDirectory,
    UnsafeSocketEntry,
    Io(std::io::Error),
}

pub trait RuntimeDirResolver {
    fn resolve(&self) -> Result<RuntimeDir, IpcPathError>;
}

pub fn production_runtime_dir() -> Result<RuntimeDir, IpcPathError>;
pub fn test_runtime_dir(path: &Path) -> Result<RuntimeDir, IpcPathError>;
pub fn fixed_endpoint(runtime: &RuntimeDir) -> SocketEndpoint;
```

`production_runtime_dir` alone reads `XDG_RUNTIME_DIR`. It rejects an absent, relative, non-directory, non-euid-owned, or group/world-accessible directory; the latter cases are `UnsafeRuntimeDir`, while absent/relative/non-directory is `MissingRuntimeDir`. `test_runtime_dir` accepts an explicit absolute temporary runtime **directory**, applies exactly the same descriptor/type/owner/mode checks, and never accepts a socket path. `fixed_endpoint` always denotes the single descendant `helm/ctl.sock`; `/tmp`, `HELM_SOCKET`, and a CLI socket-path override do not exist in production or the fixture API.

All filesystem operations after resolution are descriptor-relative. On Linux, the resolver opens the runtime directory with `openat2` using `RESOLVE_NO_SYMLINKS | RESOLVE_NO_MAGICLINKS`, `O_DIRECTORY`, and `O_CLOEXEC`; an unavailable or failing secure resolution fails closed. It retains that descriptor. It creates/opens `helm` using `mkdirat` then the same no-symlink descriptor-relative resolution, and uses `fstat`, `fstatat(..., AT_SYMLINK_NOFOLLOW)`, and `unlinkat` relative to those retained descriptors. No check-then-use operation may re-resolve an attacker-controlled pathname.

The runtime directory and `helm` directory must both be directories owned by the daemon effective uid and have mode exactly `0700`; an existing object with any other type, owner, or mode is rejected and never chmodded. The daemon creates `helm` as `0700`. `ctl.sock`, when present, must be a socket owned by that uid with mode exactly `0600`; every other existing entry, including a symlink, is rejected. Before binding, while the process is still single threaded, the daemon uses a `0177` umask so bind creates `0600`; it then rechecks the directory entry without following links and compares its `st_dev/st_ino` with the listener descriptor. Binding uses the retained `helm` directory descriptor (not a fresh resolution of `$XDG_RUNTIME_DIR`).

## Listener ownership and stale reclaim

If `ctl.sock` exists and passes the socket/uid/mode checks, the daemon tests it with a fresh nonblocking `AF_UNIX/SOCK_STREAM` socket. Only an immediate `ECONNREFUSED` authorizes reclaim: Linux documents it as no listener for a stream socket. `EAGAIN`, `EINPROGRESS`, `EALREADY`, `ETIMEDOUT`, `EACCES`, `EPERM`, resource errors, and every other result do **not** authorize unlink; the daemon exits non-zero and leaves the entry unchanged. A nonblocking in-progress connect is polled for writability for at most 100 ms and is stale only if `SO_ERROR` is `ECONNREFUSED`; timeout is not stale.

Immediately before `unlinkat`, the daemon re-runs no-follow `fstatat` and requires the same device/inode, socket type, euid ownership, and `0600` mode observed before probing. Immediately after bind it makes the analogous directory-entry/listener-descriptor identity check. Any mismatch is a path race: close the listener, do not unlink any replacement, and fail startup.

## Linux admission and connection state machine

The M2 session transport is Linux-only. `helm-core` remains portable; the Linux session/client transport is compiled only under `target_os = "linux"`. Unsupported targets must fail the transport build explicitly rather than omit peer admission or substitute pathname permissions.

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

## Client startup race

For a normal command, `helm-ctl` retries only `ENOENT` and `ECONNREFUSED`, with five nonblocking attempts separated by 10, 20, 40, 80, and 160 ms (310 ms maximum waiting). It performs the mandatory Hello on the successful connection. Any other connect/path error fails immediately; exhausting the retry schedule is exit code 3. `doctor` retains its existing no-session reporting behavior.

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
| A1 | Given invalid production runtime input, when resolved, then the exact path error is returned and no fallback/override is read | `ipc::tests::socket_path_requires_xdg_runtime_dir`, `ipc::tests::production_path_ignores_helm_socket` |
| A2 | Given a test runtime directory, when resolved, then only its fixed descendant is used and production owner/type/mode checks still apply | `ipc::tests::test_runtime_dir_resolver_preserves_the_fixed_descendant` |
| A3 | Given foreign or missing credentials, when accepted, then no byte is read; a same-euid peer remains live | `control_socket::tests::rejects_foreign_uid_before_read`, `control_socket::tests::rejects_missing_peer_credentials_before_read` |
| A4 | Given symlinks, wrong modes/types, an identity race, or an existing socket, when binding, then unsafe entries are refused and only verified `ECONNREFUSED` stale sockets are reclaimed | `control_socket::tests::path_resolution_is_descriptor_relative`, `control_socket::tests::stale_same_uid_socket_is_reclaimed_only_after_verified_refusal` |
| A5 | Given every state/error-table case, when a frame is read, then its reply (if any), deadline, and close/continue result match the table | `control_socket::tests::protocol_state_machine_is_total` |
| A6 | Given 64 occupied slots or an excess frame/queue, when the limit is reached, then only that peer is closed and listener draining continues | `control_socket::tests::connection_and_queue_limits_preserve_admitted_peers` |
| A7 | Given a non-reading ordinary client, mismatch client, subscriber, or shutdown, when its applicable drain deadline expires, then it is evicted/exited without blocking | `control_socket::tests::all_write_classes_have_bounded_nonblocking_drain` |
| A8 | Given a stalled peer and a key event, when the loop runs, then `manage_finish` precedes every write and real Linux measurement remains below 4 ms | `control_socket::tests::stalled_peer_preserves_key_path`, `control_socket::tests::linux_key_path_budget_with_full_socket_buffer` |

## Failure modes

This prevents the runtime-fallback, pathname-race, unauthenticated-local-peer, protocol-version-skew, unbounded-pipeline, and wedged-subscriber failures recorded in ADR 0004. No failure permits a fallback endpoint, a best-effort path-security check, an unbounded queue, or a blocking write.

## Open questions

None. Remote or cross-user control requires a new ADR and specification.
