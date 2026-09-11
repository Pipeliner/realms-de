# SPEC 0003 — realm-session

- **Status:** Accepted (2026-09-10; control-transport integration correction
  2026-09-11)
- **Milestone:** M2
- **Issue:** [#36](https://github.com/Pipeliner/realms-de/issues/36)
- **Decisions:** [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md),
  [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0004](../adr/0004-ndjson-control-socket.md),
  [ADR 0009](../adr/0009-no-animation-budget.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md),
  [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Implements:** [INTERFACES.md §1](../INTERFACES.md) (`WmBackend`) and
  [§4](../INTERFACES.md) (the socket's server half)
- **Supersedes / Superseded by:** Theme apply/reload clauses are superseded by
  [SPEC 0011](0011-theme-activation-generations.md): the CLI publishes a
  generation without a session request or notification. Accepted
  [SPEC 0012](0012-activation-launch-lifecycle.md) governs activation ownership,
  lifecycle reconciliation and the WM unit restart rule.

> Written before the code, as S14 requires. The **Test** column below is
> deliberately empty: those tests get written next, watched to fail, and only
> then implemented against.
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
`river-xkb-bindings-v1` and `river-input-management-v1`; the keymap, the mode
machine and key repeat; the control-socket server and subscriber fan-out; the
right-hand bar modules; ledger persistence and restart recovery; client
lifecycle.

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
2. **Connect to river and negotiate versions.** Bind, from the registry:

   | Global | Bind at | Refuse below | Because |
   |---|---|---|---|
   | `river_window_manager_v1` | 5 | **4** | `river_window_v1::identifier` and `river_window_manager_v1::exit_session` are `since="4"`; `set_content_clip_box` is `since="3"` *(verified)* |
   | `river_xkb_bindings_v1` | 3 | **3** | `modifiers_watch` / `modifiers_update` are `since="3"` and carry the mode badge and chord echo; `get_seat`, `ensure_next_key_eaten` and `ate_unbound_key` are `since="2"` *(verified)* |
   | `river_layer_shell_v1` | 1 | **1** | Its only version *(verified)* |
   | `river_input_manager_v1` | 2 | **2** | `set_repeat_info` is v1; the atomic device boundary `done` is `since="2"` *(verified)* |
   | `river_xkb_config_v1` | 2 | **2** | The per-keyboard atomic `done` boundary is `since="2"` *(verified)* |
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

   If `river_window_manager_v1::unavailable` arrives, another window manager
   holds the seat *(verified: "guaranteed to be the first and only event")*.
   Exit non-zero immediately; do not retry. `StartLimitBurst=5` in 30 s then
   surfaces it as a dead unit rather than a crash loop.
3. **Bind `river_layer_shell_v1` unconditionally.** Binding it *is* the signal
   that realm supports layer shell: "If the window manager does not bind this
   interface, the compositor should not allow clients to map layer surfaces.
   This can be achieved by closing layer surfaces immediately" *(verified,
   verbatim)*. **Until this bind happens the bar does not appear at all**, and
   the symptom looks like a broken bar rather than a broken window manager.
4. **Seed.** `Ledger::new()` gives six orbits with orbit 1 active
   (`ORBIT_COUNT` is 6, `OrbitId::rune()` gives `ᚠᚢᚦᚨᚱᚲ`). If a recoverable
   snapshot exists, apply §6 instead.
5. **Configure input.** `river-input-management-v1` requests are *not* part of
   a manage sequence — none of them carries the "modifies window management
   state" note *(verified)* — so repeat rate, scroll factor and device-to-seat
   assignment are set once at connect time, outside any sequence. The seat named
   `default` always exists and need not be created *(verified)*; realm does not
   create seats in M2. (ADR 0013's "Seat creation" is available but unused.)
6. **Create bindings, recover, and project.** §3 covers the keymap. Complete
   §6's replay/reconciliation, submit the required complete projection, and
   transition the session to `Live`. The bound endpoint remains non-listening
   throughout recovery and no readiness notification is permitted.
7. **Activate, verify, then report ready.** Consume the one
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

**Shutdown.** `Request::Quit` and `Action::Quit` mean the user asked to log out.
Call SPEC 0007's total `begin_shutdown` once. It stops admission/actions and
closes every non-subscriber immediately. The #38 loop continues servicing
subscriber interests and exact expiry until `is_shutdown_complete()` is true;
only then make `river_window_manager_v1::exit_session`, which is documented as
being for user-requested logout only *(verified)*. The compositor exit makes
the entry freeze admission and stop `realm-session.target`; that target stop is
what prevents restart. SPEC 0012 fixes `realm-wm.service` at `Restart=always`,
because an unsolicited clean WM exit while the compositor and target remain
live is not a logout and must not leave river unmanaged.

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

- **`INTERFACES.md` §1 says "`apply()` maps onto one `manage` sequence". That is
  half of it.** A single `apply()` spans a manage sequence *and* the render
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

**`apply(&[Placement])` — the contract.** The slice is the complete set of
windows that should be visible. Any managed window absent from it is hidden.
Concretely, over one manage + render pair:

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

**Idempotence** (required by `INTERFACES.md` §1): the backend keeps the last
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

### 3. The five companion protocols realm must serve

Under river a window manager is not merely a client of the window-management
protocol. Five companion protocols are obligations, and realm ships broken
without them.

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
focus is `BackendEvent::FocusChanged`, while layer-shell exclusivity is
`BackendEvent::ExclusiveFocusChanged`. A generic session must not infer one
from `FocusChanged(None)`.

**`river-xkb-bindings-v1` — the keymap does not exist until this is served.**
No `river_xkb_binding_v1` object means no keybinding fires at all; the desktop
is a mouse-only tiler.

**`river-xkb-config-v1` and `river-libinput-config-v1` — usable input is not
implicit.** The former is required to manage keyboard layouts rather than
freezing the layout river started with; the latter is required for input-device
policy such as tap-to-click. Both are bound and version-checked during startup.

- At start-up, for each `Binding` in the `Keymap`, call
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
verbatim)*. So realm owns the repeat timer for held bindings, honouring the rate
and delay it configured through `set_repeat_info`, and disarms it on `released`
**or** `stop_repeat`. This is a timer, and
[ADR 0009](../adr/0009-no-animation-budget.md) permits exactly one (the clock),
so it is justified explicitly: **the repeat timer is armed only between a
`pressed` and its `released`/`stop_repeat`, and never exists at idle.** Idle
CPU is unaffected.

**`river-input-management-v1`.** Enumerate `input_device`s, wait for `done`
(v2) so a multi-event device description is seen atomically, and apply repeat
rate, repeat delay and scroll factor. No manage sequence is involved. Devices
stay on the `default` seat.

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
- The **worker** thread owns nothing and does only what it is told over a
  bounded channel: ledger snapshot writes, process spawning and any future
  D-Bus work. It reports back over a channel and signals the
  `eventfd`. The event loop never blocks on a send to it; a full worker queue
  drops the oldest pending duplicate of the same job.
- **No locks.** A single owner means the input path cannot contend, which
  matters because a lock held for 40 ms is a stall nobody sees in review.

The #38 combined loop services pending backend repair and backend work first,
then at most one SPEC 0007 control operation, then performs a zero-time backend
readiness check before another control operation. It captures one
`std::time::Instant` per turn for every transport call. The transport never
drains a ready fd to `EAGAIN`, and the MVP adds no token bucket or audit-only
rate limiter.

*Why not an async runtime.* It would give the same non-blocking behaviour, and
is a defensible alternative. It loses on two counts. First, the committed seam
is synchronous: `WmBackend::next_event(&mut self, deadline: Option<Instant>)`
in `INTERFACES.md` §1 blocks with a deadline, and an async model would have to
fight or replace it. Second, a work-stealing scheduler puts a fairness policy
we did not write between a key press and `manage_finish`, and the budget it must
hold is 4 ms. Two threads with one owner each is boring, and boring is what the
most conservative code in the tree should be
([ADR 0003](../adr/0003-session-daemon-owns-state.md) Consequences).

**One extension to `WmBackend` is required.** For the event loop to poll river's
fd alongside the socket, the trait needs

```rust
/// The backend's readable file descriptor, for the session's poll set.
/// `next_event` may then be called with a deadline of `Instant::now()` to
/// drain without blocking.
fn event_fd(&self) -> std::os::fd::RawFd;
```

This is an extension, not a redesign: no existing method changes meaning, and
`NativeBackend` at M5 can return an `eventfd`. `INTERFACES.md` §1 is updated in
the same commit as the trait, as that file's own rule requires.

**Ordering rule.** `manage_finish` and `render_finish` are issued *before* the
session derives `RealmState`, encodes it, or writes a byte to any subscriber.
This is the single rule that makes a wedged `realmctl` unable to wedge the
desktop.

**Transactional desired state.** The session stages user-requested ledger and
window-metadata changes, projects the staged ledger, and commits them only after
a required `WmBackend::apply` succeeds. An application error leaves the
authoritative ledger, last-successful-projection cache, visible `RealmState`,
and revision unchanged. Externally observed lifecycle and workarea facts are
different: they have already happened in the compositor, so an application
error must not roll them back. They remain recorded, leave the projection dirty
for retry or restart recovery, and the error is returned to the outer loop.
SPEC 0007's socket loop separately guarantees that a rejected desired operation
does not produce a state frame.

After any attempted `WmBackend::apply` returns an error, the compositor's
projection is unknown: it may have accepted a prefix before reporting the
failure. The session marks the projection dirty even when the failed operation
was a desired-state operation that was rolled back locally. Projection equality
must not suppress repair while dirty. The next projection attempt reapplies the
complete current authoritative projection, and only a successful complete apply
clears the dirty state and replaces the last-successful-projection cache.
The same rule applies inside every backend: before returning any `apply` error,
it invalidates all projection, per-window, diff, and request caches. Its next
`apply` must issue the complete requested projection even when that projection
equals the backend's last successful cache; only a successful complete apply
restores cache validity. Identical-projection suppression is permitted only
when no apply error intervened.

`BackendEvent::WindowOpened` is an observed lifecycle fact before identity
assignment is attempted. The session first allocates or restores the stable
`BackendWindowId` mapping, advances the non-reuse watermark when needed, and
records the window in the ledger and metadata. It then calls
`WmBackend::assign_window` before any projection containing that window. An
assignment error preserves the observed window and its numeric identity as a
pending binding, publishes no visible state, and is retried before the next
backend event is read or the next projection is attempted. `assign_window` is
idempotent and error-atomic: repeating the same identity/id pair after success
is a no-op, while an error leaves no binding installed. Replaying the same
backend identifier within one connected backend incarnation reuses the recorded
`WinId` and never advances the watermark or binds twice. Snapshot recovery
preloads identity knowledge but marks every restored mapping unbound in the new
backend incarnation, so each replayed identity is assigned again before use.

The session exposes typed desired-state operations, not an arbitrary mutable
`Ledger` closure. Lifecycle events are the only path that adds or removes a
window. Before any desired projection is submitted, the ledger's window set,
the window-metadata keys, and the values of the stable identity map must be the
same set. This makes it impossible to send a placement for a `WinId` that has
not been observed and successfully assigned.
Typed desired operations and `request_close_focused()` return a
`SessionActionError`: local pre-Live or pending-work refusal is
`SessionActionError::NotReady`, while compositor failures are wrapped as
`SessionActionError::Backend(BackendError)`. `BackendError::Unavailable` is
reserved for a backend that cannot be or remain the active window manager and
must never represent local session readiness.

`BackendEvent::GeometryDrifted` is advisory and does not itself bypass
projection deduplication. The ledger and last-successful-projection cache remain
unchanged; the next genuine ledger or workarea projection supersedes the drift.
The backend that reports drift must therefore invalidate its own per-window
request cache so that applying that next changed projection restores every
affected field.

Until the companion layer-shell focus policy is attached, the generic reducer
returns focus events as deferred work. A valid focus event is never converted
into a backend failure and never mutates the ledger speculatively.

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
> session. `BackendEvent::WindowOpened` carries the compositor's stable
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
   nothing yet. Window-open/workarea observations update only the replay
   accumulator. Realm emits no visible state, assigns no identity, applies no
   projection, accepts no desired mutation, and produces no persistence record
   in this phase. The River adapter coalesces raw child events and emits
   `WindowOpened` only for complete identities that are still live at its first
   `ManageStart`. Therefore Session accepts only `WindowOpened`,
   `WorkareaChanged`, `Disconnected`, and the one `InitialReplayComplete`
   barrier before finalization; `TitleChanged`, `WindowClosed`, focus, exclusive
   focus, or geometry drift before the barrier is a backend protocol error and
   is never deferred.
2. **`FinalizingReplay`.** The backend emits the explicit
   `BackendEvent::InitialReplayComplete` barrier exactly once per backend
   incarnation; a second barrier is a protocol failure. Realm stages the whole
   reconciliation before changing authoritative state or calling the backend.
   It preflights id capacity for every unknown identity, traverses snapshot
   orbits and window order to remove missing windows deterministically, then
   allocates and summons new windows in backend report order into the resulting
   active orbit. Only after staging succeeds does it install the result, clear
   undo/redo once, bind each live identity exactly once for this backend
   incarnation, and submit one forced complete projection, including an empty
   projection. No later backend event may be read while this work is pending.
   Staging failure retains no partial mapping, watermark, ledger, backend call,
   publication, or persistence effect. Only successful assignment and
   projection moves the session to `Live`; it publishes exactly one initial
   `RealmState` at revision 1.
3. **`Live`.** Ordinary observed and desired transitions use §§4 and 9. A
   persistence record can be produced only here and always reflects the
   authoritative observed ledger/mapping/watermark. It remains valid while a
   repair of that authoritative projection is pending, but never contains a
   rejected desired candidate.

Transient backend assignment or projection failure schedules exactly one
immediate retry for the next event-loop turn. `has_pending_backend_work()` is
the mechanical read gate: while true, the loop must call
`retry_pending_backend_work()` and must not call `next_event`. Retry success
resumes the interrupted phase and restores one retry allowance for later work.
While this gate is set, every desired ledger operation and close request is
rejected as `SessionActionError::NotReady` without a backend call, and
non-fallible which-key or module updates are unchanged without publication. The only publication query
allowed during pending work is the §6 authoritative `Live` persistence
snapshot; `FinalizingReplay` still exposes none.
A library-level event injection while work is pending must be rejected without
changing phase or authoritative state. The library test proves that rejection
and exposes the read-gate predicate; proof that the outer loop does not invoke
`WmBackend::next_event` while the predicate is true belongs to the event-loop
binary slice and remains required before that binary is accepted.
A pure backend-turn helper owns the library's `next_event` decision. The future
event loop must call it before entering a blocking poll whenever backend work is
pending; otherwise it calls the helper after poll with whether the backend fd
was ready. Pending work is retried regardless of that readiness flag and with
zero reads. Without pending work, a false readiness flag is an idle turn with
zero reads and a true flag permits at most one `next_event(Some(now))` call. A
successful retry returns without a read; only a later ready invocation may
read. A failed retry is fatal with zero reads, and an event that schedules
repair returns immediately without draining another event. This helper is not
the poll loop and does not by itself complete A32's binary-level read-gate
acceptance.
A second failure of the same pending work is a restartable fatal session error;
it is not retried indefinitely. `Disconnected` and `Unavailable` are
immediately fatal because that backend incarnation is gone; `Unsupported`
while applying authoritative recovery state is an immediate contract failure;
and `WindowIdExhausted` is terminal. A failed desired candidate is rejected
before its authoritative repair is scheduled; an observed lifecycle change
remains authoritative and withholds publication until its repair succeeds.

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

**Requests.** A mutating `Request` is validated and staged as one typed desired
operation, after which the session makes `manage_dirty`. The connection retains
that single decoded request and reads no second request, as required by SPEC
0007. When the compositor grants the manage/render sequence, the session runs
§9's transaction. Only backend success commits the ledger and queues
`Response::Ok`; a backend error rejects the candidate and queues
`Response::Error`. `Ok` therefore means "the ledger changed", not merely "the
request was accepted". Waiting to queue this nonblocking response does not put
the client on the input path: the event loop continues serving backend and key
events, and no socket write precedes `manage_finish`. Either application
response returns a read-open peer to Ready only after it drains; a
read-half-closed peer closes after the response drains, including Error.
`Request::ShowLedger` answers `Response::Ledger(Vec<OrbitLedger>)` built from
the ledger plus the per-window `app_id` and `title` last reported by river (both
nullable in the protocol *(verified)*, rendered as empty strings).
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
ledger access. Mode changes and process-level actions such as spawn, launcher,
grimoire, theme reload, and quit remain outside this reducer because they have
different compositor or process-lifecycle contracts.

These operations are the compositor-sequence execution boundary, not the
socket-decoding or acknowledgement boundary. Section 7 stages at most the one
request already admitted by SPEC 0007 and invokes the corresponding operation
only when the backend can complete its transaction.

Every typed ledger operation uses the same transaction boundary: clone the
authoritative ledger, invoke exactly the corresponding `Ledger` method, derive
one complete projection, submit it at most once, and commit the candidate
ledger and newly derived visible state only after backend success. A backend
error rejects the candidate, retains the prior ledger, visible state, and
revision, and leaves projection state dirty so the next repair submits the
complete authoritative projection. An operation whose externally visible
ledger state is unchanged emits no state and does not submit an unchanged
clean projection.

`request_close_focused()` is intentionally not a ledger mutation. With no
focused window it is a no-op. Otherwise it asks the backend to close exactly
that `WinId` and leaves the ledger, projection, visible state, and revision
unchanged whether the request succeeds or fails. Only the later observed
`BackendEvent::WindowClosed` removes the window.

An observed `WindowOpened` or `WindowClosed` is a hard undo-history boundary
under SPEC 0001. Clearing history on a window-set change is required before
`undo()` can be exposed: Undo may alter desired ordering, focus, stow, layout,
fullscreen, or active orbit, but it may never remove a currently observed
window or restore a closed one.

`toggle_whichkey()` toggles only `RealmState::whichkey`, increments the
revision once, and emits that state without a backend apply or a locally
invented workarea. If the bar's exclusive zone changes, the resulting
`BackendEvent::WorkareaChanged` is the sole trigger for re-projection.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a river advertising `river_window_manager_v1` v5, `river_xkb_bindings_v1` v3, `river_layer_shell_v1` v1, `river_input_manager_v1` v2, `river_xkb_config_v1` v2, and `river_libinput_config_v1` v2, when `realm-session` starts, then it binds all six, seeds six orbits with orbit 1 active, and reports `Capabilities` with `exact_geometry`, `server_side_borders`, `hide_show`, `explicit_ordering` and `fullscreen` all true and `unsupported` empty | |
| A2 | Given any required river global missing or advertising below its refusal version in §1, when `realm-session` starts, then it exits non-zero with a message naming the interface, the version advertised or missing, and the version required, and makes no window-management request | |
| A3 | Given a projection placing two windows, when `apply` runs, then every `propose_dimensions` is sent before `manage_finish`, every `set_position` is sent after `render_start` and before `render_finish`, and no `set_position` is sent inside the manage sequence | |
| A4 | Given two tiled windows, when `Swap(Dir::Next)` is applied, then both windows' new positions are sent in a single render sequence terminated by exactly one `render_finish` | |
| A5 | Given a projection already applied, when `apply` runs again with identical placements, then no `propose_dimensions`, no `set_position` and no `manage_dirty` request is made | |
| A6 | Given three tiled windows, when `Focus(Dir::Next)` is applied, then no `propose_dimensions` is sent and the only requests are `focus_window` and two `set_borders` | |
| A7 | Given a tiled window, when it is stowed, then `hide` is sent inside a render sequence, no `propose_dimensions` is sent for it, and the window is still present in `Response::Ledger` | |
| A8 | Given `realm-session` bound to `river_layer_shell_v1`, when a `wlr-layer-shell` client maps a top-anchored surface with a 32 px exclusive zone, then the surface is not closed and the resulting `Workarea` has `tiles.y == 32` and `tiles.h == output height − 32` | |
| A9 | Given `Keymap::default()`, when the first manage sequence completes, then one `river_xkb_binding_v1` exists per binding, each created with the xkbcommon keysym of its `Binding::key`, and exactly the `Mode::Nav` bindings are enabled | |
| A10 | Given `Mode::Nav` and the binding for `r`, when its `pressed` event arrives, then in that same manage sequence the `Mode::Resize` set is enabled, the `Mode::Nav` set is disabled, and exactly one `ensure_next_key_eaten` request is made | |
| A11 | Given `Mode::Resize` with `ensure_next_key_eaten` outstanding, when `ate_unbound_key` arrives, then the mode returns to `Mode::Nav`, the Nav set is re-enabled, `chord_echo` is empty and no further `ensure_next_key_eaten` is made | |
| A12 | Given a subscriber that has stopped reading and whose socket buffer is full, when a bound key is pressed in the real #38 combined loop with #40's River backend, then backend/pending work precedes bounded control work, backend readiness is rechecked between control operations, and `manage_finish` is made within the key-press budget before any subscriber write | #38 owns loop-order evidence; #40 supplies real `manage_finish`; #65 owns the real Linux budget assertion |
| A13 | Given subscriber output whose last positive send was two seconds ago, when the #38 loop supplies the exact deadline to SPEC 0007, then that subscriber is closed without waiting for another state change and remaining subscribers continue | Transport deadline evidence belongs to SPEC 0007 A16; #38 owns deadline integration |
| A14 | Given a client that sends `Request::Hello` with a version other than `PROTOCOL_VERSION`, when the session receives it, then it answers `Response::Hello` carrying its own version and then closes the connection | |
| A14a | Given `XDG_RUNTIME_DIR` is absent, relative, or not a directory, when `realm-session` starts or a production client resolves the control socket, then it fails with `IpcPathError::MissingRuntimeDir`, never probes `/tmp`, and ignores `REALM_SOCKET` | Delegated to SPEC 0007 A1/A2: `realm_control::tests::runtime_capability_rejects_every_unsafe_input_and_openat2_failure`, `realm_control::tests::server_creates_realm_exactly_once_under_scoped_umask`, and `realm_control::tests::client_endpoint_missing_realm_is_retryable_and_creates_nothing` |
| A14b | Given a connecting peer with a uid other than the session's effective uid, or no readable credentials, when it is accepted, then the transport closes it before consuming a frame and a separately admitted same-uid peer remains intact | Delegated to SPEC 0007 A13 |
| A14c | Given every accepted connection state and frame error class, when input is read, then the response, exact deadline, and close/continue result match SPEC 0007's total table without affecting another peer | Delegated to SPEC 0007 A14-A16 |
| A14d | Given matching Hello plus `Subscribe` and optional clean read-half EOF, when the adapter accepts it, then the Hello drains first, the last visible accepted `Session::state()` is the immediate current event, later states use current-plus-latest coalescing, positive subscriber input closes, and shutdown cannot replace an unstarted initial State | Transport evidence belongs to SPEC 0007 A14/A14b/A16a; #38 owns the authoritative-state adapter |
| A14e | Given 64 admitted peers, a 65th peer, oversized/unterminated input or output, excess pipeline, a stalled subscriber, or a stalled ordinary client, when a SPEC 0007 bound is reached, then only the affected connection or bounded subscriber set closes; #38 still services pending/backend work first and checks backend readiness between bounded control quanta | Transport evidence belongs to SPEC 0007 A14-A17; #38 owns combined-loop evidence |
| A14f | Given a bound endpoint and incomplete or failed recovery, when startup runs, then the endpoint remains non-listening and no readiness is sent; given successful projection and transition to `Live`, then the bound capability is consumed once, the listener is verified active, and only then is `READY=1` sent; listen failure is fatal and sends no readiness | `realm_session::tests::readiness_follows_live_listener_activation` |
| A14g | Given any mix of transport states when logout begins, when #38 calls `begin_shutdown` repeatedly and drives only remaining subscriber interests/expiry, then no new connection/action appears, every non-subscriber closes, the first 100 ms deadline is fixed, later live completion/publication rejects as `ShuttingDown` without mutation while an already-stale id may remain stale, and River `exit_session` occurs only after `is_shutdown_complete()` | Transport transition evidence belongs to SPEC 0007 A16e; #38 owns the shutdown driver |
| A15 | Given an idle session, when the clock module's tick changes the clock text, then exactly one `Event::State` is broadcast and no `manage_dirty` and no other river request is made | `session::tests::module_change_emits_once_without_backend_apply` covers the in-process state effect; socket coverage remains SPEC 0007 |
| A16 | Given a module that recomputes to the text it already had, when derivation runs, then `revision` does not increment and no `Event::State` is sent | `session::tests::module_change_emits_once_without_backend_apply` |
| A17 | Given a client that quantises its dimensions down to a multiple of a 9×18 cell, when a triptych of three such clients is applied, then after at most one corrective `propose_dimensions` per window each `set_content_clip_box` equals that window's projected rect and the clip boxes tile the workarea exactly | |
| A18 | Given a successfully persisted ledger snapshot holding three windows across two orbits, their `BackendWindowId` mappings, and a next-`WinId` watermark, when the session is killed, all three identities replay, and the first manage sequence after restart completes, then each restored window is back in its snapshotted orbit and ledger position, persisted focus is restored when no new identity appears, otherwise the last report-order summon is focused, a newly reported identity receives the persisted next id rather than a reused id, immediate Undo is a no-op, and the first full `RealmState` has the ledger-derived fields from the snapshot, revision 1, `Mode::Nav`, empty chord/module state, and the default which-key state | |
| A19 | Given two observed and assigned windows, when a typed desired layout operation is staged and a backend that advertises the required capability as unsupported rejects its projection, then `SessionActionError::Backend(BackendError::Unsupported)` carries that capability name, the staged ledger and projection are not committed, and the visible state and revision remain unchanged | `session::tests::unsupported_apply_rolls_back_and_emits_no_state`; socket-frame coverage remains SPEC 0007 |
| A20 | Given projection P1 succeeded, an attempted projection P2 may have partially applied before returning an error, and authoritative state later projects to P1 again, when projection is retried, then both the session and backend treat their caches as dirty and the backend issues the complete P1 projection rather than suppressing it by equality with the last-successful cache | `session::tests::failed_apply_marks_projection_dirty_until_a_complete_repair`; the River backend cache contract remains part of #40 |
| A21 | Given a newly observed backend window identity and an error-atomic transient `assign_window` failure, when the event is handled and the outer loop retries before reading another backend event, then the ledger, metadata, stable mapping, and advanced non-reuse watermark survive the error, no state is published before binding succeeds, and retry binds the same `WinId` before applying and publishing it | `session::tests::failed_identity_binding_preserves_observed_window_for_retry` |
| A22 | Given two tiled windows, when `focus_step(Dir::Prev)` succeeds, then the focused id and title change, the revision advances once, and exactly one complete candidate projection is submitted; if that submission fails, the candidate ledger and state are rejected and the next repair submits the complete prior authoritative projection | `session::tests::focus_step_commits_one_projection_and_one_visible_state`, `session::tests::failed_focus_step_rejects_the_candidate_and_repairs_authoritative_state` |
| A23 | Given two tiled windows, when `swap(Dir::Prev)` succeeds, then their ledger order and focus change with exactly one projection submission; with fewer than two windows it submits no projection and emits no state | `session::tests::swap_changes_order_once_and_is_a_no_op_with_one_window` |
| A24 | Given any typed ledger operation in §9, when its candidate projection fails, then no candidate ledger, visible state, or revision is committed, and retry repairs the complete prior authoritative projection; a failed `undo()` does not consume history, and observed window lifecycle prevents Undo from changing the live window set | `session::tests::failed_undo_does_not_consume_history`, `session::tests::undo_never_removes_an_open_window_or_restores_a_closed_window`; A19 and A22 exercise the shared transaction boundary through other typed operations |
| A25 | Given a focused window, when `request_close_focused()` succeeds or fails, then it targets exactly that id and does not mutate the ledger, projection, visible state, or revision; the window is removed only when `WindowClosed` is observed | `session::tests::close_request_waits_for_the_observed_close_before_mutating_state` |
| A26 | Given no focused window, when `request_close_focused()` runs, then it makes no backend request and returns no target | `session::tests::close_request_is_a_no_op_without_a_focused_window` |
| A27 | Given a stable session, when `toggle_whichkey()` runs, then `whichkey` and the revision change once with no backend apply; only a later observed `WorkareaChanged` may re-project windows | `session::tests::whichkey_toggle_only_emits_state_until_workarea_is_observed` |
| A28 | Given a staged mutating socket request, when its backend transaction has not completed, then no ordinary reply is queued and the connection reads no second request; backend success commits and queues `Response::Ok`, while backend failure rejects the candidate and queues application `Response::Error` as an ordinary response, without blocking the event loop or writing before `manage_finish`. After draining either response, only a read-open peer returns Ready; a read-half-closed peer closes | Authoritative socket adapter coverage belongs to #38; reusable completion behavior belongs to SPEC 0007/#41 |
| A29 | Given absent, wrong-version, malformed, semantically invalid, and valid `SessionSnapshotV1` records, when they are classified, then absence and invalid content start fresh without partial state, valid content is accepted, and a non-`NotFound` read error is fatal | `session::tests::snapshot_validation_is_closed_and_total`, `snapshot::tests::classifies_completed_snapshot_reads_without_file_io`; pure classification is covered, while pathname/file-worker coverage remains in the event-loop binary slice |
| A30 | Given a valid snapshot and an initial replay, when restored, duplicate, and new identities arrive before `InitialReplayComplete`, then the first occurrence retains report order, latest metadata wins, each identity is assigned exactly once after the barrier, no projection/state/snapshot is produced early, and event order deterministically fixes new ids. Any non-replay event forbidden by §6 is a protocol error rather than deferred work | `session::tests::initial_replay_is_silent_and_rebinds_each_identity_once`, `session::tests::pre_barrier_non_replay_events_are_protocol_errors` |
| A31 | Given snapshotted identities that do not all reappear plus new identities, when `InitialReplayComplete` arrives, then missing windows are removed before new windows are summoned in report order, undo is empty, one forced complete projection succeeds, and exactly one revision-1 state is published before entering `Live` | `session::tests::replay_barrier_reconciles_then_publishes_once` |
| A32 | Given assignment or projection fails once, when pending backend work exists, then the library rejects an injected backend event and every desired/control publication path without a state transition or backend call, still exposes an authoritative Live persistence snapshot, exposes the read gate, and exactly one next-turn retry can complete the interrupted phase; if that retry fails, the session reports a restartable fatal error. The event-loop binary must separately prove that it does not call `next_event` while the gate is set | `session::tests::backend_work_gets_one_retry_and_gates_event_reads`, `session::tests::pending_backend_work_gates_actions_and_publication`, `turn::tests::backend_turn_retries_pending_work_without_readiness_before_a_later_read`; the pure readiness/read decision is covered, while poll integration remains in the event-loop binary slice |
| A33 | Given `InitialReplay`, `FinalizingReplay`, a live failed desired candidate, and a live observed change awaiting projection repair, when persistence is requested, then only the two live cases produce a snapshot and both contain authoritative state rather than a replay accumulator or rejected candidate | `session::tests::persistence_exposes_only_authoritative_live_state` |
| A34 | Given `next_win_id == u64::MAX` and an unknown identity buffered during replay, when the replay barrier stages reconciliation, then it is refused as exhausted without assigning `WinId(u64::MAX)`, changing the ledger, or publishing state | `session::tests::exhausted_watermark_never_allocates_the_sentinel` |
| A35 | Given snapshot A is in flight and later authoritative values B then C arrive, when A succeeds or fails, then neither completion clears C, only C is yielded next, the first dirty deadline never slides but is hidden while A is in flight, a failed latest value is retained with a 250 ms retry deadline, value reversions create no redundant write after their matching durable result is known, and shutdown yields the latest unpersisted value | `persistence::tests::older_completion_cannot_clear_the_latest_snapshot`, `persistence::tests::failed_latest_snapshot_rearms_from_completion`, `persistence::tests::queued_value_reverting_to_persisted_is_not_written`, `persistence::tests::in_flight_value_reversion_is_cleared_only_by_success`, `persistence::tests::failed_in_flight_write_clears_latest_value_already_persisted`, `persistence::tests::shutdown_yields_the_latest_unpersisted_snapshot` |

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md); no number here is new.

| Path | Budget | This component's share |
|---|---|---|
| Key press → new geometry submitted | **< 4 ms** | The whole of it. Measured from the `pressed` event being read off river's fd to `manage_finish` being flushed |
| State change → bar redraw | **< 8 ms** | The first part: derive, `renders_same_as`, encode, non-blocking write |
| Bar idle CPU | **~0%** | Nothing polls. The clock schedules the next minute boundary; one shared 1 Hz sampler runs off the input path; the key-repeat timer exists only while a key is held |
| Cold session start → usable | **< 900 ms** | Socket bound, six globals bound, ledger seeded or recovered, first `manage_finish` made |
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
from replacing a newer snapshot. Clean shutdown flushes the latest dirty live
snapshot. A worker failure leaves authoritative in-memory state untouched,
records degraded persistence, and permits a later retry. Snapshot capture occurs
after a desired transaction applies and commits, after an observed open/close
becomes authoritative even when its projection repair is pending, and after
successful initial finalization. It never captures a rejected desired candidate
or changes that affect only workarea, title, modules, mode, chord, which-key,
pending assignments, retry state, or projection caches. “Across a session”
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
§2 specifies `apply(&[Placement])` as "the visible set", so a hidden window
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
but it must not turn an unsupported operation into a silent success.

---

These resolutions make A1–A18 implementable without an unresolved product or
protocol decision. Later measurements may refine persistence cadence, inactive
orbit sizing, or liveness telemetry without changing the MVP behaviour above.
