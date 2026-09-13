# SPEC 0003 — realm-session

- **Status:** Accepted (2026-09-10; control-transport and nonblocking backend
  transaction corrections 2026-09-11; Task 3 MVP fail-closed, provenance, and
  evidence-ownership corrections 2026-09-12; bounded Wayland transport API
  clarification 2026-09-12)
- **Milestone:** M2
- **Issues:** [#36](https://github.com/Pipeliner/realms-de/issues/36),
  [#38](https://github.com/Pipeliner/realms-de/issues/38),
  [#40](https://github.com/Pipeliner/realms-de/issues/40)
- **Decisions:** [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md),
  [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0004](../adr/0004-ndjson-control-socket.md),
  [ADR 0009](../adr/0009-no-animation-budget.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md),
  [ADR 0017](../adr/0017-immutable-theme-activation-generations.md),
  [ADR 0021](../adr/0021-bounded-wayland-ingress-and-dispatch.md)
- **Implements:** [INTERFACES.md §1](../INTERFACES.md) (`WmBackend`) and
  [§4](../INTERFACES.md) (the socket's server half)
- **Supersedes / Superseded by:** Theme apply/reload clauses are superseded by
  [SPEC 0011](0011-theme-activation-generations.md): the CLI publishes a
  generation without a session request or notification. Accepted
  [SPEC 0012](0012-activation-launch-lifecycle.md) governs activation ownership,
  lifecycle reconciliation and the WM unit restart rule.

> Written before the corresponding code, as S14 requires. The **Test** column
> names the exact obligations to add next; each must be observed failing before
> its implementation is written.
>
> **On verification.** Every claim below about river's behaviour is marked
> *(verified)* where it was read from the protocol XML at
> `codeberg.org/river/river`, tag `v0.4.8` (commit
> `c4b5f706314555f4846e25b8d3635631387b3fdd`), `protocol/*.xml` and the
> implementation files named under *Resolved design questions*. ADR 0013 exists because a
> plausible summary of this protocol was wrong in three places; four further
> corrections are recorded in *Behaviour §2 and §3* below, and the ADR and
> `INTERFACES.md` need amending for them.

## Purpose

`realm-session` is the process that makes realm a desktop rather than a library.
It holds the one authoritative `Ledger`, derives the one `RealmState` every
client draws from, serves the control socket that makes the desktop scriptable,
and — under river 0.4 — *is* river's window manager. While it is running,
windows are where the ledger says they are and keys do what the keymap says.
While it is not, river has no window management at all: windows are unplaced,
unfocused and unresized, and no keybinding works. A user notices its absence
within one keystroke.

## Scope

**In:** connecting to river and negotiating protocol versions; owning `Ledger`
and deriving `RealmState`; driving a `WmBackend` and translating its events back
into ledger mutations; implementing the window-manager half of
`river-window-management-v1` and *serving* `river-layer-shell-v1`,
`river-xkb-bindings-v1`, `river-input-management-v1`, and
`river-libinput-config-v1`; the keymap, the mode machine and key repeat; the
control-socket server and subscriber fan-out; the right-hand bar modules;
ledger persistence and restart recovery; client lifecycle.

**Out:**

| Not this component's job | Whose it is |
|---|---|
| Drawing anything | `realm-bar` (SPEC forthcoming, [ADR 0008](../adr/0008-layer-shell-rendering-stack.md)); the bar is an ordinary `wlr-layer-shell` client |
| Template expansion, sealed generation publication, and generation-aware diff | `realm-theme` ([SPEC 0002](0002-theme-pipeline.md), [SPEC 0011](0011-theme-activation-generations.md)); `realmctl` calls it in-process and the session is not involved |
| The ledger's mutation rules, the layout projection, colour maths, the wire types | `realm-core` ([SPEC 0001](0001-realm-core-contracts.md)) |
| The environment handshake, systemd ordering, portals, cursor theme | the session entry contract ([ADR 0011](../adr/0011-session-integration-contract.md)) and `packaging/` |
| The CLI surface | `realmctl`; it is a client of this socket and holds no privilege |

The boundary with `realm-core` is a rule, not a suggestion: `realm-session` must
not reimplement or second-guess ledger policy. When a control-socket request
arrives it calls the corresponding `Ledger` method and re-projects. If a
mutation's outcome looks wrong, the fix belongs in `realm-core` with a test,
not in a special case here. Two sources of window-order truth is the exact
failure [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md) exists to
prevent.

## Behaviour

### 1. Lifecycle

**Start-up order.** `realm-session` runs as `realm-wm`, started by
`realm-wm.service` after the environment import in ADR 0011 step 3. It
refuses to start without `WAYLAND_DISPLAY` (already enforced by the unit's
`ConditionEnvironment`).

1. **Prepare and bind the control endpoint first, without listening.** While
   the process is still single-threaded, prepare the fixed endpoint and obtain
   `BoundControlEndpoint` through the Accepted capability, filesystem,
   singleton-ownership, and stale-reclaim contract in
   [SPEC 0007](0007-control-socket-security.md). This session specification
   does not define another path, reclaim predicate, or transport API. The
   bound fd remains private and has `SO_ACCEPTCONN == false`; pre-recovery
   clients therefore receive the bounded retry behaviour specified there
   rather than entering a socket that cannot yet answer.
2. **Load and classify the snapshot before River can offer replay.** Start the
   worker, submit exactly one pathname-based snapshot read, and wait in a
   startup-only `poll` on its `eventfd` until that result is available. The
   control endpoint remains non-listening. At this point no River window-manager
   global has been bound, so a delayed file read cannot hold an offered manage
   sequence open. Classify `NotFound`, valid, rejected, and fatal results by §6
   before continuing.
3. **Connect to River, discover the required globals, and negotiate versions.**
   This step records the registry name and advertised version for each row; it
   creates no protocol binding object yet:

   | Global | Bind at | Refuse below | Because |
   |---|---|---|---|
   | `river_window_manager_v1` | 5 | **4** | `river_window_v1::identifier` and `river_window_manager_v1::exit_session` are `since="4"`; `set_content_clip_box` is `since="3"` *(verified)* |
   | `river_xkb_bindings_v1` | 3 | **3** | `modifiers_watch` / `modifiers_update` are `since="3"` and carry the mode badge and chord echo; `get_seat`, `ensure_next_key_eaten` and `ate_unbound_key` are `since="2"` *(verified)* |
   | `river_layer_shell_v1` | 1 | **1** | Its only version *(verified)* |
   | `river_input_manager_v1` | 2 | **2** | `set_repeat_info` is v1; the atomic device boundary `done` is `since="2"` *(verified)* |
   | `river_libinput_config_v1` | 2 | **2** | The per-device atomic `done` boundary is `since="2"` *(verified)* |

   Child objects (`river_window_v1`, `river_seat_v1`, `river_output_v1`,
   `river_node_v1`, …) carry the version their factory was bound at; none of
   the creating events takes a version argument *(verified — the `window`,
   `output` and `seat` events have a bare `new_id` arg)*. So one bind version
   settles the whole tree.

   On a version below the refusal column, or on a missing global, exit non-zero
   with a message naming **the interface, the version advertised and the
   version required**, and make no window-management request. Half-managing a
   session on an older river is worse than not starting: the user gets a
   desktop that mostly works and fails at the one thing they were doing. This
   closes the "protocol version drift after a river bump" row of
   [PITFALLS.md](../PITFALLS.md).

4. **Bind each of the four companion globals exactly once.** Bind
   `river_layer_shell_v1`, `river_xkb_bindings_v1`,
   `river_input_manager_v1`, and `river_libinput_config_v1` at the table's
   version. Do not bind `river_window_manager_v1` yet. The layer-shell binding
   is unconditional: binding it *is* the signal
   that realm supports layer shell: "If the window manager does not bind this
   interface, the compositor should not allow clients to map layer surfaces.
   This can be achieved by closing layer surfaces immediately" *(verified,
   verbatim)*. **Until this bind happens the bar does not appear at all**, and
   the symptom looks like a broken bar rather than a broken window manager.
5. **Configure the minimal MVP input policy before opening management.**
   `river-input-management-v1` and `river-libinput-config-v1` requests are not
   part of a manage sequence. The seat named `default` always exists and every
   unassigned input device already belongs to it *(verified)*; Realm creates no
   seat and sends no `assign_to_seat`, scroll-factor, mapping, natural-scroll,
   acceleration, drag, or other preference request in M2. Those current values
   remain compositor/libinput policy rather than guessed user preferences.

   Issue a `wl_display.sync` and service bounded quanta through its callback so
   every pre-existing input/libinput device event preceding the callback is
   classified. No layer-shell output object can exist yet:
   `river_layer_shell_v1::get_output` requires a `river_output_v1`, and River
   creates those output objects only after the window-manager global is bound
   and within its initial manage sequence *(verified against River v0.4.8)*.
   After each input-device `done`, send
   `set_repeat_info(25, 600)` exactly once for a keyboard and no repeat request
   for another device type. After each libinput-device `done`, send
   `set_tap(enabled)` exactly once only when `tap_support` reports a positive
   finger count and `tap_current` is disabled; an already enabled or unsupported
   device receives no request. Every emitted tap request owns one result object:
   `success` is required, while `unsupported` or `invalid` is a typed fatal
   startup/backend-contract error. Issue a second `wl_display.sync` after those
   requests and do not proceed until every result preceding its callback has
   settled. A device removed before its policy request/result completes is
   destroyed and its unfinished policy is cancelled without object reuse.
   Devices hot-plugged after `Live` follow the same done-gated policy through
   bounded service, without re-running startup or perturbing focus.

   Runtime keyboard-layout switching is deliberately post-MVP. Realm does not
   bind `river_xkb_config_v1` in M2 and preserves the keymap/layout with which
   River created each keyboard. This corrects the earlier unsupported claim
   that the MVP offered a switchable layout; adding a layout source, switching
   action, state publication, persistence, and UI requires its own Accepted
   contract.
6. **Seed.** `Ledger::new()` gives six orbits with orbit 1 active
   (`ORBIT_COUNT` is 6, `OrbitId::rune()` gives `ᚠᚢᚦᚨᚱᚲ`). If a recoverable
   snapshot exists, apply §6 instead.
7. **Bind the window-manager global last, recover, then await authoritative
   workarea before projecting.** Only after both step-5 input sync fences bind
   `river_window_manager_v1` exactly once. Return the constructed backend, let
   Session create the stable xkb binding mechanisms, then service the initial
   manage/replay turn. This ordering ensures River
   cannot hold an open manage sequence while Realm waits for an input or
   layer-shell sync callback. If `river_window_manager_v1::unavailable`
   arrives, another window manager holds the seat *(verified: "guaranteed to be
   the first and only event")*. Exit non-zero immediately; do not retry.
   `StartLimitBurst=5` in 30 s then surfaces it as a dead unit rather than a
   crash loop. §3 covers the keymap. Complete
   §6's replay/reconciliation. River reports each `river_output_v1` only in
   this first turn. The backend selects the first complete output and first
   `river_seat_v1` in report order. It creates one
   `river_layer_shell_output_v1` per output and, for the selected seat, exactly
   one `river_xkb_bindings_seat_v1`, one `river_layer_shell_seat_v1`, and one
   xkb binding object per pre-registered stable binding spec. The first turn is
   answered without a window projection while Session keeps every binding
   disabled. The first selected-output `non_exclusive_area` arrives in a later
   manage turn. Only the clean completion of the complete projection answering
   that authoritative `WorkareaChanged` transitions Session to `Live`.
   No neutral or output-sized placeholder is projected. If the initial replay
   contains no complete output, the backend answers the open manage sequence
   neutrally and then terminates startup as unavailable; it never waits
   indefinitely or reports readiness.

   Selection is deterministic and backend-local. The first complete output in
   River creation order is selected and remains selected while present. The
   backend requests a layer-shell output object exactly once for every output,
   retains each output's latest complete geometry and non-exclusive area, and
   sends `set_default` only for the selected output in an applicable manage
   response. Additional outputs are supported but do not split Realm's single
   workspace projection. If the selected output disappears, the earliest
   surviving creation ordinal with both complete geometry and a retained
   non-exclusive area becomes selected, its latest complete workarea is
   exposed, and it becomes the default in the same response. If no such output
   survives, the backend becomes unavailable; systemd may restart when a usable
   output exists again. Output object ids and ordinals are incarnation-local
   and never reused.

   Selected-default application is backend-private: it is not a field of
   `BackendPolicyResponse`, and a Session fake cannot observe which
   `set_default` request RiverBackend emitted. RiverBackend records an attempted
   selected default against the response that carried it. For the MVP, a failed
   terminal result participates in Session's fatal fail-closed rule; Session
   does not manufacture a neutral repair response. RiverBackend still leaves
   the failed default dirty rather than claiming success. Re-emitting it in a
   later applicable response, including a future neutral recovery response, is
   explicit post-MVP #40 work with real-backend proof
   `backend::tests::failed_selected_default_is_reemitted_by_next_applicable_response`.

   M2 is deliberately single-seat. The first `river_seat_v1` in River creation
   order is selected and owns all Realm focus requests, xkb bindings/chord
   policy, modifier observation, and layer-shell exclusive-focus observation.
   Additional seats remain compositor-owned and receive no Realm binding or
   layer-shell-seat object. `configure_bindings` registers and bounds mechanism
   specs before replay but performs no seat-dependent protocol request; object
   creation waits for the selected seat. If no seat exists at the replay
   barrier, answer the open sequence neutrally and then terminate startup as
   unavailable. Removing the selected seat after replay is likewise unavailable
   rather than silently switching a user's input authority. Seat object ids and
   selection are incarnation-local.

   The bound endpoint remains non-listening throughout recovery and no
   readiness notification is permitted. Recovery is
   driven by a pre-listener pump, not by waiting for the normal control loop:
   it uses the same one-quantum backend driver and polls only the backend fd.
   The worker snapshot result was already classified before the window-manager
   global was bound. Backend
   immediate/prepared-read/write progress, replay turns, pending completions,
   and successful retained-observation drains continue through this pump until
   Session reaches `Live`. No clock/repeat timer or control fd is admitted in
   this phase. Backend loss, worker read failure other than `NotFound`, poll
   failure, or any failed post-admission replay result terminates startup without
   activating the listener or sending readiness.
8. **Activate, verify, then report ready.** Consume the one
   `BoundControlEndpoint` to call `listen(..., 64)` exactly once. Continue only
   with the returned `ActiveControlListener`, which proves
   `SO_ACCEPTCONN == true`, then consume that listener into SPEC 0007's
   `ControlServer`; activation or verification failure is fatal and sends no
   readiness. Only then call `sd_notify(READY=1)` and enter
   the normal poll loop (§4). `realm-wm.service` switches from `Type=exec` to
   `Type=notify`, so `realm-bar.service` can order after a session that answers.

The capability-order invariant is therefore exact: prepare/bind endpoint;
recover and project; transition to `Live`; consume the bound capability to
activate; verify the listener; consume it into `ControlServer`; only then send
`READY=1`. No error path may reorder or skip one of these boundaries.

**Shutdown.** Response receipts, response-barrier settlement, and every fixed
deadline in this sequence are #38 driver/transport concerns. Session owns no
receipt and no clock. When the adapter first observes the terminal
`QuitPending` boundary, after all pre-Quit effect reservations have committed,
it immediately submits `PersistenceCoordinator::shutdown_snapshot()` and seals
the worker with the same captured monotonic `now`. This fixes the non-sliding
worker deadline at `now + 2,000 ms` before any response-barrier wait or control
shutdown drain. Direct `Request::Quit` first calls
`Session::begin_direct_quit()`, which is valid in Live and returns the
`QuitPending { CurrentControlRequest }` boundary. #38 queues that request's Ok
and waits for its exact response receipt to become Drained or Closed. A
key-derived `Action::Quit` remains transactional until its final clean boundary;
if another request owns that boundary, #38 drains or closes its exact response
first. Only then does the adapter call SPEC 0007's total control-server
`begin_shutdown` once and externally invoke Session's total `begin_shutdown`
once. ControlServer
stops transport admission and closes every non-subscriber immediately; Session
stops Realm actions and abandons any remaining private candidate, origin,
ticket, or close without commit, action completion,
persistence, or publication. The #38 loop continues servicing subscriber
interests, exact expiry, and bounded worker results/fence. It may begin
compositor exit
only when `is_shutdown_complete()` is true and the worker fence either
acknowledged every pre-seal terminal result or reached its explicit 2,000 ms
degraded-shutdown deadline.
Session performs no QuitPending/ShuttingDown drain or neutral-discard protocol
in the MVP and allocates no shutdown response ticket. #38 does not ask Session
to reduce another public backend event after the Quit handoff; once its external
conditions settle, it invokes `Session::begin_exit_session()`. RiverBackend's
#40-owned cutoff, not a Session fake, finishes or supersedes backend-private
pending/open protocol work. Rich discard sequencing remains explicit post-MVP
scope.

`Session::begin_shutdown()` is total and idempotent. Its first call abandons
all private candidates, origins, tickets, closes, and
effects while retaining the consumed-ticket watermark, enters
ShuttingDown, and returns only the immediate repeat-timer Disarm. It calls no
backend method and emits no persistence, state, action completion, diagnostic,
or ordinary effect. The first call has this effect in InitialReplay,
FinalizingReplay, Live, or QuitPending; calls in ShuttingDown, Exiting, or
ExitComplete are unchanged. In contrast, `begin_direct_quit()` is valid only in
Live and idempotent in QuitPending; every pre-live/post-shutdown phase returns
typed `InvalidLifecycleOperation` without mutation.

`Session::begin_exit_session()` is legal exactly once in ShuttingDown. #38 MUST
first prove that the control drain is complete and the worker fence acknowledged
or explicitly degraded at its deadline; Session cannot observe those external
facts and rejects only a wrong-phase or repeated invocation. It
calls `WmBackend::begin_exit_session(BackendExitPolicy)` exactly once with the
last committed enabled/watch sets and enters the explicit terminal `Exiting`
phase only on backend success. A wrong-phase or repeated call is a fatal contract
error. It is nonblocking. Its successful call is one local exit linearization
cutoff: the backend cancels any prepared read, stops new ingress admission, and
suppresses public exposure of terminal turns/completions. Any turn already
consumed by `respond_policy_turn` retains every emitted/queued byte and finishes
any manage/render phase already open or parsed at the cutoff exactly once from
its accepted immutable response, even if only a prefix was written; its public
completion/drain is suppressed. A later render phase whose `render_start` is
still kernel-unread is abandoned and never replaced. Only an
admitted/open/queued sequence not yet exposed and answered receives no
projection/closes, those authorized sets, and next-key Cancel.
Kernel-unread bytes, decoded non-policy work, and incomplete decoder fragments
are terminally superseded; every backend-owned descriptor retained by
superseded work is closed exactly once. Every applicable open-phase finish is queued and
flushed before the exit request is queued and flushed. No later public policy
request or response is issued. The pinned [River v0.4.8 implementation](https://github.com/riverwm/river/blob/v0.4.8/river/WindowManager.zig#L199-L202)
handles `exit_session` by terminating the display regardless of its current manage or
render state, matching the protocol's promise to disconnect all session
clients; the cutoff therefore does not wait for kernel-unread policy events.
After the cutoff, `BackendPollInterest::readable` is false. The loop services
only immediate flush, requested writable work, and independently reported
terminal readiness one bounded quantum at a time, rechecking poll interest
after each quantum. Readable readiness alone neither admits bytes nor completes
exit. Service returns only `None` until
the backend reports `Disconnected` after every internal sequence finish and
every byte through the exit request have been flushed; any other public event
is a fatal backend-contract violation.
Only that post-flush disconnect returns the one-shot
`BackendTurn::ExitComplete`, completing logout successfully; it is neither a
restartable failure nor an unsolicited clean WM exit. A second service call
after ExitComplete is a contract error because the driver must stop the target.
The backend reports
disconnect or I/O loss before that flush as a service error; failure before
`begin_exit_session()` succeeds, or any service error during the exit drain, is
fatal. The compositor exit makes the entry
freeze admission and stop `realm-session.target`; that target stop is what
prevents restart. SPEC 0012 fixes `realm-wm.service` at `Restart=always`, because
an unsolicited clean WM exit while the compositor and target remain live is not
a logout and must not leave river unmanaged.

### 2. The manage sequence, and which state is which

This is the part that is a protocol error rather than a style question. The
protocol declares "two disjoint categories of state" *(verified, verbatim)*,
and modifying either outside its sequence is
`river_window_manager_v1::error::sequence_order`.

The real loop, from the interface description *(verified)*:

```
  server: … state events …  manage_start
  realm:   window-management requests [+ rendering requests]  manage_finish
  server: sends new state to windows, waits for replies
  server: river_window_v1::dimensions × n           render_start
  realm:   rendering requests                        render_finish
  server: if dimensions changed again → back to render_start
          if WM-relevant state changed, or realm sent manage_dirty → back to manage_start
```

Two consequences the repo's current docs understate:

- **`INTERFACES.md` §1 says "a projection maps onto one `manage` sequence". That
  is half of it.** A single projection transaction spans a manage sequence *and* the render
  sequence that follows it, because the size half of a `Placement` is
  window-management state and the position half is rendering state. realm cannot
  send `manage_finish` and then immediately send positions; it must wait for
  `render_start`.
- **realm cannot start a manage sequence at will.** It can only ask, with
  `manage_dirty`, and river starts one "as soon as possible" *(verified)*. So a
  control-socket mutation is: mutate the ledger → `manage_dirty` → wait →
  apply in the sequence river then opens.

**The category table.** Every row was read from the request's own "This request
modifies … state" sentence *(all verified)*.

| realm operation | river request | Category |
|---|---|---|
| `Placement::rect` **size** | `river_window_v1::propose_dimensions(w, h)` | **window management** |
| `Placement::rect` **position** | `river_node_v1::set_position(x, y)` | **rendering** |
| `Placement::focused` — the focus itself | `river_seat_v1::focus_window` | **window management** |
| `Placement::focused` — the 1 px seam and inset glow | `river_window_v1::set_borders(edges, width, r, g, b, a)` | **rendering** |
| `Placement::occluded` (mono) | `river_node_v1::place_top` on the focused window | **rendering** |
| Stow (`Orbit::stowed`), and every window in a non-active orbit | `river_window_v1::hide` / `show` | **rendering** |
| Fullscreen | `river_window_v1::fullscreen(output)` + `inform_fullscreen` | **window management** |
| Banish | `river_window_v1::close` | **window management** |
| Tiled-edge hint, so CSD clients stop drawing shadows into the seam | `river_window_v1::set_tiled(edges)` | **window management** |
| Quantisation clip (§5) | `river_window_v1::set_content_clip_box` | **rendering** |
| Per-mode keymap | `river_xkb_binding_v1::enable` / `disable` | **window management** |
| Chord submap | `river_xkb_bindings_seat_v1::ensure_next_key_eaten` | **window management** |
| Default output for unanchored layer surfaces | `river_layer_shell_output_v1::set_default` | **window management** |

`hide`/`show` being rendering state is the row people get backwards, and it is
better for realm rather than worse: a stowed window stays managed and stays in
the ledger, which is exactly what `Orbit::stowed` means. ADR 0013 records this
as correction 2; it is confirmed here against the XML.

**One policy turn owns one River response boundary.** River reports every
changed state and binding event preceding a `manage_start`, then waits for
exactly one `manage_finish`. The backend therefore exposes that complete
report-order batch as one `BackendEvent::PolicyTurn`, does not finish it before
Session answers, and does not service past an unanswered turn. Individual
observations followed by a bare boundary marker are forbidden: Session could
otherwise answer after the first observation and miss later facts in the same
River batch.

After backend construction and before the initial replay is serviced, Session
calls `configure_bindings` with stable mechanism-only ids, keysyms, and
modifier sets. This call validates and stores at most 64 specs but performs no
seat-dependent protocol request. The production backend creates the concrete
objects only when the first replay reports its selected `river_seat_v1`. The
backend never receives Realm `Mode` or `Action` values. Session retains the
private id-to-binding-to-policy mapping and includes
the complete desired enabled binding set, modifier watches, and any one-shot
next-key edge in each `BackendPolicyResponse`. That edge is the explicit enum
`Preserve | Ensure | Cancel`, not a Boolean. `Cancel` emits
`cancel_ensure_next_key_eaten`. The MVP uses the edge only in successful policy
responses and the fixed terminal exit policy; a post-admission failure ends the
session before any edge-repair decision. Cancelling an uncertain failed Ensure
or restoring an Ensure after `UnboundKeyEaten` is part of the explicit
post-MVP recovery scope, not a launch obligation.
Any binding press/release/repeat-stop id absent from that configured private map
is a typed fatal `BackendContractError::UnknownBinding` before partial reduction
or response. Treating it as a no-op would hide mechanism/policy drift.

The Accepted MVP resource envelope is 256 managed windows, 64 configured
binding mechanisms, 16 live River outputs, 16 live River seats, 64 live input
devices, 64 live libinput devices, and 256 ordered events in one live/draining
response boundary. Events are capped at 256 per turn. Staged effects are independently
capped at 256 across the complete active transaction and every tagged
follow-up; an external admitted effect counts with all turn-derived effects.
The sum of UTF-8 bytes in every occurrence of a backend identity, app id, title,
or other String carried by one exposed turn is capped at
`MAX_POLICY_TEXT_BYTES = 65_535`, including replay; both the backend before
exposure and Session before partial reduction enforce the same total. Every
`ModifiersChanged.old/new` vector is a canonical enum-order, duplicate-free
subset of the four declared modifiers and therefore has length at most four;
noncanonical vectors are a typed fatal backend-contract error before partial
reduction.
The four backend-local live-object caps include objects whose initial `done`
has not arrived. Each kind has a checked nonzero `u64` creation ordinal; an
ordinal never wraps or is reused in one incarnation. The production backend
checks the applicable cap and ordinal before installing a map entry or creating
a dependent child. Overflow is `BackendError::Capacity` for `Outputs`, `Seats`,
`InputDevices`, `LibinputDevices`, or `ObjectOrdinals`, installs no partial
state, and destroys the just-received proxy plus already-owned children exactly
once. Removal frees a live slot but never an ordinal. The input-device `name`
string and every unused libinput value/array are discarded in their one bounded
dispatch quantum; per-device retained state consists only of the closed
type/link/tap support/current/done/result fields required by the Accepted fixed
policy. Thus a stream of pre-`done` objects or irrelevant device properties
cannot accumulate unbounded normalized state.
Initial replay permits at most 259 events: 256 opens, one optional latest
ordinary focus, one optional latest exclusive-focus state, and its singleton
final `InitialReplayComplete` barrier. Session construction has no
authoritative workarea. River cannot supply a layer-shell workarea in the first
turn because its layer-shell output object depends on a `river_output_v1` first
created by that turn. The replay response therefore reconciles identities but
contains no projection and keeps bindings disabled. The first selected-output
workarea is accepted only in a later turn; its complete projection must reach a
clean final boundary before Session can enter Live or publish revision 1.
Reaching the applicable numeric limit is valid; crossing it is a typed fatal
capacity error before any partial turn is exposed or response is submitted.
A replay workarea, duplicated focus state, repeated barrier, misplaced barrier,
or any other forbidden replay content is instead its declared typed
contract/protocol error before assignment or response and is never reclassified
as capacity exhaustion.
Every compositor-origin window reference in the offered batch is a
`BackendWindowId`; the backend never allocates Realm identity. In live/draining
turns Session resolves the batch in report order. `WindowOpened`
allocates/restores and locally assigns the identity, after which later title,
focus, geometry, or close events in that same turn can resolve it. Initial
replay is the sole exception: a focus reference may resolve against an earlier
open in the replay accumulator, but allocation and `assign_window` wait until
the final barrier preflights and reconciles the entire batch. A reference before
open, or before assignment outside that exception, is fatal.

Title, focus, workarea, modifier, geometry, and lifecycle facts may be coalesced
only when their full report-order reduction is identical. One named
normalization exception exists: a backend-local object opened and closed wholly
before either lifecycle fact is exposed may be elided and is defined never to
have entered Realm identity authority. Every other lifecycle reduction includes
the never-reuse watermark; once an open is exposed it cannot be cancelled.
Binding press, release, repeat-stop, and unbound-key input are never dropped,
coalesced, or reordered. Derived actions preserve report order until the first
Quit. That Quit is a sticky terminal policy barrier across the entire
transaction: later derived actions/effects in this and every tagged follow-up
turn are suppressed, while later authoritative input/compositor facts are still
retained. The response projection is bounded by the 256-window envelope. Its
close list is first-occurrence ordered and contains each still-live WinId at
most once; repeated close derivations collapse to one edge and a close after a
WindowClosed fact emits none, so closes are bounded by the same envelope. ADR
0021 and A40-A41 make both raw I/O and the complete normalized transaction
mechanically finite.

The private pinned Rust Wayland backend adaptation exposes only the primitives
needed to enforce that schedule: an exclusive `prepare_read_bounded` guard
whose consuming `read_once` performs one `recvmsg`, `dispatch_one_pending`
which parses and handles at most one already-admitted protocol message without
reading the socket, and `flush_once` which performs at most one `sendmsg`.
Their result values distinguish read-needed, progress/complete, and
`WouldBlock` without an internal retry. Dropping the exclusive guard cancels
it; a second preparation and protocol dispatch are refused while it is live.
The adaptation remains in the pinned `wayland-backend` parser, object map, and
`ObjectData` callback path; it does not introduce another wire decoder or use
`wayland-client`'s stock drain-all event-queue APIs. River's future backend
owns that guard across poll and calls these primitives according to ADR 0021's
phase priority.

An external desired action that changes required response state, a dirty
projection-equal action, or a close request stages private intent, reserves its
origin ticket, and idempotently calls `request_policy_turn`; it neither commits
nor submits an independent projection or close. A clean projection-equal action
with no binding/edge change finalizes locally as §9 defines. The eventual policy turn may
also contain compositor observations or input. Session folds an admitted
external candidate first and then folds every event in report order before
deriving the sole response. A key event is therefore resolved against current
ledger and mode state and its binding-state, close, projection, process, or quit
effects are linearized at the same response boundary. Empty, release-only,
modifier-only, spawn-only, quit, and projection-equal turns still receive one
response and one fresh response ticket.

`BackendPolicyResponse::projection` is `None` only when the clean projection is
preserved; `Some(empty)` means hide every managed window. Every placement names
a window assigned in the current backend incarnation, except for the exact
tombstone exception below. The response's close list contains bounded edge
requests for assigned live windows only. There are no independent
`submit_projection`, `submit_close`, or focus methods: two submission owners
could not safely linearize an external action with observations or a key event
in the same River sequence.

`respond_policy_turn` is nonblocking. `BackendSubmission::Complete` means the
entire transaction below completed, its final `render_finish` was flushed, and
the admitted post-turn epoch was sealed empty: no decoder fragment or queued
protocol message admitted before this decision can later yield retained facts.
If final output remains or any admitted post-turn work remains unnormalized,
the backend returns `BackendSubmission::Pending` even when the finish flushed
synchronously. Pending means the immutable response was accepted, and
exactly one matching `BackendEvent::OperationCompleted` later carries its
terminal result; after Ok, normalization produces exactly one tagged retained
turn or empty marker. This remains true unless that backend incarnation
terminates or #38 externally calls `begin_shutdown` first. Either transition
abandons the ticket without committing or completing the action; Session emits
no shutdown-discard response and #40 owns the backend cutoff. Returning an error
from `respond_policy_turn` is fatal for every class because River may otherwise
remain waiting in an open sequence. Operation failures after acceptance arrive
as matching terminal events and are fatal to the MVP Session. Concretely, over one manage
+ render pair:

1. In the manage sequence: for each placement whose `rect` size differs from
   the size last proposed for that window, `propose_dimensions(w, h)`. For a
   newly managed window, also `set_tiled` and `set_capabilities`. If the
   focused window changed, `focus_window`.
2. `manage_finish`.
3. In the following render sequence: `set_position(x, y)` for every placement
   whose position changed; `set_borders` for every placement whose focus flag
   changed; `hide` for every managed window not in the slice; `place_top` for
   the focused window under `Layout::Mono`; the clip boxes from §5.
4. `render_finish`.

The successful terminal result for a response containing a close edge means the
close request and sequence finishes were flushed; it does not mean that the
client closed. Only a later `WindowClosed` event inside a policy turn removes the
window. Standalone focus is absent from the backend seam because focus is
already carried by `Placement::focused` in a projection.

**Idempotence** (required by `INTERFACES.md` §1): after successful completion the backend keeps the last
applied position, size, focus flag and hidden flag per window and issues no
request for an unchanged value. Because `layout::project` is pure, "did anything
change?" is an equality check on the projection, not a rectangle diff — which is
most of the 4 ms budget.

**Atomicity.** A swap is two `set_position` requests in one render sequence, so
the two windows exchange places in a single frame with no intermediate geometry.
Undo is the same shape: restore the snapshot, re-project, apply once. This is
the structural form of "no animation" that
[ADR 0009](../adr/0009-no-animation-budget.md)'s planned M2 guard asserts.

**The show-after-dimensions rule.** A window that has been hidden (in another
orbit, or stowed) may hold a stale size if the workarea changed while it was
hidden. `show` is rendering state and applies at `render_finish`, but the
`dimensions` event answering a `propose_dimensions` "may not be possible … in
the very next render sequence" if the window is slow to respond *(verified)*.
So realm proposes the new size in the manage sequence and issues `show` only in
the render sequence whose `dimensions` event covers the projected rect. An
orbit switch is therefore one frame in the common case and two when the window
was slow — never a frame of a window at the wrong size.

**Fullscreen and the bar.** `fullscreen` makes the compositor own position and
dimensions and ignores `set_position`, `set_clip_box` and
`set_content_clip_box`; borders are not drawn *(all verified)*. This matches
`layout::tests::fullscreen_covers_the_output_including_the_bar`, where the
projection is `area.output`.

> **Correction to ADR 0013.** Its fullscreen row says "Shell surfaces above the
> fullscreen window still render, so bar visibility over fullscreen is our
> choice of node order." The XML sentence is about `river_shell_surface_v1` —
> surfaces *realm itself* creates through `get_shell_surface`, which do get a
> `river_node_v1` *(verified)*. realm's bar is a separate `wlr-layer-shell`
> client (ADR 0008), and `river-layer-shell-v1` exposes **no node object and no
> ordering request at all** — only `non_exclusive_area`, `set_default` and the
> three focus events *(verified)*. So bar-over-fullscreen is river's wlr-layer
> semantics, decided by the layer the bar picks, and is **not** realm's to order.
> If M2 finds the ordering wrong, the lever is the bar's chosen layer, or
> revisiting ADR 0008; it is not a `place_*` call.

**Workarea.** `river_layer_shell_output_v1::non_exclusive_area(x, y, width,
height)` gives an arbitrary rect in **global** coordinates *(verified)*, and
`river_output_v1::position` / `dimensions` give the output's rect. So:

```rust
Workarea { output: Rect::new(ox, oy, ow, oh), tiles: Rect::new(x, y, width, height) }
```

> **Correction to `INTERFACES.md` §1**, which says the event "is literally
> `Workarea::new(w, h, top, bottom)`". It is not: `Workarea::new` models only a
> top and a bottom reserved strip, and `non_exclusive_area` is a free rectangle.
> For realm's own bar and which-key strip the two agree, and the struct's fields
> are public so the general case is expressible; but a left- or right-anchored
> layer surface from some other client cannot be expressed through
> `Workarea::new` and must be constructed field-wise.

realm must never compute the workarea from `metrics.bar_height` itself. The bar
declares its exclusive zone, river subtracts it, and realm learns the result.
Computing it locally is how realm and the bar come to disagree by 26 px the first
time someone toggles the which-key strip.

### 3. The four companion protocols Realm must use for MVP

Under River a window manager is not merely a client of the window-management
protocol. Four companion protocols are MVP obligations. Runtime layout
switching is explicitly deferred rather than counted as implemented.

**`river-layer-shell-v1` — the bar exists because of this.** Covered in §1
step 3 and in the workarea note above. One further correction:

> **Correction to ADR 0013.** Its launcher row lists
> `river_layer_shell_seat_v1::focus_exclusive` / `focus_non_exclusive` /
> `focus_none` as things realm calls. They are **events**, not requests
> *(verified)*. realm does not grant a launcher exclusive focus; hecate (fuzzel
> in M2) asks for keyboard interactivity through ordinary `wlr-layer-shell`,
> river decides, and realm is *told*. What realm owes each event is a response:
> on `focus_exclusive`, stop trying to set focus — "all window manager requests
> to change focus are ignored" until it clears *(verified)* — and clear the
> focused-window border and `RealmState::focused_title`; on `focus_none`, return
> focus to `Ledger::focused()`.

The compositor seam preserves that distinction: ordinary effective window
focus is `BackendPolicyEvent::FocusChanged`, while layer-shell exclusivity is
`BackendPolicyEvent::ExclusiveFocusChanged`; both arrive inside a policy turn.
Session reduces them in exact report order. `ExclusiveFocusChanged(true)` makes
the response carry no focused placement/border and makes finalized visible
`focused_title` empty. `ExclusiveFocusChanged(false)` makes the same response
restore focus and border from `Ledger::focused()` and restores that window's
title at finalization.

Ordinary `FocusChanged` reports compositor-effective focus; it never grants
policy authority or mutates ledger order/focus. Outside exclusive focus,
equality with `Ledger::focused()` acknowledges the clean focus component, while
a difference makes the sole response reassert ledger focus. During exclusive
focus it is recorded but creates no window-focus request. `FocusChanged(None)`
does not imply exclusivity. Any live/draining `Some` identity must already have
been opened/assigned in report order; initial replay may reference an earlier
accumulated open and defers assignment through its barrier. Both event kinds are answered in their offered
turn and become visible only at its final clean boundary.

**`river-xkb-bindings-v1` — the keymap does not exist until this is served.**
No `river_xkb_binding_v1` object means no keybinding fires at all; the desktop
is a mouse-only tiler.

**MVP input policy is deliberately small and complete.**
`river-input-management-v1` supplies the fixed 25 Hz / 600 ms application-key
repeat setting, while `river-libinput-config-v1` enables tap-to-click on every
device that reports positive tap support. Both wait for each v2 `done` boundary
before acting. Realm preserves every other device setting. In particular it
does not invent a scroll factor, seat assignment, acceleration profile, natural
scroll preference, or mapping policy. Runtime keyboard-layout switching and
`river-xkb-config-v1` are post-MVP; M2 preserves River's existing keyboard
keymap and layout rather than claiming a switch Realm does not yet expose.

- During initial replay, after selecting the first reported `river_seat_v1`,
  create its xkb-bindings-seat and layer-shell-seat objects exactly once, then
  for each pre-registered `Binding` in the `Keymap` call
  `river_xkb_bindings_v1::get_xkb_binding(seat, keysym, modifiers)`. This spec
  fixes `Binding::key` as an **xkbcommon keysym name**, resolved with
  `xkb_keysym_from_name`. Every value in `Keymap::default()` is one:
  `Return`, `d`, `b`, `j`, `k`, `h`, `l`, `1`–`6`, `s`, `m`, `f`, `r`, `q`,
  `t`, `u`, `w`, `question`, `p`, `Escape`.
- `modifiers` uses `river_seat_v1::modifiers`, whose values are a bitfield with
  `mod4 = 64` ("commonly called super or logo") *(verified)*. Bindings in
  `Mode::Nav` are created with `mod4`; bindings in a submap mode
  (`Mode::Resize`, `Mode::Move`) are created with `none = 0`, because in a
  submap the user presses a bare `h`, not `mod+h`.
- A binding is inert until `enable` is made in a manage sequence *(verified)*.
  **A mode is an enabled set.** On entering mode *M*, in one manage sequence,
  `disable` every binding whose `Binding::mode != M` and `enable` every binding
  whose `mode == M`. `Keymap::resolve(key, mode)` stays the authority on what is
  bound where; the backend only mirrors it.
- **Key events that trigger a binding are not delivered to the focused
  surface** *(verified)*, so the focused application never sees `mod+j`.

**The chord model.** `ensure_next_key_eaten` is purpose-built for this; its own
rationale names "chorded keybindings where triggering a binding activates a
submap" and the need "to know that it should error out and exit the submap when
a key not bound in the submap is pressed" *(verified, verbatim)*. realm's model
maps onto it exactly:

1. `mod+r` fires `Action::EnterMode(Mode::Resize)`. In the *same* manage
   sequence as the `pressed` event, realm swaps the enabled set as above and
   makes one `river_xkb_bindings_seat_v1::ensure_next_key_eaten`.
2. The next non-modifier key press is not delivered to the focused surface.
   - If it triggers an enabled binding, realm gets `pressed` on that binding,
     acts, and calls `ensure_next_key_eaten` again to stay in the submap.
   - If it does not, realm gets `ate_unbound_key`, and **leaves the submap**:
     mode back to `Mode::Nav`, Nav set re-enabled, `chord_echo` cleared, and
     no further `ensure_next_key_eaten`. The stray key is swallowed, not
     delivered — which is the point, since delivering a stray `x` into the
     user's editor is the failure this request exists to prevent.
3. `cancel_ensure_next_key_eaten` backs out a pending chord on a timeout. realm
   does **not** use a chord timeout in M2: the mode badge and chord echo make
   the pending state visible, `Escape` is bound to `EnterMode(Nav)`, and the
   protocol warns that a timeout needs a `manage_dirty` round trip and may race
   the `ate_unbound_key` event anyway *(verified)*.

Two honest notes on the current keymap. `Keymap::default()` has every binding
in `Mode::Nav`, so today `Mode::Resize` has an *empty* enabled set and the first
key pressed in it produces `ate_unbound_key` and exits immediately. That is a
gap in `realm-core`'s keymap, not in this design; the resize bindings are M2 work
in `realm-core` with their own test. And realm's chord model is presently one
modifier plus a mode rather than a multi-key prefix; a future `mod+g` then `w`
prefix uses the identical mechanism with a one-key submap.

**The mode badge and chord echo.** `modifiers_watch(mod4)` in a manage sequence
asks for `modifiers_update(old, new)` whenever `mod4` changes state *(verified)*,
which is how "hold `mod` to see the which-key strip" is implemented without
polling. Every such event opens a manage sequence, so holding a modifier costs
one round trip per press and one per release — bounded, and inside budget, but a
reason not to watch modifiers realm does not use.

**Key repeat is realm's job.** Because bound keys never reach a surface, river
does not repeat them; `river_xkb_binding_v1::stop_repeat` exists precisely to
tell a window manager "that has been repeating some action" to stop *(verified,
verbatim)*. The fixed MVP policy is `KEY_REPEAT_RATE_HZ = 25` and
`KEY_REPEAT_DELAY_MS = 600`. Realm sends rate 25 repeats/second and delay 600 ms
through `set_repeat_info`; the outer timerfd's first expiry is 600 ms after Arm
and its interval is exactly 40 ms. The protocol defines rate zero as disabled;
if a future typed configuration permits zero, Realm sends zero and emits
Disarm rather than constructing an interval. Realm owns that timer for held
bindings and disarms it on `released`
**or** `stop_repeat`. `BindingPressed` records physical held state but does not
arm or fire repeat before that turn reaches its final clean boundary. On
success, the timer arms only if the binding is still held and no
`BindingRepeatStopped` has arrived. `BindingReleased` and
`BindingRepeatStopped` are input-safety facts staged in report order. At a
successful final boundary they disarm the existing repeat timer before later
effects. If their admitted response fails, the whole session fails closed and
no timer directive or other ordinary update escapes. A failed press likewise
emits no staged repeat arm. This is a timer, and
[ADR 0009](../adr/0009-no-animation-budget.md) permits exactly one (the clock),
so it is justified explicitly: **the repeat timer is armed only between a
`pressed` and its `released`/`stop_repeat`, and never exists at idle.** Idle
CPU is unaffected.

Repeatability is explicit policy, not inferred by the backend. The
`realm_core::keys::Binding` contract gains `repeatable: bool`; the MVP default
marks only the directional Focus and Swap bindings repeatable and marks every
toggle, close, launch, mode, orbit, layout, undo, theme, grimoire, and Quit
binding non-repeatable. Session owns exactly one repeat target. When a
repeatable press reaches a successful final clean boundary, its resolved
binding identity and action replace and restart that target. Release or
`BindingRepeatStopped` disarms only when its id names the current target; a
noncurrent release cannot kill the target, and releasing the replacement never
resumes an older binding that remains held. A finalized mode change that removes
the current binding from the committed enabled set disarms it.

The timer calls `Session::fire_key_repeat()`. It first rechecks that the exact
captured binding is still held, armed, configured, repeatable, and enabled by
committed binding policy; otherwise it returns an unchanged update and consumes
no ticket. A fire while any external, internal-repeat, response, or drain
transaction is active is likewise consumed as an unchanged
update: it retains the target but allocates no ticket and requests no second
turn, so a later scheduled tick may retry after Session becomes Idle. A
repeated action that is a local no-op or needs only a local/effect update
finalizes synchronously. If it needs a compositor policy response, Session
reserves a fresh response ticket, enters `AwaitingInternalTurn`, and calls
error-atomic `request_policy_turn`.
This internal origin never sets `pending_action` and never produces an
`ActionCompletion`. Any request error or post-admission terminal failure on
this requesterless path is fatal in the MVP and emits no diagnostic or effect.
Backend event service has priority over a
simultaneously ready repeat timer, and `fire_key_repeat()` rechecks held state,
so a release or repeat-stop already observed cannot race into a new request.

Every `SessionUpdate` carries a closed immediate `RepeatTimerDirective` before
its persistence/state/result/effects: `Preserve`, `Arm { delay, interval }`, or
`Disarm`. The outer loop applies it before every other update field. Arm is an
edge, not a state getter: every successful finalized repeatable press emits Arm
and therefore restarts the timer even when its id equals the previous target.
Only a successful final boundary may Arm. Current-target release/stop emits
Disarm at its successful final boundary; a finalized mode change that disables
the target and every transition to
QuitPending/ShuttingDown also emits Disarm. Preserve is the unchanged default.

**`river-input-management-v1`.** Enumerate `input_device`s, wait for `done`
(v2) so a multi-event device description is seen atomically, and apply exactly
the fixed 25 Hz repeat rate and 600 ms delay to keyboard devices. No manage
sequence is involved. Devices stay on the existing `default` seat; Realm sends
no scroll-factor, mapping, seat-creation, or seat-assignment request.

### 4. Liveness — the sharpest constraint in the design

The mechanism, stated three times in the XML *(verified, verbatim)*: "The
compositor should wait for the manage sequence to complete before processing
further input events. … The window manager should of course respond as soon as
possible as the capacity of the compositor to buffer incoming input events is
finite." River v0.4.8 does not impose a window-manager response timeout; its
finite seat queue makes the consequence of a sustained stall dropped input
rather than a protocol disconnect.

So: **input stops while realm thinks.** A stall is not a slow frame, it is a dead
session, and the frame budgets in
[ARCHITECTURE.md §4](../ARCHITECTURE.md) stop being comfort targets.

**Forbidden between `manage_start` and `manage_finish`, and between
`render_start` and `render_finish`:**

- any blocking `write(2)` to a control-socket client, however small;
- any filesystem access — no palette read, no template render, no
  `realm_theme::apply`, no ledger snapshot write, no `stat` of a spawn target;
- any process creation or `waitpid`;
- any D-Bus call, DNS lookup or network I/O;
- any mutex acquisition (there are none — see below);
- `RealmState` derivation, JSON encoding and fan-out, which are cheap but are
  not needed before `manage_finish` and therefore must not precede it.

**The specified architecture: one event-loop thread, one worker thread, no
shared mutable state.**

- The **event loop** thread owns the river connection, the `Ledger`, the
  `Keymap`, the mode machine, the last-applied backend state, the control-socket
  listener and every client connection. Every fd is non-blocking and the loop is
  a single `poll(2)` over: river's Wayland fd, the listener, each client fd, a
  `timerfd` for the clock, an armed-only-when-held `timerfd` for key repeat, and
  an `eventfd` the worker signals.
- The **worker** thread owns nothing and receives process jobs over a bounded
  `MAX_WORKER_JOBS = 256` FIFO plus one separate replaceable snapshot slot.
  The snapshot slot does not consume process-job capacity, so a due snapshot is
  accepted even when all 256 process slots are reserved or committed; total
  physical input retention is therefore 257 entries. A newer immutable
  snapshot supersedes only an older not-started snapshot job. Process effects
  never coalesce or drop during live operation. Before queuing an ordinary
  requester response, the adapter nonblockingly reserves the exact number of
  process-effect slots released at that boundary; failure is fatal
  `WorkerCapacityError { resource: Jobs, limit: 256 }` and queues neither
  response nor partial jobs. A
  reservation is not visible to the worker. After the requester completion
  attempt, it is committed both on a queued response and on the peer-local
  close-proving `StaleConnection`, `ResponseSequenceExhausted`, or
  `OutboundFrameTooLarge` outcomes, so unrelated pre-Quit effects are not lost;
  other errors are fatal and do not commit it. With no requester it is committed
  after persistence/publication. The worker returns results through a bounded
  `MAX_WORKER_RESULTS = 256` queue and signals the `eventfd`. Results never
  coalesce or drop; a full result queue may block only the worker, never the
  event loop. The event loop consumes at most one result per cycle and retains
  immediate interest while more remain. Maximum and maximum-plus-one behavior
  for both queues is tested. Future D-Bus work must choose an explicit
  coalescing/admission rule before entering this mailbox.

  At the adapter's first observation of the terminal `QuitPending` boundary,
  after all pre-Quit effect reservations have committed, explicit logout calls
  `PersistenceCoordinator::shutdown_snapshot()`, places the latest dirty value
  in the dedicated snapshot slot, and seals the worker with the same captured
  monotonic `now`. This happens before any requester response-barrier wait or
  control shutdown drain. Sealing admits no new
  jobs and sets one sealed control flag outside both the 256-entry process FIFO
  and the snapshot slot. That flag cannot be capacity-rejected. Its ordered
  fence acknowledgement means every pre-seal process job and the final snapshot
  reached a terminal worker result. The
  event loop continues bounded eventfd/result service and may call
  `Session::begin_exit_session()` only after both the control drain and this
  fence settle. It never blocks synchronously on the worker. The first seal at
  captured monotonic `now` fixes `deadline = now + 2,000 ms`; repeated sealing
  is idempotent and cannot slide it, and expiry is exactly `now >= deadline`.
  Each job error is one terminal result and does not release the fence early;
  the worker continues every later pre-seal job. Fence acknowledgement after
  any error records an explicit degraded-shutdown diagnostic and permits
  compositor exit, as does deadline expiry even if work remains; the
  guarantee is therefore no silent loss, not survival of a failed/hung worker.
  A successful clean shutdown has acknowledged the final snapshot write and
  every pre-Quit process job. Maximum-capacity Spawn followed by Quit cannot
  overtake or omit the fence.
- **No locks.** A single owner means the input path cannot contend, which
  matters because a lock held for 40 ms is a stall nobody sees in review.

Each #38 combined-loop cycle captures one readiness snapshot and performs these
bounded steps in order:

1. service at most one eligible backend quantum from immediate work
   or current readable/terminal/writable readiness, then recompute backend
   interest. If already-admitted work keeps `poll_interest().immediate` true,
   begin the next cycle without consuming any timer, worker, or control
   readiness. This lets one bounded ingress epoch cross protocol dispatch and
   public dequeue before a received release can be overtaken by repeat. Merely
   observing fresh level-readable bytes in a zero-poll does not extend that
   burst: after the admitted epoch is quiescent, timers/worker/control each get
   their bounded opportunity before a later cycle admits another ingress unit;
2. consume at most one key-repeat `timerfd` readiness. One `read` consumes the
   kernel overrun count and coalesces all expirations into one attempt. If a
   transaction is active the attempt is the unchanged no-ticket/no-request
   result in §3; missed counts are never replayed, and only a newly scheduled
   later tick may fire after Idle;
3. consume at most one clock `timerfd` readiness, coalescing its overrun count
   into one latest-value recomputation. During an active transaction it records
   one bounded dirty flag and defers that recomputation until finalization;
4. consume at most one worker completion. A single `eventfd` read may represent
   several queued completions, so a bounded in-memory result queue keeps
   immediate interest until later cycles process the rest one at a time;
5. service at most one SPEC 0007 control quantum; and
6. zero-poll and recheck backend interest/readiness before another control
   quantum.

Backend input therefore preempts timers, workers, and control until its complete
bounded ingress/dispatch/dequeue handoff is quiescent; a release or repeat-stop
received before a simultaneous timer is reduced first and disarms the target.
Timer overruns never become an unbounded catch-up loop. While `QuitPending`, the
driver first installs SPEC 0007's exact response barrier and includes no
listener or peer read interest; it services only that receipt's
writable/terminal progress and expiry until Drained/Closed, then immediately
begins shutdown. The listener/client portion of the normal order exists only
after `Live`; the pre-listener recovery pump uses the restricted order stated
in §1.

If a backend quantum returns no event while immediate interest remains, the next
cycle services the backend again before control. The loop captures one
`std::time::Instant` per cycle for every transport call. The transport never
drains a ready fd to `EAGAIN`, and the MVP adds no token bucket or audit-only
rate limiter.

*Why not an async runtime.* It would give the same non-blocking behaviour, and
is a defensible alternative. It loses on two counts. First, the committed seam
is an explicit poll-driven state machine, so a second scheduler would duplicate
rather than simplify readiness ownership. Second, a work-stealing scheduler puts a fairness policy
we did not write between a key press and `manage_finish`, and the budget it must
hold is 4 ms. Two threads with one owner each is boring, and boring is what the
most conservative code in the tree should be
([ADR 0003](../adr/0003-session-daemon-owns-state.md) Consequences).

**The `WmBackend` poll contract.** The event loop polls River's descriptor
alongside the sockets, but readability alone is insufficient: a nonblocking
Wayland flush may need `POLLOUT`, and dispatch may leave a public event queued
after the raw fd has been drained. The backend therefore exposes both its
interest and one bounded, nonblocking service quantum:

```rust
fn event_fd(&self) -> std::os::fd::BorrowedFd<'_>;
fn poll_interest(&self) -> BackendPollInterest;
fn service(
    &mut self,
    ready: BackendReady,
    now: std::time::Instant,
) -> BackendResult<Option<BackendEvent>>;
```

`BackendPollInterest::immediate` skips blocking poll and is true for already
queued protocol/public work and whenever no prepared-read guard exists and one
preparation attempt is eligible. A successful preparation clears that latter
reason; preparation returning queued work or no guard is progress and leaves
another immediate quantum eligible. `readable` adds `POLLIN` and remains true
until the exit cutoff; `writable` adds `POLLOUT`. `POLLNVAL` is an immediate
fatal backend-incarnation error and is never mapped to ordinary readability.
The outer poll maps `POLLIN` and `POLLERR | POLLHUP` separately into
`BackendReady::readable` and `BackendReady::terminal`, so a live read guard is
consumed while post-cutoff readable payload cannot masquerade as terminal
completion. `service` performs at most one
bounded quantum, never blocks, and returns at most one public event. It may make
protocol progress and return `None`. The outer loop keeps calling it without a
blocking poll while `immediate` remains true. `NativeBackend` at M5 can return an
`eventfd`. `INTERFACES.md` §1 is updated in the same commit as this contract.

**Ordering rule.** Session derives the policy response while River waits at the
offered turn. The backend issues and flushes `manage_finish` and every required
`render_finish` *before* Session commits or publishes `RealmState`, starts a
process effect, begins shutdown, or writes a byte to any subscriber. This is the
single rule that makes a wedged `realmctl` unable to wedge the desktop.

**Transactional desired state.** The session stages user-requested ledger and
window-metadata changes, requests a policy turn, and commits them only after
the required policy response and its retained-observation/follow-up chain reach
a clean final boundary. `BackendSubmission::Complete` is such a boundary because
it promises no later completion or retained observations; returning `Pending`
or receiving its terminal success is not. An application error leaves the
committed authoritative ledger, Session's last-committed-clean projection,
visible `RealmState`, and revision unchanged. A pending desired transaction
tracks its most recently completed projection separately and privately so a
drain can suppress an unnecessary follow-up without exposing an intermediate
commit. The existing `last_projection()` accessor means the committed-clean
projection and therefore stays at P1 throughout A36 until finalization.
For the MVP, every compositor, input, desired, key-policy, and effect delta in
an admitted transaction stays in private working state W until the one final
clean boundary. There is no observed-only early-authority exception. This is
deliberately stricter than the eventual recovery design: a crash before the
boundary may lose an observation from the current process, but River replay in
the replacement incarnation restores compositor authority without ever
persisting or publishing a half-applied transaction.

**MVP failure cut.** Once a turn has been admitted and its response accepted,
any matching terminal `Io` or `Unsupported` is fatal for the current session,
as are `Protocol`, `Capacity`, `Disconnected`, and `Unavailable`. Session
discards W and the origin, emits no ordinary `ActionCompletion`, state,
persistence, diagnostic, or effect, and performs no repair request. The outer
driver terminates the incarnation and supervised startup learns authoritative
facts again through replay. This fail-closed rule also covers a late failure of
a tagged follow-up response. SPEC 0007 separately guarantees that no rejected
transaction produces a state frame.

**Single flight and observation ordering.** `Session` allocates opaque,
strictly increasing nonzero `BackendTicket`s and never reuses one in a backend
incarnation. Exhausting `u64` is `BackendTicketExhausted`: the attempted desired
action returns `SessionActionError::BackendTicketExhausted`, an observed or
replay transition returns `SessionEventError::BackendTicketExhausted`, the
outer driver terminates the current session, and no zero, wrapped, or reused
ticket is submitted. A clean no-op is detected before allocation. At most one
policy response is in flight, and its single-flight gate remains
active through the retained-observation drain described below. While gated,
desired operations and another close are `NotReady`, but backend reads and
writes continue so protocol work can finish.

River likewise allocates strictly increasing nonzero policy-turn ids. Its id
field has a public checked constructor for backend implementors. Turn-id
exhaustion is `BackendError::Capacity { resource: PolicyTurnIds, ... }`, exposes
no partial turn, terminates the incarnation, and never emits zero or wraps.

An asynchronous external desired or close action is identified by the ticket
reserved before its first `request_policy_turn`. The accepted request returns a
non-final update with `pending_action = Some(origin_ticket)` while Session waits
for River to offer the turn. That origin ticket is used for the first response,
and `ActionCompletion::ticket` repeats it even when later tagged follow-up
responses use fresh tickets. `request_policy_turn` is idempotent and
error-atomic: an error means no turn request was registered. Its `Io` or
`Unsupported` result is therefore an immediate application Error with no dirty
cache, pending action, response, or drain; `Disconnected` or `Unavailable` is
fatal. Success guarantees the request is flushed or keeps immediate/writable
poll interest asserted until bounded service flushes it, after which readable
interest awaits the untagged turn. The reserved origin number is consumed
even when the error-atomic request fails and is never reused. Once a turn is
offered, an error from `respond_policy_turn` is fatal for
every class and produces no ordinary action completion because River may remain
blocked in the open sequence.

After every `Pending` response, the backend emits exactly one matching
`OperationCompleted`. A matching `Ok` seals retained facts
exactly once: either one `PolicyTurn` whose `drains` field names that completed
ticket, or one empty `RetainedObservationsDrained` marker. The two forms are
mutually exclusive. A draining policy turn contains the complete bounded,
report-order fact/input batch and simultaneously opens the next mandatory
response boundary; Session answers it with a fresh response ticket. A
completion reporting any error, a standalone `Disconnected`, or a
service/respond error terminates the incarnation,
abandons the drain, and neither requires nor permits a later marker. Retention
is bounded by the current managed-window set plus fixed focus/workarea/input
state. The named pre-authority normalization exception permits a backend-local
open followed by close to cancel only when neither fact has been exposed; that
object never receives Realm identity or advances its watermark. Surviving opens
retain report order; latest title, focus, workarea, and modifier facts win only
when full report-order reduction remains equivalent. An untagged policy turn while a
completed response awaits its drain, or an unknown, stale, duplicate, or
out-of-order completion, drain tag, or marker, is a backend contract failure.

Matching success records that the response reached the backend but does not yet
commit, publish, run effects, or complete its action. At a draining policy turn,
Session reduces all events into that private state, derives the complete
binding/close/projection response from current facts, and answers with a fresh
ticket; it performs no intermediate publication, persistence, process effect,
shutdown transition, or action completion. An empty matching marker states only
that no facts were retained and is the clean final boundary. The original
action result remains retained through every successful follow-up completion
and drain. A final clean boundary is either that empty matching marker or
`BackendSubmission::Complete` returned by the initial or a tagged draining-turn
response. It atomically installs working authority, installs the final
clean projection/binding state, creates any visible publication and persistence
authority, and co-releases the original `ActionCompletion`; no marker follows a
`Complete` result. Task 3 proves that none of those `SessionUpdate` fields is
exposed early and that the effect vector and `QuitAfter` value are ordered. #38
alone proves temporal consumption as persistence, publication, requester
response, then effects. The transaction retains one
report-ordered effect list whose total length may not exceed 256 across the
initial and every tagged follow-up turn; overflow is fatal before the offending
turn is answered or any partial effect is retained. Ordinary process effects
before the first Quit are enqueued in report order only after that response.
The first Quit becomes a sticky terminal barrier; later derived actions/effects
in the same or any later tagged turn are suppressed, although later
authoritative compositor and physical-input facts continue to reduce. Quit is
stricter: finalization enters
`QuitPending` and stops new admission/actions. With an original requester, the
adapter waits until that response frame is fully written or its connection is
closed, then calls `begin_shutdown`; without a requester, it calls
`begin_shutdown` immediately after persistence/publication at the final
boundary. Thus a successful mixed external action plus Quit never lets
`begin_shutdown` close the requester before the promised response drains. A
failed follow-up abandons private state, terminates the session, and produces no
ordinary action completion. Initial replay remains
`FinalizingReplay` until this same policy-response/drain chain reaches its final
boundary.

A retained `WindowOpened` is reduced into private identity, metadata, ledger,
and watermark state. Before any marker-derived projection may name it, Session
must call local `assign_window` for that identity. Assignment is bounded,
nonblocking, and emits no compositor request. Any assignment error is a fatal
backend-contract violation because River is waiting for the current policy
response; it abandons the incarnation. The
final clean boundary commits the new mapping and watermark with the rest of the
transaction. If the backend's bounded
retention helper cancels an open/close pair before exposing either event,
Session sees neither and consumes no Realm id; once `WindowOpened` is exposed,
the normal never-reuse watermark rule applies even if a later retained close
removes it before finalization.

A drain boundary seals only facts already received by the backend. A window may
still disappear after an empty marker and before the next offered turn, or
during a pending response. The backend may omit that placement only when the exact identity
was successfully assigned in this backend incarnation and then concurrently
tombstoned; it retains exactly one corresponding `WindowClosed` behind the new
pending operation. A placement for an identity never assigned in this
incarnation remains a contract failure. The later close observation remains
authoritative and may cause one further projection. This narrow rule prevents a
normal client close from invalidating an otherwise successful drain without
hiding an invalid Session projection.

Operation failure classification is closed and phase-sensitive:

| Failure boundary | Result |
|---|---|
| External desired/close `request_policy_turn` returns `Io` or `Unsupported` | The error-atomic request registered no turn. Reject the staged intent and return its application Error immediately; no cache is dirty and no pending action, response, or drain exists. |
| Internal-repeat `request_policy_turn` returns `Io` or `Unsupported` | The error-atomic request registered no turn, but there is no requester to receive an application error. Fail the session closed without an ordinary completion/effect and retain the consumed ticket watermark. |
| Any `request_policy_turn` returns `Disconnected` or `Unavailable` | Fatal backend loss; abandon staged intent without an ordinary action completion. |
| Any `request_policy_turn` returns `Protocol` or `Capacity` | Fatal contract/capacity failure; abandon staged intent without a pending action, ordinary completion, or drain. Never wrap it as an application Error. |
| Any `respond_policy_turn` error | Fatal for every class because a River sequence is open and may be unanswered; abandon private state and emit no ordinary action completion. |
| Any post-admission matching terminal `Io` or `Unsupported` | Fatal fail-closed MVP boundary. Discard every private desired/key/effect/observation delta and the origin; emit no ordinary completion, persistence, publication, diagnostic, or effect; request no repair. |
| Any matching terminal `Disconnected` or `Unavailable` | Fatal backend loss; abandon candidate, ticket, and drain without an ordinary completion. |
| Any matching terminal `Protocol` or `Capacity` | Fatal contract/capacity failure; abandon candidate, ticket, and drain without an ordinary completion. |
| Any `service()` error or standalone `Disconnected` | The backend incarnation is unusable; abandon the active turn/transaction and fail fatally. |

No post-admission error is reclassified as an application Error in the MVP. A
backend that reports a pending response failure does so in the matching
`OperationCompleted`; returning an error from `service` or
`respond_policy_turn` likewise declares the incarnation unusable.

**Explicit post-MVP transaction-recovery scope.** A later Accepted amendment may
replace fail-closed restart with one bounded in-incarnation repair. That work is
not an M2/M3 launch gate and includes `RetryReady`/`AwaitingRepairTurn`, one
repair allowance, repair tickets and second-failure rules, rollback-to-continue,
observed-only authority promotion before the final drain, uncertain next-key
edge repair, nonfatal post-admission diagnostics, exhaustive direct-Quit
substate transmutation, and QuitPending/ShuttingDown discard sequencing.

That future recovery must use **component provenance**, not whole-batch
provenance. Private transaction metadata records why each attempted projection,
binding, next-key, or close component was included. A failed `Unsupported`
component is authoritative only when replay or a compositor/default-output fact
required that same component; an unrelated authoritative observation such as a
title change does not poison a desired-only projection failure. If desired and
authoritative causes jointly require one component, authoritative provenance
wins for that component. The deferred Session evidence is
`desired_projection_unsupported_with_unrelated_title_observation_is_repairable`,
`desired_projection_unsupported_with_authoritative_workarea_need_is_fatal`, and
`desired_binding_unsupported_with_authoritative_binding_need_is_fatal`. Until
that follow-up is accepted and implemented, the MVP terminal rule above is
unconditional fatal fail-closed behavior.

`BackendPolicyEvent::WindowOpened` is an observed lifecycle fact before identity
assignment is attempted. The session first allocates or restores the stable
`BackendWindowId` mapping, advances the non-reuse watermark when needed, and
records the window in the ledger and metadata. It then calls
`WmBackend::assign_window` before any projection containing that window. An
assignment error is a fatal backend-contract failure: the method is local and
nonblocking, and retrying it while River waits for the offered turn could
deadlock input. `assign_window` is idempotent; repeating the same identity/id
pair is a no-op, while an unknown object or conflicting pair terminates the
incarnation without publishing the private mapping. Replaying the same
backend identifier within one connected backend incarnation reuses the recorded
`WinId` and never advances the watermark or binds twice. Snapshot recovery
preloads identity knowledge but marks every restored mapping unbound in the new
backend incarnation, so each replayed identity is assigned again before use.

The session exposes typed desired-state operations, not an arbitrary mutable
`Ledger` closure. Lifecycle events are the only path that adds or removes a
window. Before any desired projection is included in a policy response, the ledger's window set,
the window-metadata keys, and the values of the stable identity map must be the
same set. This makes it impossible to send a placement for a `WinId` that has
not been observed and successfully assigned.
Typed desired operations and `request_close_focused()` return a
`SessionActionError`: local pre-Live or pending-work refusal is
`SessionActionError::NotReady`, while compositor failures are wrapped as
`SessionActionError::Backend(BackendError)`. `BackendError::Unavailable` is
reserved for a backend that cannot be or remain the active window manager and
must never represent local session readiness.
`fire_key_repeat() -> Result<SessionUpdate, SessionEventError>` is the separate
timer-origin operation defined above; an internally requested turn consumes a
response ticket but exposes neither an external pending ticket nor an action
completion.

`BackendPolicyEvent::GeometryDrifted` is advisory and does not itself bypass
projection deduplication. The ledger and last-successful-projection cache remain
unchanged; the next genuine ledger or workarea projection supersedes the drift.
The backend that reports drift must therefore invalidate its own per-window
request cache so that applying that next changed projection restores every
affected field.

Focus is therefore fully owned by the generic policy-turn reducer; no focus
event is deferred outside its offered response boundary.

### 5. The dimension-proposal problem

`propose_dimensions` is a proposal: "The window may not take the exact
dimensions proposed. … For example, a terminal emulator may only allow
dimensions that are multiple of the cell size" *(verified, verbatim)*. A
terminal that rounds 700×580 down to 696×576 puts a 4×4 hole in a layout whose
entire premise is exact tiling, and
`layout::tests::every_layout_tiles_exactly_for_every_plausible_size` passes
happily throughout, because it tests the projection and not what the client did
with it.

The mitigation, concretely enough to implement and to test:

1. Project. `Placement::rect` is the tile, and remains the truth.
2. Manage sequence: `propose_dimensions(rect.w, rect.h)`.
3. The `dimensions(w, h)` event arrives before `render_start` *(verified)*.
   - **Exact** (`w == rect.w && h == rect.h`): if content clipping is enabled
     for this window, disable it with `set_content_clip_box(0, 0, 0, 0)` — a
     zero width or height disables clipping *(verified)*. Otherwise do nothing.
   - **Short** (`w < rect.w` or `h < rect.h`): record the shortfall
     `(dw, dh) = (rect.w - w, rect.h - h)` against this window, and in the next
     manage sequence re-propose `(rect.w + dw, rect.h + dh)`. For a client
     quantising to a cell, the shortfall is strictly less than one cell, so the
     bumped proposal rounds *up* past the tile. One corrective round, not a
     loop.
   - **Over** (`w >= rect.w && h >= rect.h`, at least one strictly greater):
     `set_content_clip_box(0, 0, rect.w, rect.h)` in the render sequence. Borders
     "are placed around the intersection of the window content … and the content
     clip box" *(verified, verbatim)*, so the 1 px seams land exactly where the
     projection put them and the visible rectangle is the tile.
4. **Give up once, loudly, and never oscillate.** If the corrected proposal
   still comes back short, realm accepts the reported dimensions, logs once per
   window and tile size, and leaves the gap. It does not propose a third time.
   An unbounded correction loop against a client with an unusual size policy is
   a stall, and a stall is a session failure (§4).
5. Cache `(tile size → accepted proposal)` per window, so returning to a
   previously used tile size costs one round, not two.
6. Fullscreen ignores both clip boxes *(verified)*, so no clipping is applied to
   a fullscreen window.

**This is an experiment and is written down as one.** What it cannot do is make
the client *draw* the clipped region: a terminal whose last text row is clipped
shows a partial row, or nothing, depending on how it renders its background. The
open question is not whether the geometry is exact — it is, by construction —
but whether the result reads as a desktop or as a bug. That is empirical, it is
an M2 acceptance judgement, and if the answer is "it reads as a bug" the
fallback is to accept the client's size and let the seam sit inside the tile,
which is what every other Wayland tiler does. `set_content_clip_box` requires
`river_window_v1` at version ≥ 3 *(verified)*; below that there is no mitigation
at all, which is one reason §1 refuses below 4.

### 6. Crash and restart

A dead `realm-session` leaves windows unplaced and keys dead — a sharper failure
than a crashed bar, and one the user cannot work around. `realm-wm.service`
uses SPEC 0012's `Restart=always`, `RestartSec=1` and a five-in-thirty-seconds
start limit; this section specifies the accepted M2 ledger state a restart
would restore.

**`WinId` is not river's identifier.**

> **Correction to ADR 0013 and `INTERFACES.md` §1**, both of which map `WinId`
> onto `river_window_v1::identifier` and call it faithful. `WinId` is
> `pub struct WinId(pub u64)`; `identifier` is "a string that contains up to 32
> printable ASCII bytes" *(verified)*. They are different types. `realm-session`
> therefore maintains a bijection and allocates `WinId`s from a monotonic
> counter whose next value is persisted and never reused within or across a
> session. `BackendPolicyEvent::WindowOpened` carries the compositor's stable
> `BackendWindowId`, not a guessed `WinId`; realm restores or allocates the
> numeric id and calls `WmBackend::assign_window` before the window can appear
> in another backend request. The
> *non-reuse property* ADR 0013 relies on is real and does carry over — but it
> comes from realm's counter, backed by river's guarantee that the identifier it
> is keyed on never repeats *(verified: "The identifier must not be reused. This
> avoids races around window creation/destruction when identifiers are used in
> out-of-band IPC")*.

**The snapshot.** Written to `$XDG_RUNTIME_DIR/realm/ledger.json` — runtime
state, not configuration, because a ledger from last week's boot is worse than
none. Written by the worker thread, never the event loop, using the same
temp-file-plus-`rename(2)` discipline as [SPEC 0002](0002-theme-pipeline.md). It
contains the serialised `Ledger`, the `WinId → identifier` map, the next
unallocated `WinId` watermark, the active orbit, and `PROTOCOL_VERSION` as a
schema guard. A snapshot whose version does not match is discarded, not
migrated.

The closed `SessionSnapshotV1` DTO has exactly these fields: `schema_version`,
`protocol_version`, `ledger`, `bindings`, `next_win_id`, and `active_orbit`.
Snapshot schema version 1 is independent of the control-wire version.
`bindings` is a `WinId`-sorted array of `{ win_id, backend_id }` records rather
than a JSON object with numeric keys. Unknown, missing, duplicate, or
out-of-order fields or bindings are malformed. Loading succeeds only when all
of the following semantic checks pass:

- the schema version is 1, the protocol version equals `PROTOCOL_VERSION`, and
  `active_orbit` equals the ledger's active orbit;
- the ledger contains exactly the six canonical orbit ids in order; every
  `WinId` occurs in exactly one orbit; each focus index is in range or absent
  exactly when that orbit is empty; each stowed id is a unique member of that
  orbit; fullscreen is absent or names a member of that orbit; and orbit names
  equal the six canonical names;
- the bindings are a bijection covering exactly the ledger's windows, every
  backend id is 1–32 printable ASCII bytes, and no backend id or `WinId` is
  repeated; and
- every allocated id is strictly below `next_win_id`. `u64::MAX` is the
  exhausted watermark sentinel and is never itself allocated.

An absent snapshot starts fresh. A version mismatch, malformed JSON, or a
semantic validation failure is reported and starts fresh without using any
part of the record; the next successful live-state write replaces it. Any
snapshot read error other than `NotFound` is fatal rather than guessed around.
The pathname worker reads at most `MAX_SNAPSHOT_BYTES + 1` bytes (65,536),
retains at most `MAX_SNAPSHOT_BYTES` (65,535), and classifies an observed extra
byte as a rejected oversized record. It never sizes an allocation from
untrusted file metadata and remains bounded if the file grows during the read.
The pure load classifier accepts an already completed `io::Result<Vec<u8>>`:
`NotFound` becomes `Fresh`, valid bytes become `Recovered`, invalid bytes become
`Rejected` while retaining the reportable validation error, and every other I/O
error remains fatal. It performs no environment lookup, pathname resolution,
file open, or write. The worker/file adapter belongs to the event-loop binary
slice.

**Undo history does not survive a restart.** `Ledger`'s `history` and `redo`
fields are `#[serde(skip)]` *(verified in `crates/realm-core/src/ledger.rs`)*, so
a round trip through JSON restores window order, focus, stow lists, per-orbit
layouts, fullscreen and the active orbit — and drops the undo stack. This is
correct rather than merely tolerable: replaying `mod+u` into a world whose
windows have changed underneath would restore a ledger referring to windows that
no longer exist. Say so in the release notes rather than pretending.

**Recovery.** On start, if a valid snapshot exists, load it and then reconcile
against what river reports:

| Case | Action |
|---|---|
| Snapshot identifier reappears | Restore the window to its recorded orbit, ledger index, stow state and `WinId` |
| Snapshot identifier does not reappear | Drop it; the window closed while realm was dead |
| River reports a window not in the snapshot | Allocate the persisted next `WinId`, advance the watermark, and `Ledger::summon` it into the active orbit, in river's report order |

`Ledger::summon` inserts after the focused window, so the result is
deterministic given a deterministic report order.
When replay introduces no new identity, the persisted focus is restored.
Otherwise ordinary summon semantics apply in report order, so the last newly
summoned identity is focused.
`ledger::tests::summoning_a_known_window_twice_is_ignored` means the reconcile
pass is safe to run more than once.
After reconciliation, realm explicitly clears undo and redo history before it
publishes state. Deserialisation starts with empty history, but the removal and
summon operations used to reconcile with live windows must not become actions a
user can undo.

Recovery is an explicit three-phase state machine:

1. **`InitialReplay`.** Every connection enters this phase, with or without a
   snapshot. `WindowOpened` records are accumulated in backend report order;
   the first occurrence fixes order and the latest metadata wins. A
   snapshotted identity reserves its recorded `WinId`; a new identity consumes
   nothing yet. Session begins with no authoritative workarea. Window-open and
   latest ordinary/exclusive-focus observations update only the replay
   accumulator. Ordinary focused identities
   must follow their `WindowOpened` in report order and never choose ledger
   policy. Realm emits no visible state, assigns no identity, applies no
   projection, accepts no desired mutation, and produces no persistence record
   in this phase. The River adapter coalesces raw child events into its first
   `PolicyTurn` and includes `WindowOpened` only for complete identities still
   live at that boundary. Before the barrier, Session accepts only
   `WindowOpened`, `FocusChanged`, `ExclusiveFocusChanged`, and the one
   `InitialReplayComplete` item in that ordered turn. A `WorkareaChanged` in
   this first turn is a typed fatal replay error: the required layer-shell
   output object cannot exist until River has reported its output in this turn
   and the backend has requested the dependent object. `TitleChanged` and
   `WindowClosed` must already have been
   coalesced into the live open set; geometry drift or input is a backend
   protocol error. The bootstrap replay response contains no placement or
   visible state. When the later workarea response projects, accumulated
   exclusive focus suppresses focused placement and the revision-1
   `focused_title`; otherwise it reasserts reconciled ledger focus.
   `Disconnected` remains the only standalone event and is fatal.
2. **`FinalizingReplay`.** The backend includes the explicit
   `BackendPolicyEvent::InitialReplayComplete` barrier exactly once as the final
   item of the first policy turn per backend incarnation; an earlier placement
   in that batch or a second barrier is a protocol failure.
   Realm stages the whole
   reconciliation before changing authoritative state or calling the backend.
   It preflights id capacity for every unknown identity, traverses snapshot
   orbits and window order to remove missing windows deterministically, then
   allocates and summons new windows in backend report order into the resulting
   active orbit. Only after staging succeeds does it retain the result and
   cleared undo/redo privately, bind each live identity exactly once for this
   backend incarnation, then answer that offered turn with no projection, no
   closes, every binding disabled, and next-key Preserve. The answer's clean
   final boundary installs the staged authority; this is a required bootstrap
   response, not a clean visible projection. If the response is pending, the phase
   remains `FinalizingReplay` and backend service continues through its matching
   terminal event and exactly one tagged draining policy turn or empty drain
   marker. A draining turn is reduced and answered without intermediate
   publication. If that tagged turn contains the first selected-output
   `WorkareaChanged`, its answer is the forced complete projection and desired
   binding state; Live still waits for that answer's own clean boundary.
   Staging failure retains no partial mapping, watermark, ledger, backend call,
   publication, or persistence effect. Completing this bootstrap response does
   not enter Live. FinalizingReplay then accepts ordinary authoritative
   window/title/focus/exclusive/workarea facts and answers every turn, but it
   rejects binding input and exposes no desired/control mutation. Before the
   first selected-output `WorkareaChanged`, each response still carries no
   projection and keeps bindings disabled. The first such workarea establishes
   authority and requires a forced complete projection, including `Some(empty)`
   for an empty ledger, plus the complete desired binding state. A response
   returning `Complete` is the immediate final clean boundary; otherwise its
   matching completion and tagged drain/follow-up chain must finish. Only that
   workarea-bearing projection's final clean boundary moves the session to
   `Live` and publishes exactly one initial `RealmState` at revision 1
   containing all facts accumulated since replay. If the backend loses its last
   output first, startup terminates without publication/readiness.
   A Pending bootstrap or workarea-projection terminal `Io`/`Unsupported`
   terminates startup without publication or readiness. A Session fake proves
   that fail-closed boundary; #40 separately owns backend-private
   selected-default behavior.
3. **`Live`.** Ordinary observed and desired transitions use §§4 and 9. A
   persistence record can be produced only here and always reflects the
   final-clean committed ledger/mapping/watermark. It never contains a private
   or failed candidate.

Assignment failures and every post-admission backend error are fatal. There is
no retry-ready Session state in the MVP. `has_active_backend_transaction()`
remains true while waiting for an external or internal-repeat turn and from a
`Pending` response through its matching successful completion and drain
boundary. While true, the loop continues servicing backend read, write, and
immediate work. Whenever it is true, every desired ledger operation and close request is
rejected as `SessionActionError::NotReady` without a backend call, and
non-fallible which-key or module updates are not admitted and return unchanged
without publication. Timer/worker readiness stays bounded at its source and
recomputes after finalization. `FinalizingReplay` exposes no persistence
snapshot, and Live exposes only the last final-clean snapshot.
Policy-turn admission by backend-transaction substate, orthogonal to the
`InitialReplay` / `FinalizingReplay` / `Live` recovery phase, is exact:

| Session state | Accepted policy boundary |
|---|---|
| `Idle` | One untagged spontaneous turn. |
| `AwaitingExternalTurn` or `AwaitingInternalTurn` | Exactly the next untagged turn, which consumes the outstanding request. |
| `InFlight(ticket)` | No policy turn; the matching terminal completion must arrive first. |
| `AwaitingDrain(ticket)` | Exactly one turn tagged `drains: Some(ticket)`, or the matching empty marker instead. |
| `QuitPending` | Session admits no action or public backend event. #38 owns the exact requester-response barrier and externally calls `begin_shutdown`; no Session discard ticket is allocated. |
| `ShuttingDown` | Session admits no action or public backend event. #38 externally authorizes the exactly-once `begin_exit_session`; RiverBackend owns the cutoff. |
| `Exiting` | `begin_exit_session` has succeeded at the local exit cutoff. No public policy request/response or new ingress admission is permitted. A turn already consumed by `respond_policy_turn` keeps its emitted/queued bytes and finishes any manage/render phase already open or parsed at cutoff exactly once from its accepted immutable response, with public completion/drain suppressed; a future render phase still kernel-unread is abandoned and never replaced. Only an admitted/open/queued sequence not yet exposed and answered receives the fixed `BackendExitPolicy`. Backend service terminally supersedes kernel-unread bytes and incomplete decoder fragments and returns only `None` until every applicable open-phase finish flushes before the queued exit request, that request flushes, and the expected `Disconnected` arrives; any other public event is fatal. |

Any other tag/state combination is rejected without
changing phase or authoritative state. A correctly tagged draining turn is reduced and answered
without intermediate publication. The library tests expose these predicates;
outer-loop scheduling remains #38-owned.
A pure backend-turn helper owns the library's service decision. Its closed
outcomes are `Idle`, `Progressed`, `Updated`, and the one-shot `ExitComplete`.
The future event loop calls it before blocking poll whenever backend interest is
immediate; otherwise it calls the helper after poll with the
readable/terminal/writable readiness triple. With no
readiness and no immediate interest is an idle turn; otherwise one invocation
permits at most one bounded `service` call. A service call returning no public
event is `BackendTurn::Progressed`, not idle; the outer loop rechecks
`poll_interest()` and may block only after immediate interest clears. This
includes multiple consecutive immediate quanta that each return `None`. Any
public terminal error returns `Err`, discards private state, and is fatal;
`BackendTurn::Updated` is success-only. This helper is not the poll loop.

`Disconnected`, `Unavailable`, `Io`, and post-admission `Unsupported` are fatal
for the current incarnation. Typed `Protocol`/`Capacity`,
`WindowIdExhausted`, and `BackendTicketExhausted` are terminal as well. None
produces an ordinary completion, diagnostic, publication, persistence value, or
effect.

The control socket path is safely created and bound before recovery, but the
event-loop binary must retain the non-listening `BoundControlEndpoint` and must
not consume it for activation or signal readiness until the session is `Live`.
It then verifies the returned active listener before `sd_notify(READY=1)`.
Listen or verification failure is fatal and readiness remains absent. Pre-live
clients therefore receive `ECONNREFUSED`, not a provisional empty state.

**What a restart must *not* restore:** the input mode (reset to `Mode::Nav` — a
restart with a dangling `ensure_next_key_eaten` in the compositor would eat the
user's next keystroke), the chord echo, module state, and any pending worker
job.

**Clients.** Subscribers see EOF, retry, re-`Subscribe`, and get a full snapshot
as their first frame (§7). A crashed bar has never been able to take the session
with it (ADR 0003); this makes the reverse also survivable.

River v0.4.8 replay is verified in source. Binding a new manager marks windowing
dirty; the next manage sequence iterates every surviving window, creates a new
protocol object for each object made inert by the previous manager's teardown,
and sends each `window` event before `manage_start`. Section 6 therefore uses
replay as a version-pinned runtime property, guarded by A18.

### 7. The control-socket server

Serves `realm_core::ipc` one JSON value per line, encoded through `ipc::encode`
and decoded through `ipc::decode` ([ADR 0004](../adr/0004-ndjson-control-socket.md)).
SPEC 0007 is the complete Accepted contract for endpoint ownership, admission,
connection states, malformed frames, queues, replies, deadlines, and test
seams. This section adds only the integration rule below; it must not be read
as a second transport state machine.

The #41 transport exposes every decoded non-Hello request unchanged as
`ControlAction::Request`; #38 owns the authoritative adapter and the combined
poll loop. For `Request::GetState` and the initial Subscribe snapshot, that
adapter uses the last visible `Session::state()` accepted for publication.
`SessionUpdate.state == None` is never published.

**Requests.** A mutating `Request` is validated and invoked as one typed desired
operation. Its initial result is a closed discriminator: an application-class
`Err` under the failure table is an immediate Error, while a fatal-class `Err`
terminates the backend incarnation without an ordinary response;
`Ok(update)` with `pending_action == None` is synchronously
final, so the adapter applies persistence and publication and then queues Ok;
`Ok(update)` with `pending_action == Some(ticket)` is admission only. The
adapter never manufactures an `ActionCompletion` for a ticketless local
completion. In the ticketed case the connection retains
that single decoded request and reads no second request, as required by SPEC
0007, until a later `SessionUpdate` carries its final action result. A pending backend terminal
event alone is not that boundary: retained observations and every required
follow-up completion/drain must finish first. The final update commits the
transaction working state and carries both its one visible publication, if
changed, and its action result. The adapter enqueues that publication to
subscribers before it queues `Response::Ok` or application
`Response::Error` to the requester. `Ok` therefore means "the requested
transaction reached a stable published boundary", not merely "the request was
accepted". A fatal backend result closes the incarnation and yields no ordinary
application response. Waiting to queue a nonblocking response does not put the
client on the input path: the event loop continues serving backend and key
events, and no socket write precedes the final required sequence finish or its
final drain boundary. Either application response returns a read-open peer to
Ready only after it drains; a read-half-closed peer closes after the response
drains, including Error.

The #38 Ready-state adapter has this total request dispatch table; it does not
invent another Session entry point or silently reinterpret a retired request:

| Request | Typed owner | Completion |
|---|---|---|
| `Hello` | SPEC 0007 handshake state, never Ready dispatch | server `Hello`, then Ready or close on version mismatch |
| `GetState` | `Session::state()` | immediate `State` from the last visible boundary |
| `GetKeymap` | `Session::keymap()` | immediate `Keymap` from the exact binding vocabulary configured for this Session; the adapter never substitutes `Keymap::default()` |
| `Subscribe` | SPEC 0007 plus `Session::state()` | immediate initial State becomes the subscription's protected current frame |
| `SwitchOrbit(n)`, `MoveToOrbit(n)` | validate `OrbitId::from_human(n)`, then the matching typed Session desired operation | invalid one-based input is application Error; otherwise synchronous Ok/Error or retained ticket completion |
| `Focus`, `Swap`, `Banish`, `Stow`, `Fullscreen`, `SetLayout`, `Undo` | the matching typed Session desired operation | synchronous Ok/Error or retained ticket completion |
| `Spawn(argv)` | `Session::request_spawn(argv)` | empty vector/program is application Error. On success reserve exactly one worker process slot before `complete_request`; commit/enqueue it only after queued Ok or a peer-local close-proving completion outcome. Ok means admitted argv, not successful `exec` |
| `ShowLedger(None)` | `Session::visible_ledger(None)` | immediate Ledger for every orbit at the last visible boundary |
| `ShowLedger(Some(n))` | validate `OrbitId::from_human(n)`, then `Session::visible_ledger(Some(orbit))` | invalid one-based input is application Error; valid input returns exactly that orbit |
| `ReloadTheme` | no Session or worker call | immediate application Error; supported theme apply is process-local to `realmctl` |
| `Quit` | `Session::begin_direct_quit()` | exact direct-Quit barrier below |

`Session::request_spawn` is valid only in Live while the semantic transaction
state is idle. It returns `NotReady` during recovery or any active
transaction, and `InvalidSpawnCommand` for an empty vector or empty program. A
successful result is ticketless, changes no Realm state, and carries exactly
one ordered `SessionEffect::Spawn`; worker reservation and acknowledgement
ordering remain adapter responsibilities.

`Request::Quit` is the sole direct preemptive exception.
`Session::begin_direct_quit(&mut self) -> Result<SessionUpdate,
SessionEventError>` requires no backend
ticket or policy turn. Its first Live call atomically abandons any private
semantic W/origin/effects without commit/completion,
enters `QuitPending`, stops all new admission/actions, and tells the adapter to queue
`Response::Ok` for the Quit requester and immediately install that receipt's
SPEC 0007 response barrier. A later call in `QuitPending` is
idempotent and returns an unchanged update with no duplicate Quit effect.
#38 masks ordinary/backend admission while the transport drains that exact
response. When its receipt reports Drained, or the requester is already/still
becomes Closed, the adapter calls `begin_shutdown`; later it authorizes
`begin_exit_session`, whose #40 backend cutoff owns any abandoned
backend-private protocol work. Session performs no neutral discard turn. The
abandoned origin receives no ordinary completion. Key-derived `Action::Quit` is
not preemptive: it remains staged in its offered turn and cannot enter
`QuitPending` until that turn's successful response chain reaches a clean final
boundary.
`Request::ShowLedger` answers `Response::Ledger(Vec<OrbitLedger>)` built from
the immutable ledger/metadata snapshot captured at the same last visible final
boundary as `Session::state()`. The per-window `app_id` and `title` are nullable
in the protocol *(verified)* and render as empty strings. Session never retains
an unbounded display string: at observation ingress it normalizes each app id
to at most 40 bytes and each title to at most 80 bytes of JSON-encoded string
content, counting escape expansion but excluding the surrounding quotes.
Overlong nonempty values retain the longest Unicode-scalar prefix for which a
final U+2026 remains inside the cap. Repeated title changes replace the bounded
value rather than accumulating bytes. With 256 windows, maximum-width `WinId`s,
all six orbit records, and worst-case escaped metadata at both caps, the complete
`Response::Ledger` plus LF must encode within SPEC 0007's 65,536-byte frame;
the exact maximum fixture is a regression test. Private observed facts are not
exposed by ShowLedger ahead of GetState/subscribers; all three advance together
at the next clean visible boundary.
Any profile-launch request is handed to the SPEC 0012 lifecycle worker and may
be acknowledged only after that spec's admission and durable preparation
boundary. The exact request DTO, request-id idempotency and reply spelling are
not accepted here; SPEC 0006/#117 owns them. `Request::ReloadTheme` has no
supported apply or notify meaning and is not sent by `realmctl theme apply`;
this specification does not promise compatibility for that retired message. Decoding and protocol-error outcomes are exactly
those in SPEC 0007's state/error table; they are not kept open by default merely
because a decoder can return an error.

**Subscribers.** The #38 adapter completes `Request::Subscribe` with the last
visible accepted `Session::state()`. The transport makes that immediate
`Event::State` the subscriber's current frame, so a restarted bar draws at once
without a round trip; thereafter the subscriber converges on the latest
published state (§8). A clean read-half EOF is allowed and does not unsubscribe
it; any positive input byte closes it.

**Back-pressure, and how a slow subscriber does not become a session failure.**
`Event::State` is a *snapshot*. The bounded transport representation is one
current cursor plus one replaceable latest state:

- The immediate initial snapshot is current even before its first byte and is
  never replaced. Once state A is current, publishing B then C delivers A then
  C whether or not A has started; B is replaced in the latest slot.
- When current completes, the replaceable latest state becomes current. A slow
  subscriber skips intermediate states and converges without an unbounded
  queue.
- Writes are non-blocking. `EAGAIN` leaves the cursor where it is and waits for
  the fd to become writable in the next `poll`.
- Subscriber output has SPEC 0007's two-second no-progress deadline from queue
  time, reset only by a positive send. A client that makes no progress for two
  seconds closes without delaying another peer or backend work.
- Shutdown discards replaceable state. An initial snapshot still current at
  offset zero is preserved and followed by Shutdown; a partial initial/current
  frame finishes alone; only a later non-initial unstarted current frame may be
  replaced by Shutdown. The whole drain has one hard 100 ms deadline. If the
  initial State cannot complete, the client Subscribe receives its ordinary
  InitialState timeout/EOF rather than a successful subscription.

### 8. State derivation

`RealmState` is derived on the event-loop thread, after `manage_finish`, from the
ledger plus the session's own mode and module state.

| Field | Derived from |
|---|---|
| `orbits` | One `OrbitCell` per `OrbitId::all()`: `number = o.human()`, `rune = o.rune()`, `windows = ledger.orbit(o).windows.len()`, `display` = `Active` when `o == ledger.active()`, else `Occupied` when `Orbit::occupied()`, else `Empty` |
| `layout` | `ledger.active_orbit().layout` |
| `mode` | The session's mode machine (§3) |
| `focused_title` | The last `river_window_v1::title` for `ledger.focused()`; empty when nothing is focused, when the title is null, or while a layer surface holds exclusive focus |
| `chord_echo` | Non-empty exactly while a submap is pending or `mod4` is held; cleared on `ate_unbound_key`, on leaving the submap, and on restart |
| `whichkey` | Toggled by `Action::ToggleWhichKey`. Changing it changes the bar's exclusive zone, so the new `Workarea` arrives from river as a `non_exclusive_area` event rather than being computed here |
| `grimoire` | Toggled by `Action::Grimoire`; cleared by the same action or `Action::EnterMode(Mode::Nav)` (the `?` and Escape bindings). It remains private through a pending response/drain chain and publishes only at the final clean boundary |
| `modules` | Owned by the session. Push-driven except for one shared 1 Hz sampler for interval-derived CPU, memory, GPU, and network-rate values; the clock schedules the next minute boundary rather than ticking once a second |
| `revision` | See below |

**When `revision` increments.** The session derives a candidate state and
compares it with the last broadcast one using `RealmState::renders_same_as`,
which ignores `revision` by construction. If they render the same, **nothing
happens**: no increment, no encode, no socket write, no wake-up for any
subscriber. If they differ, `revision += 1` and one `Event::State` goes to every
subscriber's SPEC 0007 current/latest coalescer; an older replaceable state may
therefore be skipped. So `revision` counts *visible* changes, monotonically
within one daemon incarnation, and
`state::tests::revision_alone_does_not_force_a_redraw`
describes the
bar's belt-and-braces check rather than the primary gate — the primary gate is
here, one process upstream, where it also saves the serialisation and the
syscall.

**A module update never touches river.** The clock tick, a CPU sample or a
battery change alters no window-management and no rendering state, so it must
not make `manage_dirty`. A session that took a compositor round trip once a
second would multiply its own input latency for a clock.

### 9. Typed desired-action reducer

The compositor-independent session core exposes one typed execution operation
for each ledger action needed by the M2 keymap and staged control requests:
`focus_step(Dir)`, `swap(Dir)`, `move_focused_to_orbit(OrbitId)`,
`toggle_stow()`, `toggle_fullscreen()`, `undo()`, `switch_orbit(OrbitId)`, and
`set_layout(Layout)`. It does not expose a generic ledger closure or mutable
ledger access. Mode changes, grimoire visibility, and process-level actions
such as spawn, launcher, theme reload, and quit remain outside this reducer
because they have different compositor, visible-state, or process-lifecycle
contracts. Grimoire visibility is nevertheless part of the private transaction
that handles its binding press: it is derived from working authority, not
emitted as a process effect, and commits/publishes only at the final clean
boundary.

These operations are the compositor-sequence execution boundary, not the
socket-decoding or acknowledgement boundary. Section 7 stages at most the one
request already admitted by SPEC 0007 and invokes the corresponding operation
only when the backend has no active transaction (waiting for a turn, in-flight,
draining, or in a follow-up chain).

Every typed ledger operation uses the same transaction boundary: clone the
authoritative ledger, invoke exactly the corresponding `Ledger` method, and
derive one complete projection. If the candidate projection equals the clean
last-successful projection and no binding, close, or policy response is needed,
commit the candidate immediately, publish only if its rendered `RealmState`
changed, and allocate no ticket. A pure ledger no-op likewise completes without
publication or ticket allocation.

Otherwise Session reserves one fresh origin ticket, stages the candidate, calls
`request_policy_turn` once, and returns `pending_action` without committing. At
the offered turn it folds the candidate first, then folds the complete event
batch in report order, and supplies one `BackendPolicyResponse` using the origin
ticket. `BackendSubmission::Complete` is the immediate final clean boundary.
`Pending` retains private state; matching terminal success still commits
nothing. Its next empty marker finalizes, while its tagged draining policy turn
is folded and answered using a fresh follow-up ticket. Any post-admission
terminal error is fatal for the MVP: the prior committed ledger, visible state,
revision, and persistence snapshot remain unchanged, private state is
abandoned, and no ordinary action completion is returned.

`request_close_focused()` begins without a candidate ledger mutation but uses
the same external turn-request and action-progress surface. The target id is
fixed at admission. If that window remains live after the awaited turn's facts
are folded, its close edge is included exactly once in that response and never
repeated by a follow-up. If the same turn already reports it closed,
the edge is omitted and the original close action succeeds at the clean response
boundary. `Complete` finalizes it;
`Pending` matching success retains its result through the empty marker or
tagged draining-turn chain. Retained `WindowClosed` and other
facts reduce into private transaction state and become authoritative only at
the final boundary. A second action is `NotReady` until finalization. The window
remains in the committed ledger until a `WindowClosed` event is observed in a
policy turn and that transaction reaches its boundary.

`request_close_focused()` is intentionally not a ledger mutation. With no
focused window it is a no-op. Otherwise it asks the backend to close exactly
that `WinId` and leaves the ledger, projection, visible state, and revision
unchanged whether the request succeeds or fails. Only the later observed
`BackendPolicyEvent::WindowClosed` removes the window.

An observed `WindowOpened` or `WindowClosed` is a hard undo-history boundary
under SPEC 0001. Clearing history on a window-set change is required before
`undo()` can be exposed: Undo may alter desired ordering, focus, stow, layout,
fullscreen, or active orbit, but it may never remove a currently observed
window or restore a closed one.

`toggle_whichkey()` toggles only `RealmState::whichkey`, increments the
revision once, and emits that state without a backend apply or a locally
invented workarea. If the bar's exclusive zone changes, the resulting
`BackendPolicyEvent::WorkareaChanged` is the sole trigger for re-projection.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a River advertising `river_window_manager_v1` v5, `river_xkb_bindings_v1` v3, `river_layer_shell_v1` v1, `river_input_manager_v1` v2, and `river_libinput_config_v1` v2, when `realm-session` starts, then it discovers/version-checks all five before binding, creates exactly one binding object for each companion, settles both input sync fences with no manage sequence open, and binds the window-manager global exactly once and last. The first complete output in River creation order remains selected while present; every output gets exactly one dependent layer-shell output object, and selected removal fails over to the earliest surviving ordinal with complete geometry and retained non-exclusive area. With no such survivor the backend is unavailable. The first River seat is the single selected MVP seat and alone receives the xkb-bindings-seat, layer-shell-seat, binding objects, focus requests, chord policy, and exclusive-focus observations; no seat at the replay barrier or selected-seat removal is unavailable after neutrally answering any open turn. The first replay is answered without projection and with bindings disabled; only a later selected-output workarea response may project and enter Live. It then seeds six orbits with orbit 1 active and reports `Capabilities` with `exact_geometry`, `server_side_borders`, `hide_show`, `explicit_ordering` and `fullscreen` all true and `unsupported` empty. Absence of the post-MVP `river_xkb_config_v1` global does not fail startup | #40 `backend::tests::river_binds_all_required_globals_and_reports_full_capabilities`, `backend::tests::input_syncs_precede_window_manager_bind_without_open_manage_sequence`, `backend::tests::initial_replay_without_workarea_finishes_then_projects_first_selected_workarea`, `backend::tests::first_complete_output_is_stable_default_and_selected_removal_fails_over_by_creation_order`, `backend::tests::no_complete_output_survivor_is_typed_unavailable`, `backend::tests::first_river_seat_exclusively_owns_realm_input_policy`, `backend::tests::missing_or_removed_selected_seat_is_typed_unavailable`, `backend::tests::xkb_config_absence_does_not_block_mvp_startup`; #38 `realm_session::tests::startup_seeds_six_orbits_with_first_active` |
| A2 | Given any required river global missing or advertising below its refusal version in §1, when `realm-session` starts, then it exits non-zero with a message naming the interface, the version advertised or missing, and the version required, and makes no window-management request | #40 `backend::tests::missing_or_old_required_global_fails_before_window_management_request`; #38 `realm_session::tests::backend_startup_failure_prevents_listener_and_readiness` |
| A3 | Given an offered policy turn and a response placing two windows, when its ticket completes successfully, then no finish is sent before the response, every `propose_dimensions` is sent before exactly one `manage_finish`, every `set_position` is sent after `render_start` and before exactly one `render_finish`, and no `set_position` is sent inside the manage sequence | #40 `backend::tests::policy_response_orders_manage_then_render_and_finishes_once` |
| A4 | Given two tiled windows, when `Swap(Dir::Next)` is applied, then both windows' new positions are sent in a single render sequence terminated by exactly one `render_finish` | `session::tests::swap_changes_order_once_and_is_a_no_op_with_one_window`; #40 `backend::tests::swap_projection_uses_one_render_sequence` |
| A5 | Given a clean successfully completed projection, when a ledger action changes rendered state but derives the same projection and a later projection-changing action follows, then the first candidate commits and publishes without allocating a ticket, the later action receives the next consecutive ticket, and no `propose_dimensions`, `set_position`, or `manage_dirty` request is made for the identical projection. A pure ledger no-op neither publishes nor consumes a ticket | `session::tests::clean_projection_equality_commits_without_consuming_a_backend_ticket`; #40 `backend::tests::clean_projection_cache_suppresses_identical_river_requests` |
| A6 | Given three tiled windows, when `Focus(Dir::Next)` is applied, then no `propose_dimensions` is sent and the only requests are `focus_window` and two `set_borders` | `session::tests::focus_step_commits_one_projection_and_one_visible_state`; #40 `backend::tests::focus_change_emits_only_focus_and_border_requests` |
| A7 | Given a tiled window, when it is stowed, then `hide` is sent inside a render sequence, no `propose_dimensions` is sent for it, and the window is still present in `Response::Ledger` | `ledger::tests::stow_removes_from_projection_but_not_from_the_ledger`; #40 `backend::tests::stow_hides_inside_render_without_dimension_request` |
| A8 | Given `realm-session` bound to `river_layer_shell_v1`, when a `wlr-layer-shell` client maps a top-anchored surface with a 32 px exclusive zone, then the surface is not closed and the resulting `Workarea` has `tiles.y == 32` and `tiles.h == output height − 32` | #40 `backend::tests::layer_shell_exclusive_zone_maps_to_workarea_without_close` |
| A8a | Given a ledger-focused titled window, when one turn reports `ExclusiveFocusChanged(true)`, then its response has no focused placement/border and its final visible state has empty `focused_title`; when a later turn reports false, that turn's response restores `Ledger::focused()` and its border and finalization restores the title. Each turn is answered exactly once. Ordinary `FocusChanged` only acknowledges or causes reassertion of ledger focus, never mutates ledger focus and never implies exclusivity. Any post-admission terminal failure is fatal without publication | `session::tests::exclusive_focus_true_then_false_clears_and_restores_ledger_focus`, `session::tests::ordinary_focus_observation_cannot_change_ledger_policy` |
| A9 | Given `Keymap::default()`, when bindings are configured and the initial replay policy turn is answered, then one `river_xkb_binding_v1` mechanism exists per stable id with the xkbcommon keysym and canonical modifiers, Session alone maps ids to policy, and that projection-free response enables none with next-key Preserve. The first later selected-output workarea response enables exactly the `Mode::Nav` ids | `session::tests::binding_configuration_contains_mechanism_not_policy`, `session::tests::initial_workarea_enables_nav_bindings_after_disabled_replay`; #40 `backend::tests::configured_bindings_create_one_stable_river_object_each` |
| A9a | Given the initial input-manager enumeration or a later hot-plug, when an input device reaches its v2 `done`, then a keyboard receives exactly one `set_repeat_info(25, 600)`, another device type receives none, the existing `default` seat is used without `create_seat` or `assign_to_seat`, and Realm sends no scroll-factor or mapping request. Removal destroys the object and cancels unfinished policy without reuse | #40 `backend::tests::input_defaults_use_existing_default_seat_and_fixed_keyboard_repeat`, `backend::tests::removed_input_device_cancels_unfinished_policy_without_reuse` |
| A9b | Given every tap-support/current combination for a libinput device, when its v2 `done` arrives, then positive support plus disabled current state emits exactly one `set_tap(enabled)` and waits for its result, while already-enabled or zero-support state emits none. `success`, `unsupported`, and `invalid` are handled totally; the latter two are fatal for a request Realm emitted. No other libinput preference request is sent. Initial enumeration and every emitted result settle before recovery/readiness continues | #40 `backend::tests::tap_to_click_is_enabled_only_after_complete_supported_device_snapshot`, `backend::tests::tap_request_result_is_total_and_startup_waits_for_success`, `backend::tests::non_tap_input_policy_is_preserved_without_requests` |
| A10 | Given `Mode::Nav` and the binding id for `r`, when `BindingPressed` arrives in a policy turn, then Session answers that exact turn with the Resize enabled set, disables Nav, and sends exactly one next-key Ensure; neither mode publication nor process/control effects occur before the response reaches its final clean boundary. A post-admission terminal failure is fatal and exposes none of those deltas | `session::tests::resize_binding_is_answered_in_the_same_policy_turn` |
| A11 | Given `Mode::Resize` with the one-shot chord edge effective, when `UnboundKeyEaten` arrives in a policy turn, then its response returns to the Nav enabled set, clears `chord_echo`, and makes no further Ensure. Only a successful final boundary commits that result | `session::tests::eaten_unbound_key_returns_to_nav_in_its_policy_response` |
| A11a | Given empty, release-only, stop-repeat, or modifier-only policy turns, when Session reduces them, then each receives exactly one bounded response; release and `BindingRepeatStopped` disarm repeat irreversibly, modifier state follows report order, and a semantic no-op still answers without publication. Any input event naming an unconfigured binding id fails fatally before partial reduction or response | `session::tests::non_action_policy_turns_are_answered_exactly_once`, `session::tests::unknown_binding_id_is_fatal_before_partial_reduction` |
| A11b | Given the exact successful held binding is still the sole armed target, configured, repeatable, and enabled by committed binding policy, when its timer fires while Session is Idle, then `fire_key_repeat()` rechecks those facts and either finalizes a local result or requests one internal-origin turn with a fresh response ticket but no pending action or ActionCompletion. A later successfully finalized repeatable press replaces/restarts the target; a noncurrent release cannot disarm it, and releasing the replacement never resumes an older held binding. Release, repeat-stop, or finalized mode disable of the current target makes firing an unchanged no-op. A tick during any active backend transaction is consumed unchanged without a ticket/request while leaving the target armed for a later scheduled tick. Every successful update emits one immediate Preserve/Arm/Disarm directive: a finalized same/new press restarts at the Accepted 600 ms delay and 40 ms interval; current release/stop, disabling mode, and Quit disarm before delayed effects. Any backend failure is fatal without an ordinary update. The MVP default marks only Focus and Swap bindings repeatable | `session::tests::armed_repeat_requests_internal_turn_without_action_completion`, `session::tests::released_repeat_is_noop_before_request`, `session::tests::new_repeatable_press_replaces_without_resuming_older_target`, `session::tests::mode_change_disables_armed_repeat_target`, `session::tests::repeat_tick_during_internal_in_flight_is_consumed_without_second_request`, `session::tests::repeat_timer_directives_cover_final_press_pending_release_mode_and_quit`, `keys::tests::only_directional_focus_and_swap_bindings_repeat`; #38 `realm_session::tests::repeat_timer_directives_program_single_timerfd_exactly`, `realm_session::tests::clock_and_repeat_overruns_coalesce_without_catchup` |
| A12 | Given simultaneous backend, repeat timer, clock timer, worker, and control readiness including a subscriber whose socket buffer is full, when the real #38 combined loop runs with #40's River backend, then one bounded ingress epoch plus all already-admitted immediate dispatch/dequeue work and the complete policy response preempt every other source. The loop does not chase merely fresh level-readable bytes, so continuous new ingress cannot starve one timer/worker/control quantum. An already admitted current-target release/stop crosses ingress, dispatch, and dequeue before repeat; repeat and clock overruns coalesce without catch-up; worker results remain bounded; at most one control quantum follows; backend readiness is zero-polled between control operations; and `manage_finish` is flushed within the key-press budget before any subscriber write | #38 `realm_session::tests::combined_loop_orders_all_ready_sources_once`, `realm_session::tests::received_release_crosses_ingress_dispatch_dequeue_before_repeat`, `realm_session::tests::continuous_backend_readability_yields_after_one_admitted_epoch`, `realm_session::tests::repeat_overrun_during_pending_work_is_dropped_without_catchup`; #40 `backend::tests::policy_response_orders_manage_then_render_and_finishes_once`; #65 `realm_session::tests::key_to_manage_finish_meets_four_millisecond_budget_on_reference_linux` |
| A12a | Given 256 admitted process jobs, a due snapshot, a full 256-result queue, or a sealed worker, when #38 performs nonblocking admission/service, then the 256 process slots and one replaceable snapshot slot remain distinct, process job 257 fails atomically, the due snapshot remains accepted/coalesced, a full result queue blocks only the worker while the event loop drains, the capacity-free fence can still seal, and all post-seal jobs are rejected | #38 `realm_session::tests::worker_process_queue_accepts_256_and_rejects_257_atomically`, `realm_session::tests::worker_result_queue_blocks_worker_not_event_loop`, `realm_session::tests::snapshot_slot_is_separate_replaceable_and_accepted_beside_256_process_jobs`, `realm_session::tests::worker_seal_rejects_new_jobs_without_consuming_queue_capacity` |
| A13 | Given subscriber output whose last positive send was two seconds ago, when the #38 loop supplies the exact deadline to SPEC 0007, then that subscriber is closed without waiting for another state change and remaining subscribers continue | Transport deadline evidence belongs to SPEC 0007 A16; #38 `realm_session::tests::subscriber_deadline_expires_without_new_state` |
| A14 | Given a client that sends `Request::Hello` with a version other than `PROTOCOL_VERSION`, when the session receives it, then it answers `Response::Hello` carrying its own version and then closes the connection | Delegated to SPEC 0007 A14: `realm_control::tests::protocol_state_machine_is_total`, `realm_control::control_socket_linux::mismatched_hello_discards_a_pipelined_request_and_sends_only_hello` |
| A14a | Given `XDG_RUNTIME_DIR` is absent, relative, or not a directory, when `realm-session` starts or a production client resolves the control socket, then it fails with `IpcPathError::MissingRuntimeDir`, never probes `/tmp`, and ignores `REALM_SOCKET` | Delegated to SPEC 0007 A1/A2: `realm_control::tests::runtime_capability_rejects_every_unsafe_input_and_openat2_failure`, `realm_control::tests::server_creates_realm_exactly_once_under_scoped_umask`, and `realm_control::tests::client_endpoint_missing_realm_is_retryable_and_creates_nothing` |
| A14b | Given a connecting peer with a uid other than the session's effective uid, or no readable credentials, when it is accepted, then the transport closes it before consuming a frame and a separately admitted same-uid peer remains intact | Delegated to SPEC 0007 A13 |
| A14c | Given every accepted connection state and frame error class, when input is read, then the response, exact deadline, and close/continue result match SPEC 0007's total table without affecting another peer | Delegated to SPEC 0007 A14-A16 |
| A14d | Given matching Hello plus `Subscribe` and optional clean read-half EOF, when the adapter accepts it, then the Hello drains first, the last visible accepted `Session::state()` is the immediate current event, later states use current-plus-latest coalescing, positive subscriber input closes, and shutdown cannot replace an unstarted initial State | Transport evidence belongs to SPEC 0007 A14/A14b/A16a; #38 `realm_session::tests::subscribe_initial_state_uses_last_visible_session_boundary` |
| A14e | Given 64 admitted peers, a 65th peer, oversized/unterminated input or output, excess pipeline, a stalled subscriber, or a stalled ordinary client, when a SPEC 0007 bound is reached, then only the affected connection or bounded subscriber set closes; #38 still services pending/backend work first and checks backend readiness between bounded control quanta | Transport evidence belongs to SPEC 0007 A14-A17; #38 `realm_session::tests::control_quantum_rechecks_backend_before_next_peer` |
| A14f | Given a delayed snapshot worker and an immediately ready River replay, when startup runs, then snapshot classification completes before binding the window-manager global, so no manage turn can open first. A restricted pre-listener pump services bounded backend immediate/prepared-read/write and only the successful projection-free replay, selected-output workarea projection, matching completion, and drain/follow-up chain while the endpoint remains non-listening. Transition to `Live`, listener activation, and `READY=1` occur only after that chain's clean final boundary; any worker/backend/poll/recovery/listen failure is fatal and sends no readiness | `realm_session::tests::snapshot_classification_precedes_window_manager_binding`, `realm_session::tests::pre_listener_recovery_pump_reaches_live_before_activation`, `realm_session::tests::readiness_follows_live_listener_activation` |
| A14g | Given any lifecycle phase and optional active transaction, when #38 externally calls `Session::begin_shutdown`, the first call preserves the ticket watermark, abandons every private candidate/origin/ticket/effect without commit, backend request/response, or completion, enters `ShuttingDown`, and returns only repeat Disarm; later calls are unchanged. Given `ShuttingDown`, #38 externally authorizes exactly one `begin_exit_session` after its own response, control-drain, and worker-fence conditions settle; Session owns no receipt or clock. The call passes last-committed binding policy, enters `Exiting` only on backend success, and treats wrong phase, repetition, or backend failure as fatal. #40 owns the backend cutoff and #38 owns receipts, deadlines, worker fences, and poll-loop shutdown proof | `session::tests::shutdown_and_exit_lifecycle_is_minimal_idempotent_and_exactly_once`, `turn::tests::expected_exit_disconnect_returns_exit_complete_once`; #38 `realm_session::tests::shutdown_driver_calls_session_exit_once_and_stops_on_exit_complete`; #40 `backend::tests::exit_cutoff_disables_read_interest_and_requires_postflush_terminal` |
| A14h | Given direct `Request::Quit` in `Live`, when the adapter calls `begin_direct_quit`, Session allocates no backend ticket or turn, abandons any private transaction without commit/completion, enters `QuitPending`, stops admission, and returns one `QuitPending { CurrentControlRequest }`; a repeat call in `QuitPending` is unchanged, and calls in other phases are typed errors without mutation. #38 queues the Quit response, proves that exact response Drained or Closed, and only then calls `begin_shutdown`; Session owns no receipt or deadline. Key-derived Quit remains staged until its successful transaction reaches a clean final boundary | `session::tests::direct_quit_enters_quit_pending_idempotently_without_backend_ticket`; #38 `realm_session::tests::direct_quit_waits_for_exact_response_receipt` |
| A14i | Given the production `realm-wm` binary, tracked service, and a headless River v0.4.8 session, when the service starts from a clean runtime directory, then the binary constructs the real River backend, control endpoint/server, worker, repeat/clock timers, and combined loop; it reaches `Live`, activates the listener, and sends `READY=1`. A real `realm_control::Client` completes Hello, sends `Request::GetState`, receives the current State, then sends `Request::Quit`; that exact Quit response drains before the daemon exits River through the specified post-flush disconnect. The smoke uses the control library directly and adds no CLI surface. A library-only driver or fake backend does not satisfy this criterion. The packaged NixOS, Ubuntu, and Fedora lanes must launch the installed binary rather than a workspace artifact | #38 `realm_session::tests::production_daemon_wires_every_runtime_owner`; #40 `backend::tests::headless_river_daemon_reaches_ready_serves_get_state_and_quits`; #74/#75/#76 packaged session smoke, including the enabled `checks.x86_64-linux.session-boots` assertion |
| A15 | Given an idle session, when the clock module's tick changes the clock text, then exactly one `Event::State` is broadcast and no `manage_dirty` and no other river request is made | `session::tests::module_change_emits_once_without_backend_apply` covers the in-process state effect; socket coverage remains SPEC 0007 |
| A16 | Given a module that recomputes to the text it already had, when derivation runs, then `revision` does not increment and no `Event::State` is sent | `session::tests::module_change_emits_once_without_backend_apply` |
| A17 | Given a client that quantises its dimensions down to a multiple of a 9×18 cell, when a triptych of three such clients is applied, then after at most one corrective `propose_dimensions` per window each `set_content_clip_box` equals that window's projected rect and the clip boxes tile the workarea exactly | #40 `backend::tests::corrective_content_clip_converges_once` |
| A18 | Given a persisted ledger snapshot holding three windows across two orbits, their `BackendWindowId` mappings, and a next-`WinId` watermark, when all identities replay and `InitialReplayComplete` is the final item of the first policy turn, then the projection-free bootstrap response either finalizes immediately on `Complete` or waits through matching completion and its drain/follow-up chain without entering Live. At that boundary each window is restored to its snapshotted orbit/order, a new identity receives the persisted next id, and immediate Undo is a no-op. Only a later selected-output workarea response produces the first full `RealmState` at revision 1 with `Mode::Nav`, empty chord/module state, and default which-key state. Accumulated exclusive focus suppresses that response's window focus/border and makes revision-1 `focused_title` empty without changing ledger focus | `session::tests::initial_replay_is_projection_free_and_stays_unpublished`, `session::tests::initial_workarea_projection_with_exclusive_focus_suppresses_window_focus` |
| A19 | Given a typed desired operation whose error-atomic turn request reports `Unsupported`, then the immediate application Error names the capability and no transaction is admitted. Given the same class after a response is accepted, the MVP terminates the session, exposes no application completion, and commits no private desired/key/effect/observation delta | `session::tests::unsupported_turn_request_is_immediate_error_but_terminal_failure_is_fatal`; socket-frame coverage remains SPEC 0007 |
| A20 | Given River attempted backend-private `set_default` and that response later failed, the production cache remains dirty. The MVP Session fails closed and sends no neutral response; post-MVP #40 must prove that any next applicable response in a same-incarnation recovery design, including a neutral response, re-emits `set_default` | #40 `backend::tests::failed_selected_default_is_reemitted_by_next_applicable_response` |
| A21 | Given a newly observed backend window identity in a live or draining policy turn, when local `assign_window` receives the live identity/id pair, then it installs the map without protocol I/O and repeating the pair is a no-op. A later focus/title/geometry/close reference to that backend identity in the same turn resolves the assigned Realm id in exact report order. Initial replay instead resolves focus against earlier accumulator opens but preflights and assigns only at the final barrier. A reference before open, unknown object, or conflicting pair is a typed fatal backend-contract error that abandons the private mapping and turn without publication or retry | `session::tests::identity_binding_is_local_idempotent_and_contract_closed`, `session::tests::open_then_focus_and_title_in_one_turn_binds_once_in_order`, `session::tests::initial_replay_is_projection_free_and_stays_unpublished` |
| A22 | Given two tiled windows, when `focus_step(Dir::Prev)` succeeds, then the focused id and title change, revision advances once, and the next offered turn receives exactly one complete candidate response. A terminal failure exposes none of the candidate and terminates under A38 | `session::tests::focus_step_commits_one_projection_and_one_visible_state` |
| A23 | Given two tiled windows, when `swap(Dir::Prev)` succeeds, then their ledger order and focus change through exactly one requested policy turn and response; with fewer than two windows it requests no turn, consumes no ticket, and emits no state | `session::tests::swap_changes_order_once_and_is_a_no_op_with_one_window` |
| A24 | Given any typed ledger operation in §9, when its candidate policy response fails terminally, then no candidate ledger, visible state, revision, persistence, completion, or effect is exposed before fatal termination. A failed `undo()` does not consume history, and observed window lifecycle prevents Undo from changing the live window set | `session::tests::failed_undo_does_not_consume_history`, `session::tests::undo_never_removes_an_open_window_or_restores_a_closed_window`; A38 exercises the shared fatal boundary |
| A25 | Given a focused window, when `request_close_focused()` is admitted, then it requests one policy turn and, except for A25a's already-closed race, the response contains exactly that id as a one-shot close edge without directly mutating ledger, projection, visible state, or revision. Error-atomic request `Io` or `Unsupported` returns Error immediately; every post-admission terminal failure is fatal without ordinary completion. A retained or later `WindowClosed` is committed/published only after its successful response chain is clean | `session::tests::close_request_waits_for_the_observed_close_before_mutating_state`, `session::tests::failed_close_turn_request_is_an_immediate_clean_error`, `session::tests::backend_loss_during_close_is_fatal` |
| A25a | Given a close target fixed at admission, when the awaited policy turn reports that target already closed, then Session omits the close edge, commits the retained close fact, and completes the original close successfully at that response's final clean boundary. If the response contains the edge, no later follow-up repeats it | `session::tests::close_target_closed_in_awaited_turn_is_not_replayed` |
| A26 | Given no focused window, when `request_close_focused()` runs, then it makes no backend request and returns `Ok(SessionUpdate::unchanged())` with no pending action, action completion, effect, publication, persistence, diagnostic, or consumed ticket | `session::tests::close_request_is_a_no_op_without_a_focused_window` |
| A27 | Given a stable session, when `toggle_whichkey()` runs, then `whichkey` and the revision change once with no backend apply; only a later observed `WorkareaChanged` may re-project windows | `session::tests::whichkey_toggle_only_emits_state_until_workarea_is_observed` |
| A28 | Given a mutating socket request, its initial typed result is closed: an application-class pre-admission Err queues immediate Error, a fatal-class Err terminates without ordinary response, `Ok` with no pending action is synchronously final, and `Some(ticket)` is admission only. Until a successful ticketed chain reaches its final clean boundary no ordinary reply or update field exposes the transaction. At that boundary one `SessionUpdate` atomically co-releases repeat directive, persistence value, visible state, original action result, ordered bounded effects, and any `QuitAfter`; Task 3 proves no early exposure and effect-vector/Quit ordering only. #38 owns temporal persistence, publication, response-receipt, effect consumption, and response-before-shutdown ordering. Any post-admission failure emits none of these ordinary outputs | `session::tests::ticketless_local_update_is_final_without_action_completion`, `session::tests::final_boundary_coreleases_state_completion_and_ordered_effects`, `session::tests::spawn_then_quit_preserves_pre_quit_effect_order`, `session::tests::quit_then_spawn_suppresses_post_quit_effect`, `session::tests::staged_effect_cap_applies_across_drain_chain`; #38 `realm_session::tests::mixed_action_and_quit_drains_response_before_shutdown` |
| A28a | Given every wire `Request`, when a Ready peer dispatches it, then the §7 table selects exactly one typed owner and total result. Direct Spawn is accepted only by Live+Idle `Session::request_spawn`, rejects empty vector/program, reserves one worker slot before Ok, and means admission rather than process success. Orbit-bearing mutations and ShowLedger reject invalid one-based values as application Error before Session mutation; valid ShowLedger reads the last visible boundary. ReloadTheme returns application Error without Session/worker work. Hello and Subscribe remain transport-state operations, and Quit remains the only preemptive Session operation | `session::tests::direct_spawn_is_ticketless_effect_only_and_rejects_invalid_command`, `session::tests::direct_spawn_is_not_ready_during_active_transaction`; #38 `realm_session::tests::request_dispatch_table_is_total`, `realm_session::tests::spawn_response_reserves_worker_before_acknowledgement`, `realm_session::tests::invalid_orbit_and_retired_reload_are_application_errors` |
| A28b | Given nullable, overlong, control-character-heavy, or repeatedly changing app ids/titles, when Session admits the observation and later builds ShowLedger or `RealmState::focused_title`, then null becomes empty, app id/title normalization counts JSON escape expansion against the exact 40/80-byte caps, truncates only at a Unicode-scalar boundary with a final U+2026, and replacement does not accumulate storage. The worst-case 256-window all-orbit `Response::Ledger`, including maximum-width ids and LF, bounded-encodes below `MAX_FRAME_BYTES` rather than cloning or serializing an attacker-sized response on the event loop | `session::tests::visible_metadata_is_json_bounded_at_ingress`, `session::tests::repeated_title_changes_replace_bounded_metadata`, `session::tests::maximum_show_ledger_fits_one_control_frame` |
| A29 | Given absent, wrong-version, malformed, semantically invalid, exact-65,535-byte, oversized, growing, and valid `SessionSnapshotV1` files, when the bounded pathname worker and pure classifier run, then absence and invalid/oversized content start fresh without partial state, valid content is accepted, no allocation follows untrusted metadata, at most 65,536 bytes are read to detect overflow, and a non-`NotFound` read error is fatal | `session::tests::snapshot_validation_is_closed_and_total`, `snapshot::tests::classifies_completed_snapshot_reads_without_file_io`; #38 `realm_session::tests::snapshot_worker_reads_at_most_limit_plus_one`, `realm_session::tests::snapshot_worker_rejects_growth_beyond_exact_limit`, `realm_session::tests::snapshot_worker_read_error_prevents_listener_activation` |
| A30 | Given a valid snapshot and an initial replay, when restored, duplicate, and new identities plus optional ordinary/exclusive-focus observations arrive before `InitialReplayComplete`, then the first open occurrence retains report order, latest metadata/focus state wins, each identity is assigned exactly once after the barrier, no projection/state/snapshot is produced, and event order deterministically fixes new ids. A replay workarea is impossible for a conforming River backend and is rejected before assignment/response. A later selected-output workarea becomes authoritative and is the sole trigger for the first complete projection. Focus never changes ledger policy; any other replay event forbidden by §6 is a protocol error rather than deferred work | `session::tests::initial_replay_is_silent_and_rebinds_each_identity_once`, `session::tests::initial_replay_is_projection_free_and_stays_unpublished`, `session::tests::initial_replay_rejects_workarea_before_assignment`, `session::tests::pre_barrier_non_replay_events_are_protocol_errors`, `session::tests::first_selected_workarea_projects_and_publishes_once`; #40 `backend::tests::initial_replay_without_workarea_finishes_then_projects_first_selected_workarea` |
| A31 | Given snapshotted identities that do not all reappear plus new identities, when `InitialReplayComplete` is the final item of the first policy turn, then missing windows are removed before new windows are summoned in report order, undo is empty, and that exact turn receives one projection-free response with bindings disabled and next-key Preserve. Neither `Complete` nor a finished pending drain enters Live. A later selected-output workarea requires a forced complete projection; `Complete` enters Live immediately, while `Pending` withholds revision-1 state, persistence, listener activation, and readiness through its successful drain/follow-up chain. If a Pending bootstrap's correctly tagged drain turn carries the first workarea, that turn emits the forced projection and enters Live only at its own clean boundary. Before workarea, compositor facts accumulate privately into revision 1 while binding input is fatal before reduction/response. Any post-admission bootstrap or workarea-projection failure is fatal without publication/readiness | `session::tests::bootstrap_replay_and_first_workarea_gate_live_until_final_boundary`, `session::tests::pre_workarea_authoritative_turns_accumulate_until_revision_one`, `session::tests::pre_workarea_binding_input_is_fatal_before_reduction`, `session::tests::bootstrap_pending_failure_is_fatal_without_publication`; binary listener/readiness evidence remains in #38 |
| A32 | Given Session is awaiting an external/internal turn or a successful pending response's completion/drain, then desired/close actions and publication remain gated while backend immediate/read/write service continues; module/which-key updates are not admitted and recompute only after finalization. The bounded helper reports only `Idle`, `Progressed`, `Updated`, or one-shot `ExitComplete`; an error is fatal rather than a retry state | `session::tests::pending_backend_work_gates_actions_and_publication`, `session::tests::active_transaction_defers_module_and_whichkey_publication`, `turn::tests::backend_turn_drives_active_success_transaction` |
| A33 | Given `InitialReplay`, `FinalizingReplay`, or any live transaction whose response/drain chain has not reached a clean final boundary, when state, ledger, or persistence is queried, then it exposes only the prior final-clean C/V/L/snapshot; every desired, key, effect, and compositor-observation delta remains private. A post-admission failure terminates without promoting any private fact | `session::tests::persistence_exposes_only_final_clean_authority`, `session::tests::workarea_authority_does_not_expand_snapshot_v1` |
| A34 | Given `next_win_id == u64::MAX` and an unknown identity buffered during replay, when the replay barrier stages reconciliation, then it is refused as exhausted without assigning `WinId(u64::MAX)`, changing the ledger, or publishing state | `session::tests::exhausted_watermark_never_allocates_the_sentinel` |
| A35 | Given snapshot A is in flight and later authoritative values B then C arrive, when A succeeds or fails, then neither completion clears C, only C is yielded next, the first dirty deadline never slides but is hidden while A is in flight, a failed latest value is retained with a 250 ms retry deadline, value reversions create no redundant write after their matching durable result is known, and shutdown yields the latest unpersisted value | `persistence::tests::older_completion_cannot_clear_the_latest_snapshot`, `persistence::tests::failed_latest_snapshot_rearms_from_completion`, `persistence::tests::queued_value_reverting_to_persisted_is_not_written`, `persistence::tests::in_flight_value_reversion_is_cleared_only_by_success`, `persistence::tests::failed_in_flight_write_clears_latest_value_already_persisted`, `persistence::tests::shutdown_yields_the_latest_unpersisted_snapshot` |
| A36 | Given committed projection P1 and desired candidate P2, then the action reserves an origin ticket and requests one turn. Request success returns only `pending_action`; error-atomic request `Io`/`Unsupported` returns an immediate Error, consumes the reserved number, and creates no pending transaction. If the turn's response is pending, matching success and every drain/follow-up keep committed ledger, `last_projection`, visible state, revision, and persistence at P1. An empty marker commits P2 and completes the origin once; a tagged draining turn is folded and answered, and either `Complete` finalizes immediately or `Pending` continues the chain | `session::tests::complete_response_finalizes_synchronously_without_pending_artifacts`, `session::tests::pending_response_commits_only_after_matching_ok_and_clean_drain`, `session::tests::tagged_retained_turn_replaces_marker_and_finalizes_only_after_its_response`, `session::tests::failed_turn_request_is_error_atomic`, `session::tests::failed_turn_request_consumes_origin_ticket` |
| A37 | Given an unanswered policy turn, a different response ticket active, a completed response awaiting its drain, or no transaction, when a stale, future, duplicate, unknown, untagged, or out-of-order turn response, completion, drain tag, or empty marker is reported, then Session returns a backend-contract error and commits nothing, publishes nothing, runs no effect, and completes no action. `Protocol`/`Capacity` from any request or matching terminal is likewise fatal and never becomes an application Error | `session::tests::policy_turn_and_completion_tags_are_exact`, `session::tests::protocol_capacity_and_backend_loss_are_fatal_without_completion` |
| A38 | Given any initial or tagged post-admission response reports `Io` or `Unsupported`, then Session fails closed, discards the entire private transaction and origin, emits no ordinary state, persistence, action completion, diagnostic, or effect, requests no new turn, and terminates the incarnation. `Protocol`, `Capacity`, backend loss, response-call error, and service error are fatal under the same no-ordinary-output boundary | `session::tests::post_admission_io_and_unsupported_fail_closed_without_completion_or_effects`, `session::tests::protocol_capacity_and_backend_loss_are_fatal_without_completion` |
| A39 | Given a tagged observed-only turn, before its own response finishes its facts are absent from C and `snapshot()`. On matching `Ok` they remain private while a drain is outstanding; only that response's clean final boundary atomically installs C/snapshot and advances V, the visible ledger, and publication. Any terminal error instead takes A38 and promotes nothing | `session::tests::tagged_observed_only_authority_is_private_until_its_clean_final_boundary` |
| A40 | Given raw observations and binding inputs preceding one River `manage_start`, then the backend exposes exactly one report-order `PolicyTurn`, sends no finish before its answer, and cannot expose a second turn while it is unanswered. `BackendWindowId` is constructible only from 1..=32 printable ASCII bytes, so every live identity can enter snapshot V1. A live/draining turn has at most 256 ordered events; every turn has at most 65,535 total UTF-8 payload bytes and canonical unique enum-ordered modifier vectors of at most four values; the session has at most 256 managed windows and 64 binding mechanisms; the production backend has at most 16 live outputs, 16 live seats, 64 live input devices, and 64 live libinput devices including pre-done objects; and one transaction has at most 256 total effects across all tagged turns. Each backend object kind uses checked nonzero never-reused `u64` creation ordinals. Cap or ordinal overflow is fatal before map/child installation, destroys the received proxy and owned children exactly once, and removal frees only the live slot. Unused input names/libinput arrays are discarded per bounded dispatch rather than retained. Backend and Session independently reject text/modifier violations before partial exposure/reduction. Initial replay alone may contain 259 events: 256 opens, optional ordinary focus, optional exclusive focus, and the final barrier. Non-coalescible input/action order is exact. The sole non-equivalent normalization is an unexposed backend-local open/close pair that never enters Realm identity authority; every exposed lifecycle fact preserves watermark reduction. Response closes are unique live WinIds in first-occurrence order. Event, text, modifier, chain-effect, object, ordinal, or close overflow/invalidity is fatal before partial exposure/answer. The first Quit suppresses later derived actions/effects across the chain but not authoritative facts. Behind a pending response, retained facts appear only in one correctly tagged draining turn, or absence appears as one empty marker; the forms are mutually exclusive and preserve equivalent deterministic reduction | `backend::tests::backend_window_id_accepts_exact_bounds_and_rejects_invalid_bytes`, `session::tests::policy_batch_is_atomic_and_linear`, `session::tests::maximum_policy_batch_preserves_input_order_and_is_bounded`, `session::tests::policy_text_and_modifier_bounds_are_atomic`, `session::tests::maximum_replay_with_focus_and_barrier_is_accepted`, `session::tests::initial_replay_rejects_workarea_before_assignment`, `session::tests::staged_effect_cap_applies_across_drain_chain`, `session::tests::quit_barrier_suppresses_effects_in_later_draining_turn`, `session::tests::duplicate_close_edges_are_unique_and_bounded`, `session::tests::retained_policy_turn_replaces_empty_drain_marker`; #40 `backend::tests::retained_observations_preserve_equivalent_report_order_under_bounds`, `backend::tests::retained_observation_capacity_is_fatal_before_partial_exposure`, `backend::tests::river_object_caps_and_ordinals_are_atomic_for_every_kind`, `backend::tests::river_object_removal_frees_slot_without_reusing_ordinal`, `backend::tests::river_object_capacity_failure_destroys_proxy_and_children_once`, `backend::tests::unused_device_metadata_is_not_retained` |
| A41 | Given queued protocol work, continuous peer output, no prepared-read guard, a live prepared read, or a blocked flush, then each `service` call selects exactly one bounded phase from ADR 0021 in fixed eligible priority: pending/fresh-writable flush, one public dequeue, one protocol dispatch, one prepared readable/error/hangup `recvmsg`, then one prepare-read attempt. Immediate interest is true whenever no guard exists and preparation is eligible, so an idle backend prepares before poll; successful preparation clears that cause, while queued-work/no-guard preparation remains immediate progress. `BackendPollInterest::readable` controls `POLLIN` and is true before, but false after, the exit cutoff; `POLLIN` and `POLLERR | POLLHUP` map independently to `BackendReady::readable` and `BackendReady::terminal`, so HUP-only consumes the live guard and post-cutoff payload cannot hot-loop or masquerade as disconnect. `POLLNVAL` is immediate fatal backend-incarnation failure and is never passed as ordinary readiness. Ingress admits at most 16,384 bytes and 253 descriptors into at most 65,535 retained partial-frame bytes, while total backend-owned received descriptors across partial decoding and queued/staged messages never exceeds 253. It returns at most one public event, never loops an ingress phase, preserves partial frames, treats truncation/invalid framing/cap overflow as fatal, closes all descriptors owned by failed ingress exactly once, reports `None` progress for internal phases, consumes/cancels each guard once, and never retries a blocked flush without fresh writable readiness. Simultaneous continuous-readable+writable service flushes finite finish output first, and consumed readiness is not reused without recomputation/poll. Stock drain-all paths are forbidden | `turn::tests::backend_turn_services_immediate_and_writable_work_once`, `turn::tests::immediate_none_progress_is_rechecked_before_blocking`; #40 owns ADR 0021 real-socket, FD-leak, maximum-turn response-budget, guard, and flush-priority evidence and #38 owns `idle_backend_prepares_read_before_poll`, `backend_hup_only_maps_to_terminal_readiness`, `backend_pollnval_is_fatal_without_service`, plus poll-set evidence |
| A41a | Given the production River socket is adversarially fragmented, truncated, descriptor-heavy, continuously readable, simultaneously writable, or blocked on flush, when #40 services it, then each ADR 0021 bound, cleanup rule, guard transition, admitted-epoch seal, and finish priority is proven against real socket behavior rather than a fake | #40 `backend::tests::protocol_dispatch_is_capped_per_service_quantum`, `backend::tests::read_ingress_is_capped_under_continuous_peer_output`, `backend::tests::partial_wayland_frame_continues_across_bounded_ingress_quanta`, `backend::tests::truncated_ancillary_data_is_fatal`, `backend::tests::retained_descriptor_cap_closes_every_owned_fd_on_overflow`, `backend::tests::policy_turn_batch_is_capped_without_reordering_inputs`, `backend::tests::river_policy_text_and_modifier_bounds_precede_exposure`, `backend::tests::writable_finish_flush_preempts_continuous_readable_ingress`, `backend::tests::prepared_read_is_consumed_or_cancelled_exactly_once`, `backend::tests::prepare_read_none_forces_immediate_dispatch_before_poll`, `backend::tests::completion_does_not_emit_empty_marker_before_precompletion_ingress_is_normalized`, `backend::tests::synchronous_flush_with_admitted_postturn_work_returns_pending`, `backend::tests::flush_would_block_requests_pollout_without_busy_retry` |
| A42 | Given the maximum ticket was already allocated, when an MVP-reachable external desired/close origin, replay/spontaneous/key/internal-repeat turn, or tagged successful follow-up needs a ticket, the typed result is `BackendTicketExhausted`, no request or response is sent, zero and the maximum are not reused, and the session terminates. A lower origin reserved before an error-atomic request failure remains consumed. Direct Quit and shutdown allocate no ticket. Origins belonging only to deferred recovery/discard states are post-MVP. River policy-turn id exhaustion is independently fatal before exposure | `session::tests::backend_ticket_exhaustion_covers_every_mvp_reachable_origin_without_request_response_or_wrap`, `session::tests::failed_turn_request_consumes_origin_ticket`, `session::tests::quit_lifecycle_allocates_no_backend_ticket`; #40 `backend::tests::policy_turn_id_exhaustion_is_fatal_without_exposure` |
| A43 | Given a pending response completes with matching `Ok`, then exactly one empty matching marker or one `PolicyTurn { drains: Some(ticket) }` follows. The tagged turn replaces the marker, reduces retained facts privately, and receives a fresh response; `Complete` from that response is the final boundary without another marker. A retained open is assigned before its response projection names it, with mapping/watermark still private. Disconnect, assignment failure, stray/duplicate forms, a marker after a tagged turn, or an untagged turn during drain is fatal | `session::tests::tagged_retained_turn_replaces_marker_and_finalizes_only_after_its_response`, `session::tests::retained_window_open_is_bound_before_followup_projection`, `session::tests::close_after_drain_marker_forms_a_later_ordinary_turn`; #40 `backend::tests::completion_does_not_emit_empty_marker_before_precompletion_ingress_is_normalized`, `backend::tests::synchronous_flush_with_admitted_postturn_work_returns_pending`, `backend::tests::retained_observations_preserve_equivalent_report_order_under_bounds`, `backend::tests::retained_observation_capacity_is_fatal_before_partial_exposure`, `backend::tests::tombstoned_retained_open_emits_one_close_and_no_placement` |

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md); no number here is new.

| Path | Budget | This component's share |
|---|---|---|
| Key press → new geometry submitted | **< 4 ms** | The whole of it. Measured from the `pressed` event being read off river's fd to `manage_finish` being flushed |
| State change → bar redraw | **< 8 ms** | The first part: derive, `renders_same_as`, encode, non-blocking write |
| Bar idle CPU | **~0%** | Nothing polls. The clock schedules the next minute boundary; one shared 1 Hz sampler runs off the input path; the key-repeat timer exists only while a key is held |
| Cold session start → usable | **< 900 ms** | Socket bound, five required globals bound in the specified order, both input sync fences settled, projection-free replay answered, first selected-output workarea projected, ledger seeded or recovered, listener active |
| `realmctl theme apply` | **< 150 ms** | No session share: the CLI publishes in its own process under SPEC 0011. No apply, diff, or post-commit notification may appear on the session input path |

**Which of these become correctness bounds under river, and why.**

The **4 ms** bound does, unambiguously. river "should wait for the manage
sequence to complete before processing further input events" and its input
buffer "is finite" *(verified)*. In v0.4.8 the seat queue holds 1024 events and
then drops new input; it does not disconnect an unresponsive manager. Exceeding
the budget is therefore not merely a slow desktop: sustained stalls can lose
keystrokes. The measurement changes with it: the number to hold is a **worst case
under adversarial conditions** — a wedged subscriber, a theme apply in flight, a
window opening — not a median on an idle machine. ADR 0013's planned M2 liveness
test is exactly that scenario.

The **idle CPU** bound becomes semi-correctness for a second-order reason: every
`modifiers_update` and every binding press opens a manage sequence, so any
periodic `manage_dirty` would sit in front of the user's next keystroke.
"Event-driven" stops being about battery life and starts being about latency.

The other three keep their original character.

**What the protocol and implementation establish.** The XML declares the
`unresponsive` error but names no threshold. River v0.4.8 does not post that
error anywhere; it uses the bounded input queue described above. The 4 ms value
is Realm's product budget, not a River timeout.

## Failure modes

From [PITFALLS.md](../PITFALLS.md). This component **owns** every row of the
"Being the window manager (river)" section:

| Row | Guard specified here |
|---|---|
| A client quantises its proposed size | §5: propose, observe the shortfall, re-propose once, `set_content_clip_box` to the exact tile, give up loudly rather than loop. A17 |
| `realm-session` stalls | §4: one owner, no locks, non-blocking writes, all filesystem and process work on the worker, `manage_finish` before any subscriber byte. A12, A13 |
| `realm-session` dies | §6: snapshot to `$XDG_RUNTIME_DIR/realm/ledger.json`, identifier-keyed reconciliation on restart, supervised by `realm-wm.service`. A18 |
| Layer-shell not served | §1 step 3: bind `river_layer_shell_v1` unconditionally, before anything else needs it. A8 |
| Protocol version drift after a river bump | §1 step 2: declared minimum versions, refuse with a message naming interface, found and required. A2 |

It also owns, from the other sections:

- **"Session dies with a client"** — the session outlives the bar, the launcher
  and the terminal; a crashed client reconnects and receives a full snapshot
  (§7).
- **"Redraw on a timer"** — the session is the *source* of `Event::State`, so
  the `renders_same_as` gate at derivation time (§8) is where no-op frames are
  actually killed. A15, A16.
- **"Version skew between components"** — the server half of the `Hello`
  handshake. A14.
- **"Focus causes relayout"** — a focus change alters only `Placement::focused`,
  so it must produce `focus_window` and `set_borders` and nothing else. A6.

It **contributes to but does not own**: "`WAYLAND_DISPLAY` never reaches D-Bus"
(the session entry script owns it, ADR 0011; the daemon only refuses to start
without it), "fractional scaling blur" (the session works entirely in logical
coordinates and hands the bar its scale), and "partially selectable theme
generation" (SPEC 0011 owns sealing and selection; this component does not
participate in apply or diff).

## Resolved design questions

**1. Ledger persistence: per-mutation or periodic? — Resolved: coalesced
per-mutation persistence.**
Per-mutation is exact — a crash loses nothing — but puts a worker job behind
every keystroke and writes to `$XDG_RUNTIME_DIR` at input rates. Periodic (say
every 2 s, and always on a clean shutdown) is cheap but loses the last few
mutations. A third option is per-mutation with coalescing: the event loop marks
the ledger dirty and the worker writes at most once every 250 ms.
Realm uses the third option. It is bounded work, it never blocks the event loop,
and the worst case is a quarter-second of lost window moves after a crash —
which is less than the user will lose noticing the restart.

The 250 ms interval is a non-sliding maximum deadline, not debounce: the first
dirty live state starts the deadline, later commits replace the queued immutable
snapshot without moving it, and one ordered worker prevents an older sequence
from replacing a newer snapshot. Explicit clean shutdown submits the latest
dirty live snapshot before sealing the worker and awaits its terminal result
behind the ordered fence. Success proves it flushed; worker error or exact
2,000 ms fence expiry records degraded shutdown and permits logout without
claiming persistence success. A live-operation worker failure leaves
authoritative in-memory state untouched, records degraded persistence, and
permits a later retry. Snapshot capture occurs
after a desired transaction applies and commits, after an observed open/close
reaches the same successful final boundary, and after successful initial
finalization. It never captures a rejected desired candidate or changes that
affect only workarea, title, modules, mode, chord, which-key, or pending
assignments. “Across a session”
means across daemon incarnations within one desktop login; the runtime directory
intentionally resets the allocator across distinct logins.

The event-loop-owned pure persistence coordinator compares authoritative
`SessionSnapshotV1` values rather than inferring persistence from visible-state
publication. It assigns one monotonic sequence when the fixed deadline yields
the latest dirty snapshot. At most one sequence is in flight. Completion of an
older sequence cannot clear a newer dirty snapshot; whether that older write
succeeded or failed, the newer value remains due at the original dirty deadline.
While a sequence is in flight, the coordinator exposes no actionable dirty
deadline, because no second request can be submitted; after completion, any
preserved deadline becomes visible again and an already-due newer value may be
submitted immediately. This prevents an event loop from repeatedly arming a
timer to an expired deadline while it can only wait for worker completion.
Value reversion is coalesced against what will actually be durable: reverting a
queued value to the already-persisted value cancels the queued write; reverting
newer dirty state to the in-flight value remains conditional until completion.
Success of that in-flight value clears the now-redundant dirty copy, while
failure re-arms that value 250 ms from completion. If the latest dirty value
equals the previously persisted value and the in-flight write fails, it is also
cleared because the desired value never ceased to be durable.
When a failed completion has no newer value, the failed value remains dirty and
receives a new fixed deadline 250 ms after completion. Shutdown yields the
latest unpersisted snapshot. This slice supplies only this deterministic state
machine: pathname I/O, the worker thread, its bounded channel and its `eventfd`
wakeup land with the real poll integration.

**2. What happens to windows that existed before a restart? — Resolved: river
replays them.**
River v0.4.8's `WindowManager.bind` installs the new manager and calls
`dirtyWindowing`. The resulting `manageStart` iterates every tracked window,
and `Window.manageStart` creates a fresh `river_window_v1` for every ready,
initialized, or mapped window and sends it to the new manager. The source even
guards foreign-toplevel handle creation with the comment that a handle may
already exist when the window manager is restarted. Therefore §6's
identifier-keyed reconciliation and A18 are required behaviour, not an
assumption. Evidence:
[WindowManager.zig](https://codeberg.org/river/river/src/commit/c4b5f706314555f4846e25b8d3635631387b3fdd/river/WindowManager.zig)
and [Window.zig](https://codeberg.org/river/river/src/commit/c4b5f706314555f4846e25b8d3635631387b3fdd/river/Window.zig)
at tag `v0.4.8`.

The three input/configuration interfaces first expose all required v2 `done`
boundaries in River v0.4.6. Source checks across every v0.4.0–v0.4.8 tag show
that v0.4.0–v0.4.5 expose v1 and v0.4.6–v0.4.8 expose v2. The runtime refusal
table therefore makes v0.4.6 the effective minimum; the release baseline and
CI remain pinned to v0.4.8.

**3. How much of `Capabilities` can river actually populate — and what does
`exact_geometry` mean?**
Four of the five fields are unambiguous at v5: `server_side_borders`,
`hide_show`, `explicit_ordering` and `fullscreen` are all true, from
`set_borders`, `hide`/`show`, `place_*` and `fullscreen` *(all verified)*.
`exact_geometry` is the problem, because it can mean "realm can place a window at
an arbitrary rect" (true) or "the window will be that size" (false, always, on
every compositor). With §5's clipping the *rendered* rectangle is exact while
the client's own buffer is not.
Resolved as recommended: `INTERFACES.md` defines `exact_geometry` as "the rendered
rectangle is exactly the projected rectangle", report it `true` when
`river_window_v1` is at version ≥ 3 and `false` below, and push
`"unclipped-dimension-quantisation"` into `unsupported` in the `false` case.*
This definition is part of the accepted interface contract.

**4. Should realm eagerly size windows in inactive orbits?**
§2 specifies a submitted projection as "the visible set", so a hidden window
keeps its old size until it is shown, and the show-after-dimensions rule can
cost an extra render sequence on the first orbit switch after a workarea change.
The alternative is to project all six orbits and propose dimensions for every
window whenever the workarea changes, making every orbit switch exactly one
frame. Projection is pure integer arithmetic over short lists, so six of them is
not the cost; the cost is that `apply`'s slice would have to carry a hidden flag
or the seam would need a second method.
Resolved for M2: ship the simple contract and measure. If the first switch
into an orbit visibly lags after a resolution change, revisit — and revisit it
in `INTERFACES.md`, not with a special case here.*

**5. What is river's actual `unresponsive` threshold, and how does it relate to
4 ms? — Resolved: v0.4.8 has no disconnect threshold.** The protocol declares
the `unresponsive` error, but the v0.4.8 implementation never posts it. River
instead queues at most 1024 seat events while a manage sequence is outstanding;
on overflow it logs and drops the new event. This is verified in
[Seat.zig](https://codeberg.org/river/river/src/commit/c4b5f706314555f4846e25b8d3635631387b3fdd/river/Seat.zig), and
an exact source search finds no use of `postError(.unresponsive)` in v0.4.8.
Realm's 4 ms bound remains a user-visible correctness budget and A12 remains the
MVP guard. A self-watchdog is not an MVP requirement: abandoning a partially
handled key action would invent recovery semantics and cannot make a blocked
event loop responsive. Watchdog and overflow telemetry are post-MVP hardening.

**6. `realm-wm.service` restart policy.** Resolved by Accepted SPEC 0012:
`Restart=always`, with user logout expressed by compositor exit followed by
admission freeze and target stop. This removes the clean-exit ambiguity without
settling any river existing-window replay behaviour.

**7. Where does `Capabilities` live? — Resolved: `realm_core::ipc`.** It is
returned by `WmBackend::connect`, carried by the session health response, and
printed by `realmctl doctor`, so it is one shared serialisable wire type rather
than a session-private duplicate. Its `unsupported` field is `Vec<String>`;
backend implementations construct owned capability names and clients can decode
them without borrowing process-static data.

**8. How are unsupported operations reported? — Resolved: a typed backend
error.** Every fallible `WmBackend` method returns `BackendResult<T>`. An
unsupported operation returns `BackendError::Unsupported { capability }`, with
the same stable capability name used in `Capabilities::unsupported`; transport
loss returns `Disconnected`, connection refusal returns `Unavailable`, and
other transport failures return `Io`. The session may add user-facing context,
but it must not turn an unsupported operation into a silent success. An
error-atomic external turn-request `Unsupported` is an immediate application
Error; after a response is admitted the same class is fatal fail-closed for the
MVP and produces no ordinary response.

---

These resolutions make A1–A18 implementable without an unresolved product or
protocol decision. Later measurements may refine persistence cadence, inactive
orbit sizing, or liveness telemetry without changing the MVP behaviour above.
