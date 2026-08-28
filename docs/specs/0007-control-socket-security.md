# SPEC 0007 — Control-socket security amendment

- **Status:** Accepted (2026-08-28)
- **Milestone:** M2
- **Decisions:** [ADR 0004](../adr/0004-ndjson-control-socket.md)
- **Amends:** [SPEC 0001](0001-helm-core-contracts.md),
  [SPEC 0003](0003-helm-session.md), [SPEC 0006](0006-helm-ctl.md)
- **Supersedes / Superseded by:** —

## Purpose

The control socket can spawn processes and change the live desktop. It must be
scriptable without making an absent runtime directory, a caller-selected path,
or an unbounded local peer a control or liveness boundary.

## Scope

**In:** runtime-path resolution, listener path safety, peer admission, the
mandatory version handshake, subscription transition, and bounded socket I/O.

**Out:** a remote-control protocol, authorization between different local
users, and changes to the JSON request/response/event vocabulary.

## Behaviour

1. Production resolves only `$XDG_RUNTIME_DIR/helm/ctl.sock`. Missing, relative
   or non-directory `XDG_RUNTIME_DIR` produces `IpcPathError::MissingRuntimeDir`;
   `/tmp`, `HELM_SOCKET`, and `helmctl --socket` are not fallbacks or overrides.
2. Tests may inject a temporary runtime **directory** through a non-production
   resolver. It derives the same `helm/ctl.sock` child and retains production
   ownership/type checks; tests cannot inject an arbitrary socket path.
3. The daemon creates `helm` mode 0700 and the listener mode 0600 without
   following links. It rejects an unsafe pre-existing path. It may reclaim only
   a same-euid stale socket after a failed connect proves no listener exists.
4. Before it reads, the listener obtains `SO_PEERCRED` and admits only a peer
   whose uid is its effective uid. Credential failure or a different uid closes
   that connection without a reply.
5. Hello is the first complete frame for every connection and arrives within
   one second. It receives `Response::Hello`; mismatch receives that response
   then EOF. Ordering violations receive `Response::Error` only if decodable,
   then EOF.
6. After a matching Hello, ordinary requests are permitted. `Subscribe` is
   terminal: it emits an immediate state snapshot and accepts no more client
   frames.
7. At most 64 connections exist; a complete frame is at most 64 KiB. Apart from
   the first Hello, at most one complete inbound request and one ordinary
   response may be queued per connection. Exceeding a bound closes only that
   peer.
8. Subscriber output is one latest-state frame plus a partial write. Nonblocking
   writes that make no progress for two seconds cause eviction; no socket peer
   may block the window-management event loop.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given missing, relative, or invalid `XDG_RUNTIME_DIR`, when production resolves the socket, then it returns `IpcPathError::MissingRuntimeDir` and does not probe `/tmp` or honour `HELM_SOCKET` | `ipc::tests::socket_path_requires_xdg_runtime_dir`, `ipc::tests::production_path_ignores_helm_socket` |
| A2 | Given a test temporary runtime directory, when the fixture resolver runs, then it derives only `helm/ctl.sock` and cannot escape that directory | `ipc::tests::test_runtime_dir_resolver_preserves_the_fixed_descendant` |
| A3 | Given a foreign or credential-less peer, when accepted, then it is closed before a frame is read while a same-euid peer remains live | `control_socket::tests::rejects_foreign_uid_before_read`, `control_socket::tests::rejects_missing_peer_credentials_before_read` |
| A4 | Given an unsafe path entry or stale same-euid socket, when binding, then the unsafe entry is refused and the stale socket is reclaimed only after failed connect | `control_socket::tests::refuses_symlink_or_wrong_type_at_runtime_path`, `control_socket::tests::stale_same_uid_socket_is_reclaimed_only_after_connect_fails` |
| A5 | Given a non-Hello first frame, duplicate Hello, or a Hello after one second, when read, then only that peer is closed and a healthy peer remains usable | `control_socket::tests::hello_is_required_before_every_request`, `control_socket::tests::hello_deadline_closes_idle_peer` |
| A6 | Given a version mismatch, when Hello is read, then the server writes its Hello response and closes before any other response | `control_socket::tests::version_mismatch_gets_hello_then_eof` |
| A7 | Given matching Hello then Subscribe, when accepted, then an immediate snapshot is emitted; Subscribe before Hello or later input is rejected | `control_socket::tests::subscribe_requires_matching_hello_and_emits_snapshot` |
| A8 | Given saturated connection/frame/queue limits or a stalled subscriber, when the limit is reached, then only the offending peer is closed and the key-press budget still holds | `control_socket::tests::connection_limit_rejects_without_harming_admitted_peers`, `control_socket::tests::oversized_or_unterminated_frame_is_closed`, `control_socket::tests::stalled_subscriber_is_evicted_without_missing_key_budget` |

## Budgets

The socket may not consume the `< 4 ms` key-press-to-`manage_finish` budget in
[SPEC 0003](0003-helm-session.md). Admission and all I/O are nonblocking; the
two-second subscriber bound is a liveness ceiling, not a normal-path delay.

## Failure modes

Prevents the runtime-fallback, unauthenticated-local-peer, protocol-version
skew, and wedged-subscriber failures recorded in ADR 0004. No failure permits
a fallback endpoint, an unbounded queue, or a blocking write.

## Open questions

None. Any remote or cross-user control surface requires a new ADR and spec.
