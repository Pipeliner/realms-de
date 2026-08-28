# ADR 0004 — Newline-delimited JSON over a unix socket

- **Status:** Accepted (2026-08-26; IPC security amendment accepted 2026-08-28)
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

ADR 0003 puts one daemon in charge of state. It needs a wire protocol with four
properties, in this order of importance:

1. **Scriptable by hand.** helm is a keyboard-first, power-user desktop. A user
   who wants to bind something we did not think of should be able to do it from
   a shell script, today, without a library. `echo '{"cmd":"switch-orbit",
   "arg":3}' | socat - $XDG_RUNTIME_DIR/helm/ctl.sock` is the bar we are aiming
   at.
2. **Unmisreadable framing.** A half-written frame must never parse as a
   complete one. `docs/PITFALLS.md` lists version skew between components as a
   packaging pitfall; a protocol that silently misinterprets a truncated or
   older frame turns that into data corruption.
3. **Cheap.** The bar's redraw budget is 8 ms end to end. Serialisation must not
   be a meaningful fraction of that.
4. **Available before the desktop is.** The session starts, then everything
   else. The protocol must not depend on something the session itself has to
   start first.

## Decision

One newline-delimited JSON stream over a `SOCK_STREAM` unix socket.

### Endpoint and admission

- The only production endpoint is `$XDG_RUNTIME_DIR/helm/ctl.sock`.
  `XDG_RUNTIME_DIR` must be an absolute existing directory; if it is absent,
  relative, or not a directory, path resolution fails with
  `IpcPathError::MissingRuntimeDir` and neither a client nor the daemon falls
  back to `/tmp`. `helm-wm` exits non-zero before binding; `helmctl` reports the
  same condition as "not inside a helm session" and does not connect anywhere.
- `HELM_SOCKET` is not a production override and `helmctl --socket` is not a
  public option. The only test seam is a non-production path resolver which
  receives an explicit temporary runtime directory. It must produce the same
  `helm/ctl.sock` descendant and perform the same ownership/type checks as the
  production resolver; it must not accept an arbitrary socket pathname. This
  seam exists solely to make isolated integration fixtures possible without
  mutating process environment.
- The daemon creates the `helm` child directory mode `0700` and the listener
  mode `0600`. It rejects a pre-existing non-directory `helm` path and a
  pre-existing non-socket `ctl.sock` without following links. A stale socket
  owned by the daemon's effective uid may be unlinked only after a connection
  attempt proves that no listener answers.
- Before reading any frame, the listener obtains Linux `SO_PEERCRED`. It admits
  only peers whose uid equals the daemon's effective uid. A missing credential,
  credential lookup failure, or different uid closes that connection without a
  protocol reply and without delaying healthy peers. Directory permissions are
  defence in depth, not a substitute for peer admission.

### Connection protocol and liveness

- `Request::Hello { version, client }` is the mandatory first complete frame on
  **every** connection, including one-shot shell clients. The server answers
  `Response::Hello` before any other response. A matching version moves the
  connection to ready; a mismatch receives the server version and is then
  closed. A request before `Hello`, a second `Hello`, or `Subscribe` before a
  successful matching Hello is refused with `Response::Error` and then closed.
  Shell scriptability remains: a script writes a Hello frame followed by its
  request, each newline terminated.
- A ready client may issue ordinary requests. `Subscribe` is a terminal request:
  after a successful Hello it changes the connection into a subscriber, emits
  an immediate `Event::State` snapshot, and accepts no further client frames.
  EOF or a forbidden subsequent frame removes only that subscriber.
- The listener admits at most **64** live connections. It reads at most
  **64 KiB** for a complete NDJSON frame and requires the initial Hello within
  **one second** of `accept(2)`. It permits at most **one** queued complete
  inbound request beyond the first Hello and one queued complete ordinary
  response per connection.
  A connection that exceeds a bound or misses the Hello deadline is closed;
  accepting the 65th connection must not evict or delay an admitted peer.
- Subscriber output remains a one-frame latest-state slot plus the current
  partial write. Writes are non-blocking. A cursor that has not advanced for
  `SUBSCRIBER_STALL_LIMIT` (**2 seconds**) is closed; `Event::Shutdown` has a
  bounded best-effort write and never delays process exit. These limits are
  liveness guarantees: a local peer cannot turn control-socket I/O into a
  window-management stall.

- One JSON value per line. `ipc::encode` appends the newline and
  `ipc::tests::requests_round_trip_through_a_frame` asserts every frame contains
  exactly one.
- `Request`, `Response` and `Event` are adjacently-tagged serde enums
  (`tag = "cmd", content = "arg"` and so on), which keeps frames readable and
  keeps the tag stable when a variant gains fields.
- `PROTOCOL_VERSION` (currently 1) is exchanged by `Request::Hello` /
  `Response::Hello` before any operation. The session always answers Hello,
  including on mismatch, so the client can print a useful error. A mismatched
  client refuses to proceed rather than guessing at fields.
- Unknown frames are an error, never a panic
  (`ipc::tests::unknown_frames_are_an_error_not_a_panic`).

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **D-Bus** | The correct desktop citizenship answer, and it is not close. We already require a session bus for portals (ADR 0011), so it is running regardless. Free service activation, free signal broadcast to multiple subscribers, introspection, `busctl` and `gdbus` as ready-made debugging tools, and other desktop components could integrate with helm without us shipping a library | Scripting D-Bus is meaningfully worse than scripting a socket: `busctl call` needs a signature string and gets the marshalling wrong in ways that are hard to diagnose. It adds a dependency (`zbus`) to every client including the bar. And it inverts a startup ordering we would rather control: the session would need the bus healthy before it could serve, when in fact the session is what puts `WAYLAND_DISPLAY` *into* the bus environment. This was the closest call in this ADR, and D-Bus remains the right answer if helm ever needs third-party integrations |
| **A binary codec** (bincode, postcard, CBOR) | Smaller frames, faster encode/decode, self-delimiting length prefixes | Solves a problem we do not have. `HelmState` is a few hundred bytes and serialises in microseconds against an 8 ms budget. It costs the entire scriptability property, which is requirement 1 |
| **A Wayland protocol extension** | The most "correct" place for compositor state; no second socket; automatic lifetime tied to the display | Only reachable from a Wayland client, so `helm ctl` from a TTY or an SSH session could not use it. Requires the compositor to serve it, which couples clients to the compositor and undoes ADR 0002 and ADR 0003. Extension design and codegen is real work for no user-visible gain |
| **HTTP on a unix socket** | Every language has a client; trivially introspectable with `curl` | Request/response only. Push subscription needs SSE or websockets bolted on, and we would have written a worse version of what NDJSON already does |

## Consequences

### Good

- `socat`, `nc -U`, `jq` and a shell are a complete client: write the mandatory
  Hello frame first. `helmctl` is a thin wrapper rather than a privileged path.
- Framing is unambiguous by inspection: if there is no newline yet, the frame is
  not complete. No length prefix to get wrong, no state machine.
- Serde does the work. Adding a `Request` variant is one enum arm and the tests
  cover it.
- The fixed per-user runtime endpoint plus same-uid credential admission avoids
  trusting a world-visible fallback directory or caller-controlled endpoint.
- Version skew fails loudly at Hello instead of quietly at field access.

### Bad

- JSON is not free. Every state push allocates and formats. It is well inside
  budget at current sizes, but it puts a ceiling on how large `HelmState` may
  grow, and nothing enforces that ceiling today.
- No service activation. Something must start the session before a client
  connects, and clients need a connect-retry loop for the startup race.
- We are not a D-Bus citizen, so a third-party panel or a desktop integration
  cannot talk to helm without implementing our protocol.
- A JSON value containing a literal newline inside a string is escaped by serde,
  so framing is safe — but only because we always encode through `ipc::encode`.
  A hand-rolled writer elsewhere could break it.

### Neutral

- One socket serves both request/response and subscription. A subscriber's
  connection simply stays open and receives `Event` frames interleaved with
  nothing else, because subscribers do not issue further requests.

## Reversal

Low. The wire format is confined to `helm-core::ipc` plus one transport module
per client. Adding a D-Bus surface *alongside* the socket, rather than instead
of it, is the likely path and would be additive: a D-Bus object that proxies the
same `Request`/`Response` types. Estimated a week for the proxy, no changes to
existing clients.

The signal to reconsider is a concrete integration request from outside helm —
someone wanting to drive orbits from an existing panel or a global hotkey daemon
— or evidence that startup ordering would be simpler with bus activation.

## Guard

- `ipc::tests::requests_round_trip_through_a_frame` — asserts single-line
  framing and exact round-trip for a representative set of variants.
- `ipc::tests::responses_round_trip`.
- `ipc::tests::unknown_frames_are_an_error_not_a_panic` — an unrecognised
  command or malformed input must be a decode error.
- *Required before M2 implementation:* `ipc::tests::socket_path_requires_xdg_runtime_dir`,
  `ipc::tests::test_runtime_dir_resolver_preserves_the_fixed_descendant`, and
  `ipc::tests::production_path_ignores_helm_socket`.
- *Required before M2 implementation:* `control_socket::tests::rejects_foreign_uid_before_read`,
  `control_socket::tests::rejects_missing_peer_credentials_before_read`,
  `control_socket::tests::refuses_symlink_or_wrong_type_at_runtime_path`, and
  `control_socket::tests::stale_same_uid_socket_is_reclaimed_only_after_connect_fails`.
- *Planned (M2):* a CI job that drives a live session end to end with `socat`
  and `jq` only, so the scriptability claim is tested rather than asserted.
- *Required before M2 implementation:* `control_socket::tests::hello_is_required_before_every_request`,
  `control_socket::tests::version_mismatch_gets_hello_then_eof`,
  `control_socket::tests::subscribe_requires_matching_hello_and_emits_snapshot`,
  `control_socket::tests::connection_limit_rejects_without_harming_admitted_peers`,
  `control_socket::tests::oversized_or_unterminated_frame_is_closed`,
  `control_socket::tests::hello_deadline_closes_idle_peer`, and
  `control_socket::tests::stalled_subscriber_is_evicted_without_missing_key_budget`.
