# ADR 0003 — A session daemon owns the state; clients subscribe

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

helm ships several processes that all need to know the same things: which orbit
is active, which are occupied, the current layout, the input mode, the focused
window title, the pending chord. The bar draws all six of those. The launcher
needs the active orbit to place what it spawns. `helm ctl` needs them to print
`orbit --list` and `ledger show`.

`helm-core::state::HelmState` is exactly that set, and it is deliberately a
plain data struct with no behaviour, because "the bar redraws when and only when
this value changes" is what event-driven rendering means in practice.

Two constraints shape where that struct lives:

- The bar's budget is a redraw under 8 ms and idle CPU near zero
  (`docs/ARCHITECTURE.md` §4). Anything that polls fails the second half.
- ADR 0002 says the compositor is swappable. Anything that couples a client to
  the compositor's own IPC undoes that.

`docs/PITFALLS.md` also records the failure mode "bar crashes, black screen".
Something has to own lifecycle, and it must not be a client.

## Decision

`helm-session` is a long-lived user daemon. It, and only it:

1. Owns the authoritative `Ledger` and derives `HelmState` from it.
2. Drives a `WmBackend` (ADR 0002) and reconciles the backend's window events
   into the ledger.
3. Serves the control socket (ADR 0004): accepts `Request`s, answers
   `Response`s, and pushes `Event::State` to every subscriber on change.
4. Launches and supervises clients. The bar, the launcher and the terminal are
   restartable; the session outlives all of them.

Clients hold no authoritative state. `helm-bar` connects, sends
`Request::Subscribe`, and renders whatever arrives. If it dies, systemd restarts
it, it re-subscribes, and it gets a fresh snapshot. It never computes state and
never asks the compositor anything.

Coalescing is the session's job: `Event::State` is sent once per change, and
clients additionally drop no-op frames with `HelmState::renders_same_as`.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Each client polls the compositor directly** (the waybar/eww default) | No daemon to write; each client is independently useful; a crashed client cannot take state with it because there is no shared state | N pollers means N timers, which means idle CPU never reaches zero and the frame budget fails by construction. It also means N implementations of "what does occupied mean", which drift. And every client would import niri's IPC vocabulary, defeating ADR 0002 |
| **The compositor serves clients directly** (sway's own IPC model) | One fewer process; lowest possible latency; the compositor already knows everything | Couples every client to the compositor. When `NativeBackend` replaces `NiriBackend` at M5, every client's protocol changes. It also means `helm-compositor` grows a socket server, a theme reloader and a process supervisor, which is scope we specifically moved out of it |
| **A shared-memory or file-backed state blob** | Cheapest possible read; no serialisation on the hot path | No change notification without a watch, so we are back to polling or to inotify with its own edge cases. Versioning a memory layout across a package upgrade is worse than versioning JSON |
| **D-Bus with the session as a service** | Standard desktop pattern; free activation and signalling | Argued in ADR 0004; the socket won on scriptability and on not needing a bus to be alive before the desktop is |

## Consequences

### Good

- One definition of every derived value. `HelmState` is computed once.
- Clients are trivially testable: feed them JSON, assert what they draw. No
  compositor, no bus.
- Swapping the compositor changes one module, as ADR 0002 promises.
- Client crashes are recoverable and invisible: restart, re-subscribe, redraw.
- Idle CPU can genuinely reach roughly zero, because nothing polls. Only the
  clock module ticks, once a second, and it ticks inside the session.

### Bad

- The session is a single point of failure. If it dies, the bar has no state
  source and the keymap stops responding. It must be the most conservative code
  in the tree, and it needs its own restart policy plus a story for what the
  compositor does while it is down.
- One extra process and one extra hop on every state change. The 8 ms budget
  absorbs it, but it is real.
- Everything serialises through JSON, so `HelmState` must stay small. That is a
  discipline, not a guarantee.

### Neutral

- The daemon is the natural home for the theme reload fan-out (ADR 0005) and for
  `helm ctl doctor`'s runtime checks, so those get a home for free.

## Reversal

Low. The seam is `helm-core::ipc` plus the client's connection module. Folding
the daemon into the compositor later would mean moving the socket server and the
supervisor into `helm-compositor` and leaving the wire protocol untouched:
clients would not notice. Estimated a few days.

The signal to reconsider is latency: if the extra hop is ever measured as the
reason the 4 ms key-to-geometry budget is missed, the ledger belongs inside the
compositor and the daemon becomes a state mirror.

## Guard

- `state::tests::revision_alone_does_not_force_a_redraw` — fails if the no-op
  frame drop stops working, which is what keeps idle CPU at zero.
- `state::tests::state_round_trips_through_json` — fails if `HelmState` grows a
  field that cannot cross the socket.
- *Planned (M2):* an integration test that starts `helm-session`, connects two
  subscribers, applies a ledger mutation and asserts both receive exactly one
  `Event::State` with identical content.
- *Planned (M2):* a supervision test that kills `helm-bar` and asserts the
  session is still serving and the restarted bar receives a full snapshot.
