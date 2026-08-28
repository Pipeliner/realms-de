# ADR 0004 — Newline-delimited JSON over a unix socket

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
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

- Current M0 path helper: `HELM_SOCKET`, when set, is the complete override
  path. Otherwise it returns `$XDG_RUNTIME_DIR/helm/ctl.sock`, or the per-uid
  `/tmp/helm-$uid/helm/ctl.sock` fallback when `XDG_RUNTIME_DIR` is absent.
  This is the implemented, tested `ipc::socket_path()` contract; it does not
  create or secure a listening socket.
- One JSON value per line. `ipc::encode` appends the newline and
  `ipc::tests::requests_round_trip_through_a_frame` asserts every frame contains
  exactly one.
- `Request`, `Response` and `Event` are adjacently-tagged serde enums
  (`tag = "cmd", content = "arg"` and so on), which keeps frames readable and
  keeps the tag stable when a variant gains fields.
- `PROTOCOL_VERSION` (currently 1) is exchanged by `Request::Hello` /
  `Response::Hello`. The session always answers Hello, including on mismatch,
  so the client can print a useful error. A mismatched client refuses to proceed
  rather than guessing at fields.
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

- `socat`, `nc -U`, `jq` and a shell are a complete client. So is `helm ctl`,
  which is a thin wrapper rather than a privileged path.
- Framing is unambiguous by inspection: if there is no newline yet, the frame is
  not complete. No length prefix to get wrong, no state machine.
- Serde does the work. Adding a `Request` variant is one enum arm and the tests
  cover it.
- The M0 helper is transport-neutral and provides no access control by itself;
  listener ownership, permissions, and peer admission belong to `helm-session`.
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
- `ipc::tests::socket_path_honours_the_environment`.
- *Planned (M2):* a CI job that drives a live session end to end with `socat`
  and `jq` only, so the scriptability claim is tested rather than asserted.
- *Planned (M2):* a handshake test asserting that a client presenting the wrong
  `PROTOCOL_VERSION` is refused with a `Response::Hello` and then disconnected.
- *Deferred M2 hardening:* before changing `ipc::socket_path()` or shipping a
  server, write an accepted SPEC 0001/0003 delta and failing tests that decide
  the override fixture contract, runtime-directory fallback, same-uid
  `SO_PEERCRED` admission, Hello/subscription ordering, and connection/frame/
  queue limits. Until then these are proposals, not the M0 public contract.
