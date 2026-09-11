# Bounded Control Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver issue #41's bounded Linux control server, single-attempt client, and exact `realmctl` startup retry driver without absorbing the #38 session loop.

**Architecture:** `realm-core::ipc` remains the portable wire owner and gains a streaming-bounded encoder. `realm-control` splits deterministic framing/connection state from Linux descriptor I/O, then exposes the consumed-listener `ControlServer` and retained-capability `ClientEndpoint`. `realm-ctl` owns only the absolute not-before retry driver; #38 later supplies session semantics and the combined backend-first poll loop.

**Tech Stack:** Rust 1.85, serde/serde_json, safe rustix 1.1.4 Linux socket/event APIs, Cargo tests, trybuild, and a CI-only `socat` interoperability fixture.

**Spec:** `docs/specs/0007-control-socket-security.md` (Accepted; 2026-09-11 correction), aligned with `docs/specs/0003-realm-session.md`, `docs/specs/0006-realm-ctl.md`, `docs/adr/0004-ndjson-control-socket.md`, and `docs/INTERFACES.md`.

## Global Constraints

- Solve at the Accepted specification level first; commits `ab5d9ab` and `4ebf4f5` are the reviewed governing correction.
- Linux-only socket I/O stays in `realm-control`; `realm-core` remains portable and contains no fd, filesystem, environment, timer, or credential operations.
- A protocol frame is at most 65,536 bytes including its single LF; bounded encoding must stop before retaining a larger allocation.
- `ActiveControlListener` is consumed into `ControlServer` and has no public `AsFd` implementation.
- One `service_one` call handles one stable ready token and performs at most one accept, receive, or send syscall; due deadlines precede socket I/O and simultaneous subscriber readability precedes writability.
- Server calls use caller-supplied `std::time::Instant`; production has no clock or credential-injection trait.
- Production accept uses atomic NONBLOCK/CLOEXEC, then real same-euid `SO_PEERCRED` before any receive; every send uses `MSG_NOSIGNAL`.
- Hello is hard-bounded to one second; Ready partial input and client exchanges are hard-bounded to two seconds; ordinary/subscriber output uses a two-second no-progress deadline; terminal and total shutdown drain use hard 100 ms deadlines.
- The connection cap is 64. Admission, buffers, output, affected-id diagnostics, and retry attempts are all statically bounded.
- Transport emits decoded non-Hello requests but never implements their session meaning. #38 owns `Session::state()`, mutation completion, publication decisions, readiness, and the combined poll loop.
- Retry targets are absolute not-before offsets 0, 10, 30, 70, 150, and 310 ms from one driver start. Slow attempts skip elapsed sleep, never overlap, and never move backward.
- No security expansion, fuzzing framework, daemon assembly, KVM dependency, package build, or remote-control feature belongs in this plan.
- Every production behavior is introduced by a focused failing test whose failure is recorded before implementation.

---

## File Map

- `crates/realm-core/src/ipc.rs`: portable bounded NDJSON encoding and frame-size constant.
- `crates/realm-core/src/lib.rs`: distinguish encode overflow from serde decode/encode failures.
- `crates/realm-control/src/protocol.rs`: fd-free frame accumulator, connection phases, queues, deadlines, and transitions.
- `crates/realm-control/src/server.rs`: Linux listener/connection ownership, poll tokens, peer admission, one-I/O service quanta, completion/publication, and shutdown.
- `crates/realm-control/src/client.rs`: retained-capability one-attempt connect, mandatory Hello, request and subscription exchanges.
- `crates/realm-control/src/error.rs`: public server/client error and phase types.
- `crates/realm-control/src/runtime.rs`: client-specific descriptor-relative realm reopen preserving `MissingRealm`.
- `crates/realm-control/src/endpoint.rs`: consume `ActiveControlListener` into the server and remove public fd escape.
- `crates/realm-control/src/lib.rs`: export only the Accepted public transport boundary.
- `crates/realm-control/src/tests.rs`: deterministic protocol and Linux transport tests beside existing endpoint tests.
- `crates/realm-control/tests/ui/active_listener_fd_cannot_escape_server_boundary.rs`: compile-fail capability proof.
- `crates/realm-control/tests/control_socket_linux.rs`: real socket, half-close, backpressure, and CI-only `socat` tests.
- `crates/realm-ctl/src/retry.rs`: absolute not-before startup retry driver and exit classification.
- `crates/realm-ctl/src/main.rs`: include the retry module without adding an unresolved live command.
- `crates/realm-ctl/tests/control_retry.rs`: deterministic immediate/slow retry schedules.
- `.github/workflows/ci.yml`: install and execute only the `socat` acceptance fixture in its own bounded job.

---

### Task 1: Portable streaming-bounded protocol frames

**Files:**

- Modify: `crates/realm-core/src/ipc.rs`
- Modify: `crates/realm-core/src/lib.rs`

**Interfaces:**

- Produces: `pub const MAX_FRAME_BYTES: usize = 65_536`.
- Preserves: `pub fn encode<T: Serialize>(&T) -> realm_core::Result<String>`.
- Produces: `realm_core::Error::IpcFrameTooLarge { limit: usize }` for bounded-writer overflow.

- [ ] **Step 1: Write the failing boundary tests.**

Add tests proving that a string whose JSON representation is 65,535 bytes produces a 65,536-byte LF-terminated frame, while one additional payload byte returns `IpcFrameTooLarge`. Add a custom `Serialize` fixture that writes repeated chunks and records how many chunks serde requested; assert the writer errors as soon as the next chunk cannot fit so no oversized output is retained. The production regression each test catches is replacing the bounded writer with `serde_json::to_string` followed by a length check.

- [ ] **Step 2: Run RED.**

Run `cargo test -p realm-core ipc::tests::frame_encoder_accepts_exact_limit_and_rejects_next_byte -- --exact` and `cargo test -p realm-core ipc::tests::frame_encoder_stops_serialization_at_the_bound -- --exact`. Both must fail because `MAX_FRAME_BYTES`/`IpcFrameTooLarge` and bounded behavior do not exist.

- [ ] **Step 3: Implement the bounded writer.**

Use a private `FrameWriter { bytes: Vec<u8>, overflowed: bool }` implementing `std::io::Write`. Its `write` accepts a chunk only when `bytes.len() + chunk.len() <= MAX_FRAME_BYTES - 1`; otherwise it sets `overflowed` and returns `std::io::ErrorKind::WriteZero` without extending the vector. `encode` calls `serde_json::to_writer`, maps an error with `overflowed == true` to `IpcFrameTooLarge { limit: MAX_FRAME_BYTES }`, maps every other serde error to `Error::Ipc`, appends exactly one LF, and converts the known-UTF-8 JSON bytes to `String`.

- [ ] **Step 4: Run GREEN and regressions.**

Run both focused tests, then `cargo test -p realm-core` and `cargo fmt --all -- --check`. Output must be clean.

- [ ] **Step 5: Commit.**

Commit the reviewed spec commits, this plan, and the bounded encoder as `feat: bound control protocol frames`.

---

### Task 2: Deterministic fd-free connection state machine

**Files:**

- Create: `crates/realm-control/src/protocol.rs`
- Modify: `crates/realm-control/src/lib.rs`
- Modify: `crates/realm-control/Cargo.toml`
- Modify: `crates/realm-control/src/tests.rs`

**Interfaces:**

- Consumes: `realm_core::ipc::{Request, Response, Event, MAX_FRAME_BYTES, PROTOCOL_VERSION}` and `RealmState`.
- Produces internally: `ConnectionMachine`, `ConnectionPhase`, `InputBuffer`, `OutputCursor`, `MachineAction`, and `MachineClose`.
- The public server types remain in Task 3; no fd or poll type enters `protocol.rs`.

- [ ] **Step 1: Add the dependency and write RED state tests.**

Add workspace `realm-core` as the sole new production dependency of `realm-control`. Write table-driven tests with literal frames for matching/mismatched/non-Hello Hello, invalid UTF-8, invalid JSON, valid unknown Request, duplicate Hello, Subscribe, second ordinary pipeline, exactly-full unterminated input, and the mandatory Hello-plus-one buffered request. Each case asserts emitted `MachineAction`, exact queued bytes, phase, and whether later input is enabled. The mutation each test catches is a wrong transition or dispatch before the Hello reply drains.

- [ ] **Step 2: Run RED.**

Run `cargo test -p realm-control protocol_state_machine_is_total -- --exact` and `cargo test -p realm-control hello_reply_drains_before_retained_request_dispatch -- --exact`. They must fail on the absent module/types.

- [ ] **Step 3: Implement framing and handshake transitions.**

Implement `InputBuffer` with capacity `MAX_FRAME_BYTES`, scanning only for LF. Exactly `MAX_FRAME_BYTES` bytes without LF is terminal. One receive ingestion may decode Hello and retain one following complete or partial request; a third complete startup frame or later excess ordinary pipeline closes before emitting a request action. Matching Hello queues bounded `Response::Hello`, enters `SendingHello`, and releases the retained request only after the Hello cursor completes. Mismatch discards all pipeline and drains only the Hello refusal. Invalid bytes/JSON close silently; valid JSON outside `Request` queues bounded `Response::Error` and terminal drain.

- [ ] **Step 4: Write RED deadline/half-close/queue tests.**

Use hand-chosen `Instant` values to prove exact `now >= deadline`, non-sliding Ready partial input, two-second output reset only on positive progress, hard terminal 100 ms, half-close Hello plus ordinary response including application Error, half-close Subscribe, positive subscriber input, A/B/C coalescing before and after A starts, and every shutdown branch including the unstarted initial snapshot. Each test names the forbidden mutation: sliding a hard deadline, replacing initial A, queueing B as well as C, returning a half-closed peer to Ready, or moving the shutdown deadline.

- [ ] **Step 5: Run RED, then implement the minimum queue/deadline machine.**

Run the new focused tests before implementation. Add explicit absolute deadline fields, a read-half-closed bit, one current `OutputCursor`, and one replaceable latest event. `expire` performs no I/O. Shutdown preserves an unstarted initial State then queues Shutdown as latest, finishes any partial current alone, replaces only a later unstarted non-initial current, and is idempotent under its first hard deadline.

- [ ] **Step 6: Run GREEN and commit.**

Run all `realm-control` unit tests and `cargo fmt --all -- --check`. Commit as `feat: model bounded control connections`.

---

### Task 3: Linux ControlServer capability and one-I/O quanta

**Files:**

- Create: `crates/realm-control/src/server.rs`
- Modify: `crates/realm-control/src/endpoint.rs`
- Modify: `crates/realm-control/src/error.rs`
- Modify: `crates/realm-control/src/lib.rs`
- Modify: `crates/realm-control/src/tests.rs`
- Create: `crates/realm-control/tests/ui/active_listener_fd_cannot_escape_server_boundary.rs`
- Modify: `crates/realm-control/tests/compile_fail.rs`

**Interfaces:**

- Produces exactly the conceptual public types/signatures in `docs/INTERFACES.md`: `ControlServer`, ordered opaque `ConnectionId`, opaque `ControlToken`, `PollInterest`, `ReadyEvent`, `ControlAction`, `ControlError`, `ActiveControlListener::into_server`, and server methods through `is_shutdown_complete`.
- `PollInterest` borrows fds. `ReadyEvent` contains copied token/boolean readiness only.

- [ ] **Step 1: Write RED public-boundary and admission tests.**

Add the trybuild case that attempts `AsFd::as_fd(&listener)` and expects compilation failure. Add real Linux tests proving an accepted same-euid stream has NONBLOCK/CLOEXEC, and injected test-only credential failure/foreign uid closes before the injected receive counter changes. Add a compile-visible check that credential injection is unavailable outside `cfg(test)`. Run these tests and record their expected failures.

- [ ] **Step 2: Implement the consuming capability and real admission.**

Remove `impl AsFd for ActiveControlListener`. Move its owned listener/lock/path identity into `ControlServer`. On listener readiness, make exactly one `rustix::net::accept_with(..., SocketFlags::NONBLOCK | SocketFlags::CLOEXEC)` call. Immediately call safe `rustix::net::sockopt::socket_peercred`, compare `uid.as_raw()` to the retained euid, and only then allocate a stable monotonic `ConnectionId`/`ControlToken` and a `ConnectionMachine`. At 64 records, accept and close exactly one fd without allocating.

- [ ] **Step 3: Write RED arbitration, errno, and fd-reuse tests.**

Cover listener EINTR/EAGAIN/ECONNABORTED as end-of-quantum, resource exhaustion as fatal, all other listener errors as `ListenerIo`, connected EINTR/EAGAIN as no progress, reset/pipe as peer close, and other connected errors as `PeerIo`. With both subscriber readiness bits set, assert exactly one receive occurs and zero sends. At an exact deadline, assert close occurs with zero socket calls. Close/reuse the same numeric fd and prove stale token and stale `ConnectionId` cannot affect the new peer.

- [ ] **Step 4: Implement poll/service/completion behavior.**

Build poll interests from current machine state; pending application work and ordinary output disable reads, while subscribers may advertise both. `service_one` rejects stale readiness, expires the selected connection first, then applies read-before-write for subscribers and the sole enabled operation elsewhere. Receive uses one fixed 65,536-byte buffer and one `recv`; send uses one `send(..., SendFlags::NOSIGNAL)`. `complete_request` and `complete_subscribe` bounded-encode before queue mutation. `publish_state` encodes once; on overflow it closes all and only current subscribers and returns their at-most-64 ids sorted ascending. After shutdown begins, completion/publication returns `ShuttingDown` without mutation.

- [ ] **Step 5: Write RED shutdown and bounded-work tests, then GREEN.**

Prove listener interest vanishes, all non-subscribers close, no later action appears, repeated shutdown does not move the deadline, subscriber drain completes early or at exact expiry, and every ready-token case makes at most one socket I/O call. Implement only the missing behavior, then run all `realm-control` tests, trybuild, fmt, and clippy for the crate.

- [ ] **Step 6: Commit.**

Commit as `feat: serve bounded Linux control connections`.

---

### Task 4: Retained-capability ClientEndpoint and bounded Client

**Files:**

- Create: `crates/realm-control/src/client.rs`
- Modify: `crates/realm-control/src/runtime.rs`
- Modify: `crates/realm-control/src/error.rs`
- Modify: `crates/realm-control/src/lib.rs`
- Modify: `crates/realm-control/src/tests.rs`

**Interfaces:**

- Produces: `RuntimeDir::client_endpoint(self) -> ClientEndpoint`.
- Produces: `ClientEndpoint::connect(&self, client: &str) -> Result<Client, ClientError>` as one attempt only.
- Produces: `Client::request`, consuming `Client::subscribe`, and `Subscription: Iterator<Item = Result<Event, ClientError>>` exactly as `docs/INTERFACES.md` defines.

- [ ] **Step 1: Write RED retained-capability/error tests.**

Prove a missing realm is `ClientError::MissingRealm` and creates nothing; environment/path replacement after endpoint construction is ignored; every retry-style repeated call reopens realm relative to the same retained runtime fd; unsafe realm is `Path(UnsafeRealmDirectory)`; and only MissingRealm/Refused report retryable. Cover every `ClientPhase` and both versions in `VersionMismatch` with direct enum assertions.

- [ ] **Step 2: Run RED and implement client-specific realm reopen.**

Run the focused tests. Add a client reopen path that maps only descriptor-relative NOENT/NOTDIR to `MissingRealm`, retains all secure resolution/property checks, forms only the internal procfd address, and never calls mkdir or rereads the environment.

- [ ] **Step 3: Write RED single-attempt connect/Hello tests.**

Using real Unix sockets plus deterministic operation seams under `cfg(test)`, cover immediate success, EINPROGRESS/Unix EAGAIN followed by every `SO_ERROR`, hard 100 ms connect timeout, terminal connect EINTR/EALREADY, exactly one connect syscall, Hello client name, shared hard two-second Hello write/read deadline, version refusal carrying both versions, oversized/malformed/unexpected/EOF responses, and `MSG_NOSIGNAL`.

- [ ] **Step 4: Implement connect and bounded exchanges.**

Create one fresh NONBLOCK/CLOEXEC stream per call. For in-progress connect, poll writable only until the absolute deadline and classify final `SO_ERROR`. Perform Hello inside one absolute two-second deadline. Client readiness waits recompute remaining time after EINTR without sliding. `request` rejects Hello/Subscribe locally, and shares one new hard two-second send/response deadline. `subscribe` shares one hard two-second send/initial-State deadline; the returned iterator has unbounded empty idle but a hard two-second partial-event deadline and yields Shutdown once.

- [ ] **Step 5: Run GREEN and commit.**

Run the focused client tests, all `realm-control` tests, fmt, and crate clippy. Commit as `feat: add bounded control client`.

---

### Task 5: Exact realmctl startup retry driver

**Files:**

- Create: `crates/realm-ctl/src/retry.rs`
- Modify: `crates/realm-ctl/src/main.rs`
- Create: `crates/realm-ctl/tests/control_retry.rs`

**Interfaces:**

- Produces an internal driver accepting one fixed start `Instant`, a `now` function, sleeper, and attempt closure returning `Result<Client, ClientError>`.
- Produces a small result classifier mapping exhausted MissingRealm/Refused to 3, VersionMismatch to 4, and every other transport error to 6.
- Does not add a placeholder CLI verb; the first real control command will call this driver after the SPEC 0006 wire revision.

- [ ] **Step 1: Write RED schedule tests.**

With fake time and literal observed timestamps, prove immediate retryable outcomes call attempts at 0, 10, 30, 70, 150, and 310 ms, make exactly five sleeps, and never sleep after attempt six. Prove an attempt that advances fake time from 0 to 90 ms causes attempt two immediately at 90, then attempts at 90, 90, 150, and 310 ms without overlap or negative sleep. Prove a terminal error stops on the first attempt.

- [ ] **Step 2: Run RED and implement absolute not-before scheduling.**

Store `[0, 10, 30, 70, 150, 310]` as millisecond offsets from one captured start. Before attempts after the first, sleep only when `now < start + target`. Invoke attempts sequentially. Continue only for `MissingRealm` or `Refused`; return the last retryable error after attempt six.

- [ ] **Step 3: Write RED exit-class tests and implement the classifier.**

Assert literal exit codes 3, 4, and 6 for exhaustion, version mismatch, and all other client failures. Keep Doctor's future no-session policy out of this driver and do not alter theme commands.

- [ ] **Step 4: Run GREEN and commit.**

Run `cargo test -p realm-ctl`, existing theme CLI tests, fmt, and crate clippy. Commit as `feat: bound realmctl startup retry`.

---

### Task 6: Real Linux interoperability and CI evidence

**Files:**

- Create: `crates/realm-control/tests/control_socket_linux.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/specs/0007-control-socket-security.md`

**Interfaces:**

- Exercises the public server/client boundary and a tiny test-owned application responder; it does not introduce session semantics into production.
- CI installs `socat` only for this functional test. No local packaging toolchain or daemon is installed.

- [ ] **Step 1: Write RED real-socket tests.**

Test ordered matching Hello plus GetState with write-half EOF, mismatch plus pipelined request producing only server Hello, matching Hello plus Subscribe with clean read-half EOF, full send-buffer EAGAIN returning from one quantum, one healthy peer beside a stalled partial-frame peer, and actual same-euid admission. Every helper thread and child has an absolute three-second test deadline and kill/join cleanup.

- [ ] **Step 2: Run RED, make only integration fixes, then GREEN.**

Run each focused integration test before fixing exposed code. Do not weaken assertions or add sleeps to the transport. After fixes, run the complete `realm-control` integration file and crate suite.

- [ ] **Step 3: Add the CI-only socat acceptance.**

Add an ignored test named `shell_two_frame_interoperability_is_ordered` that launches `socat`, writes the two literal LF-terminated ADR frames, half-closes input, and asserts decoded Hello then application response. Add a dedicated Ubuntu job that installs `socat` with apt and runs exactly that ignored test under the test's own three-second child deadline. Do not install `socat` locally as part of the plan.

- [ ] **Step 4: Update evidence columns only after tests pass.**

Fill SPEC 0007 A13-A16d/A17 transport test cells with the exact passing test paths. Leave #38/#40/#65 cells explicitly pending; do not claim the combined-loop or real key-to-`manage_finish` evidence.

- [ ] **Step 5: Run final local gates.**

Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`, `cargo test --workspace --all-features --locked`, `RUSTUP_TOOLCHAIN=1.85 cargo build --workspace --all-features --locked`, `./docs/check-readme-truth-snapshot.sh`, `scripts/check-project-namespace`, `git diff --check`, and the task brief's exact retired-name scan. The namespace scan must return no matches.

- [ ] **Step 6: Commit.**

Commit as `test: prove control transport interoperability`.

---

## Plan Self-Review

- Spec coverage: Tasks 1-6 cover A2 client half, A10, A13-A16d, A16e's transport half, and A17's bounded transport half. A12 and the remaining A16e/A17 integration stay with #38/#40/#65 exactly as Accepted.
- Placeholder scan: no TBD, TODO, generic error-handling instruction, or delegated unnamed test remains.
- Type consistency: bounded `ipc::encode` feeds both server and client; `ConnectionId` is ordered for the bounded affected-id vector; `ReadyEvent` never borrows; `ClientEndpoint` is single-attempt; retry ownership stays in `realmctl`.
- Scope ruling: the production retry driver lands before its first caller because Accepted SPEC 0007 assigns the startup-race behavior to #41, but no fake command is added to make it appear user-visible before SPEC 0006's protocol revision.
