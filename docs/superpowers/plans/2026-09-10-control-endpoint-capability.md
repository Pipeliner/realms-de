# Control endpoint capability implementation plan

Date: 2026-09-10
Issue: #218
Target maturity: candidate
Governing specifications: Accepted SPEC 0007 and Accepted SPEC 0003

## Outcome

Implement the smallest Linux-only shared capability needed to reserve Realm's
fixed local control endpoint before recovery and activate it exactly once after
recovery. This slice owns runtime and realm directory validation, the fixed
ctl.sock pathname, singleton ownership, conservative stale reclaim, a
bound-but-not-listening capability, and consuming activation.

It does not implement ClientEndpoint, client retry, peer credentials, Hello or
framing, request dispatch, subscriptions, persistence, session-loop assembly,
or readiness wiring. Those remain #41 and #38.

## Non-negotiable design constraints

- Follow the repository order: Accepted spec, failing behavior test, minimal
  implementation, verification.
- Keep realm-core portable. Filesystem and socket work belongs in the new
  realm-control crate. realm-session and realm-ctl may depend on it.
- Put crate-wide forbid(unsafe_code) in realm-control/src/lib.rs. Use no direct
  libc, FFI, raw-pointer, from_raw_fd, or borrowed-raw-fd construction.
- Use locked rustix 1.1.4 safe APIs. socket_error and socket_acceptconn come
  from rustix::net::sockopt; they are not re-exported from rustix::net.
- Production reads only XDG_RUNTIME_DIR. There is no /tmp fallback,
  REALM_SOCKET override, or caller-selected production socket path.
- Validate runtime and realm directories by retained descriptors: directory,
  daemon euid, exact mode 0700, and no symlink or magic-link traversal except
  for the internal procfd bridge.
- Before creating realm, traverse the actual held runtime descriptor through
  /proc/self/fd/<fd>, compare its directory identity and properties to the
  retained descriptor, and preflight canonical and worst-case procfd address
  lengths including the terminating NUL.
- Create realm under scoped umask 0077. Bind ctl.sock under scoped umask 0177.
  Preparation through verified bind is a documented single-threaded startup
  operation because umask is process-global.
- Acquire an independent CLOEXEC directory fd and nonblocking exclusive flock
  before any ctl.sock stat, probe, unlink, or bind.
- Reclaim only an owned exact-mode socket for which a one-shot nonblocking
  connect proves ECONNREFUSED and whose identity is unchanged at the final
  recheck. Every other result preserves the entry and returns EndpointInUse.
- The accepted same-euid trust boundary applies to unavoidable bind-to-stat
  and stat-to-unlink gaps. Never claim Linux provides atomic compare-and-unlink
  or that socket-fd fstat identifies the pathname inode.
- Create sockets with NONBLOCK and CLOEXEC atomically. Verify pathname type,
  owner and exact 0600 mode; exact getsockname procfd address; and false
  SO_ACCEPTCONN before returning the bound capability.
- A post-bind stat syscall failure preserves errno as IpcPathError::Io. Only a
  successfully read pathname with bad properties is UnsafeSocketEntry.
- BoundControlEndpoint exposes no fd and no Clone. activate consumes self,
  calls listen with backlog 64 exactly once, verifies SO_ACCEPTCONN, and returns
  ActiveControlListener. Only the active wrapper implements AsFd.
- Ownership cleanup closes the socket first, then under the still-held lock
  stats and unlinks only an identity that matches. Cleanup is best effort.

## Task 1: Establish the crate and validated runtime capability

Files:

- Cargo.toml and Cargo.lock
- crates/realm-control/Cargo.toml
- crates/realm-control/src/lib.rs
- crates/realm-control/src/error.rs
- crates/realm-control/src/runtime.rs
- crates/realm-control/src/tests.rs
- crates/realm-core/src/ipc.rs
- crates/realm-session/Cargo.toml
- crates/realm-ctl/Cargo.toml

### RED

Add realm-control to the workspace and as a dependency of realm-session and
realm-ctl. Add rustix event to the existing workspace feature list. The new
crate is Linux-only with an explicit non-Linux compile_error and exports:

- IpcPathError
- RuntimeDirResolver
- production_runtime_dir
- test_runtime_dir
- RuntimeDir
- RealmDir

Write and run these failing tests before implementing the behavior:

- runtime_capability_rejects_every_unsafe_input_and_openat2_failure
- production_runtime_reads_only_xdg_runtime_dir
- realm_directory_validation_is_descriptor_relative_and_exact

The tests must cover missing, relative, absent, non-directory, symlinked,
foreign-owner via a validation seam, and modes 0000, 0600, 0701, 0710, 0770,
and 0777. Inject an opener failure and prove its errno is retained. Expectations
must be literal and independent of production helpers.

Run each focused test and record the compile or assertion failure caused by the
missing production behavior.

### GREEN

Implement IpcPathError with MissingRuntimeDir, UnsafeRuntimeDir,
UnsafeRealmDirectory, UnsafeSocketEntry, EndpointInUse, and Io(std::io::Error).
Map rustix Errno to Io without losing raw_os_error.

RuntimeDir retains the absolute display path, OwnedFd, euid, and directory
device/inode. Open with openat2(CWD, path, RDONLY | DIRECTORY | CLOEXEC,
empty mode, NO_SYMLINKS | NO_MAGICLINKS). Map absent/not-directory to
MissingRuntimeDir, symlink/access/policy rejection to UnsafeRuntimeDir, retain
other OS errors, then fstat and require the exact properties.

RealmDir retains its separately opened descriptor, device/inode, and euid and
publicly exposes only as_fd. Its open is relative to RuntimeDir with the same
openat2 resolution restrictions and exact property validation.

Remove realm_core::ipc::socket_path and its tests/imports. Do not add any
filesystem dependency to realm-core.

Run focused tests, realm-core tests, formatting, and cargo tree -p realm-core.
Commit the accepted spec correction and this plan with the first completed
slice so the implementation history contains its governing design.

## Task 2: Prepare the fixed realm endpoint and prove procfd usability

Files:

- crates/realm-control/src/sys.rs
- crates/realm-control/src/endpoint.rs
- crates/realm-control/src/runtime.rs
- crates/realm-control/src/lib.rs
- crates/realm-control/src/tests.rs

### RED

Write and run these failing tests first:

- server_creates_realm_exactly_once_under_scoped_umask
- actual_runtime_procfd_bridge_is_verified_before_mutation
- missing_inaccessible_or_mismatched_procfd_fails_before_mutation
- sockaddr_un_overflow_fails_before_mutation_without_fallback

The bridge tests use an injected bridge opener/validator. They must prove that
opening /proc/self/fd alone is insufficient: the implementation traverses the
actual retained runtime fd, compares device/inode/type/euid/mode, and returns
before creating realm or touching an existing ctl.sock on any failure.

The umask test serializes process-global mutation and proves restoration after
success, error, and a caught unwind. It proves an existing realm is neither
chmodded nor replaced.

### GREEN

Add a ScopedUmask RAII guard and fixed constants realm and ctl.sock. Add helpers
for /proc/self/fd/<fd> and /proc/self/fd/<fd>/ctl.sock. Reject NUL and any byte
length plus terminating NUL above Linux sun_path capacity 108 before mutation.

Preflight order in RuntimeDir::prepare_server_endpoint(self):

1. Open the actual runtime procfd bridge using the retained raw fd rendered by
   trusted internal code.
2. Compare its fstat identity and exact directory properties with RuntimeDir.
3. Check canonical display and worst-case procfd socket address lengths.
4. mkdirat realm under scoped umask 0077, accepting only success or EEXIST.
5. Securely reopen and validate realm relative to RuntimeDir.
6. Construct the exact procfd SocketAddrUnix for that RealmDir.

Return SocketEndpoint retaining canonical display path, RealmDir, and private
bind address. Public accessors are path and realm_dir only.

Run focused tests and formatting. Commit only when GREEN.

## Task 3: Lock ownership and conservative stale reclaim

Files:

- crates/realm-control/src/endpoint.rs
- crates/realm-control/src/tests.rs

### RED

Write and run these failing tests first:

- linux_stale_probe_completion_table_is_total
- singleton_lock_precedes_socket_inspection
- unsafe_realm_and_socket_entries_are_preserved
- live_listener_is_preserved_and_verified_refusal_is_reclaimed
- stale_reclaim_rechecks_identity_before_unlink

Use decision functions for deterministic table coverage. Immediate ECONNREFUSED
is Stale; EAGAIN and EINPROGRESS require Poll; success, EALREADY, and every
other result Preserve. After poll, only readiness followed by SO_ERROR equal to
ECONNREFUSED is Stale. Timeout, poll failure, successful completion, and every
other SO_ERROR Preserve.

Race hooks may replace the pathname before the final identity recheck. Prove a
detected replacement remains. Do not claim or test atomic protection after the
final stat; the Accepted spec records the same-euid trust boundary.

### GREEN

EndpointLock independently opens RealmDir dot with RDONLY | DIRECTORY |
CLOEXEC, verifies the same identity/properties and FD_CLOEXEC, then calls flock
NonBlockingLockExclusive. Contention maps to EndpointInUse; other errno remains
Io. No public fd accessor exists.

Only after the lock is held, stat ctl.sock without following symlinks. Require
socket type, current euid, and exact mode 0600. Probe through a fresh Unix stream
socket created atomically NONBLOCK | CLOEXEC. Import socket_error from
rustix::net::sockopt. Poll POLLOUT for at most 100 ms when required and inspect
SO_ERROR once. Re-stat all identity/properties immediately before unlinkat.

Run focused tests and formatting. Commit only when GREEN.

## Task 4: Bind and retain the non-listening capability

Files:

- crates/realm-control/src/endpoint.rs
- crates/realm-control/src/lib.rs
- crates/realm-control/src/tests.rs

### RED

Write and run these failing tests first:

- singleton_lock_protects_the_prelisten_state
- bound_capability_has_exact_path_fd_and_address_properties
- post_bind_bad_properties_preserve_detected_replacement
- post_bind_stat_error_preserves_errno_and_pathname
- bind_scoped_umask_restores_after_success_and_error
- post_bind_verification_failures_use_ownership_safe_cleanup
- bound_drop_closes_and_removes_only_matching_identity
- singleton_lock_fd_is_private_directory_cloexec_and_not_inherited_across_exec

Inject the post-bind pathname stat operation so EIO can be asserted as Io(EIO)
without replacing OS behavior globally. The property mismatch test uses a
non-socket replacement visible before validation and proves it remains.

Run bind under a hostile ambient umask and prove restoration after successful
bind and an injected bind error. Together with Task 2's caught-unwind test of
the same ScopedUmask guard, this covers success, error, and unwind restoration
before the process may become multithreaded.

Inject the verification operations after ownership is constructed. Exercise a
getsockname error and SO_ACCEPTCONN query error against an unchanged pathname
and require identity-checked removal. In a second case, replace the pathname
before returning the injected verification error and require cleanup to
preserve the detected replacement.

For the exec test, launch the current test executable with --ignored, --exact,
and --nocapture. Read the readiness line on a helper thread and use a bounded
channel receive. On timeout kill and wait for the child before failing, so CI
cannot hang. Once ready, drop the owner while the exec child remains, prove a
new binder acquires the lock, then close child stdin. Bound the final exit wait
too; on timeout kill and reap the child before failing. Neither readiness nor
shutdown may contain an unbounded read or wait.

### GREEN

SocketEndpoint::bind consumes self. It acquires EndpointLock, performs stale
inspection/reclaim, creates a Unix stream socket atomically NONBLOCK | CLOEXEC,
and binds the exact private address under scoped umask 0177.

After bind, injected/default no-follow stat errors preserve raw errno as Io.
Successfully read bad properties return UnsafeSocketEntry. Retain the pathname
identity only after successful validation. Construct ownership before later
getsockname/SO_ACCEPTCONN verification so failures use identity-checked Drop.

Verify fd NONBLOCK and CLOEXEC, exact SocketAddrUnix conversion of getsockname,
and false socket_acceptconn imported from rustix::net::sockopt. Never compare
pathname identity with fstat(socket_fd).

EndpointOwnership field/lifetime behavior must close the socket before cleanup,
retain the independent lock through cleanup, and unlink only a currently
matching identity. BoundControlEndpoint publicly exposes endpoint and realm_dir
borrows but no fd/AsFd/Clone.

Consolidate module imports when extending endpoint.rs; do not paste duplicate
OFlags, SocketAddrUnix, socket_with, AddressFamily, SocketFlags, or SocketType
imports.

Run focused tests and formatting. Real Unix socket tests that receive sandbox
EPERM must be rerun outside the sandbox; record the distinction. Commit GREEN.

## Task 5: Consuming one-shot activation

Files:

- crates/realm-control/src/endpoint.rs
- crates/realm-control/src/lib.rs
- crates/realm-control/src/tests.rs
- crates/realm-control/tests/compile_fail.rs
- crates/realm-control/tests/ui/bound_endpoint_cannot_activate_twice.rs
- crates/realm-control/tests/ui/bound_endpoint_cannot_activate_twice.stderr

### RED

Before implementing activate, add a unit type assertion assigning the method to
this exact function-pointer shape:

    fn(BoundControlEndpoint) -> Result<ActiveControlListener, IpcPathError>

Add behavior tests:

- activation_signature_consumes_bound_capability
- activation_is_consuming_one_shot_and_listens_with_backlog_64
- activation_failure_uses_identity_checked_cleanup
- active_drop_preserves_detected_replacement

Run them and record RED caused by the absent consuming API.

### GREEN

Add ActiveControlListener. BoundControlEndpoint::activate(self) moves ownership,
calls listen on the private fd with literal backlog 64, verifies
socket_acceptconn is true, and returns the active wrapper. Activation failure
uses the existing owned cleanup. Add an internal activation-operation seam so
tests can make listen or the post-listen SO_ACCEPTCONN query fail after owned
identity exists; prove the unchanged pathname is removed and a detected
replacement is preserved. Active exposes endpoint and realm_dir and is the
only public wrapper implementing AsFd.

After GREEN, add trybuild as a dev dependency and the named UI regression that
attempts to call activate twice. First capture compiler-owned stderr, inspect it
for E0382 and the ownership note, commit the exact fixture, then rerun without
overwrite. The preceding function-pointer test is the primary TDD evidence;
the UI fixture is the durable negative-compile regression.

Run focused activation, compile-fail, formatting, and clippy checks. Commit.

## Task 6: Full #218 verification and handoff

Run from a clean branch head:

    cargo test -p realm-control
    cargo test -p realm-core
    cargo test --workspace --all-targets --locked
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo fmt --all -- --check
    cargo tree -p realm-core
    cargo tree -p realm-control
    git diff --check

Audit with rg that realm-control contains no unsafe block, libc/FFI, client
retry, ClientEndpoint, Hello/framing, peer credentials, sd_notify, readiness,
or daemon-loop implementation. Confirm realm-core has no rustix or
realm-control dependency, Bound has no AsFd/Clone, Active does implement AsFd,
and the only listen call is the consuming transition with backlog 64.

Evidence matrix for #218:

| SPEC row | Evidence in this slice |
| --- | --- |
| A1 | runtime_capability_rejects_every_unsafe_input_and_openat2_failure |
| A2 server half | server_creates_realm_exactly_once_under_scoped_umask |
| A2a | actual_runtime_procfd_bridge_is_verified_before_mutation; sockaddr_un_overflow_fails_before_mutation_without_fallback |
| A3 | unsafe_realm_and_socket_entries_are_preserved; post_bind_bad_properties_preserve_detected_replacement |
| A4 | live_listener_is_preserved_and_verified_refusal_is_reclaimed |
| A5 | linux_stale_probe_completion_table_is_total |
| A6 | singleton_lock_protects_the_prelisten_state |
| A6a | singleton_lock_fd_is_private_directory_cloexec_and_not_inherited_across_exec |
| A7 | bound_capability_has_exact_path_fd_and_address_properties; post_bind_stat_error_preserves_errno_and_pathname; bind_scoped_umask_restores_after_success_and_error; post_bind_verification_failures_use_ownership_safe_cleanup |
| A8 | activation_is_consuming_one_shot_and_listens_with_backlog_64 |
| A9 | bound_drop_closes_and_removes_only_matching_identity; activation_failure_uses_identity_checked_cleanup; active_drop_preserves_detected_replacement |
| A11 | activation_signature_consumes_bound_capability; compile_fail::bound_endpoint_cannot_activate_twice |

A10 is #41. A12 is #38. A13-A17 are #41.

Request adversarial code review against the Accepted spec and this matrix. Fix
all Critical and Important findings, rerun the full commands, commit corrections,
push the branch, open a PR through scripts/gh-body-file, monitor all CI, repair
failures, merge only on green, close #218, and reprioritize the next MVP issue.
