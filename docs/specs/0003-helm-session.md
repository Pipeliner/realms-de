# SPEC 0003 — helm-session

- **Status:** Draft (2026-08-26) — see *Open questions* for what moves it to Accepted
- **Milestone:** M2
- **Decisions:** [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md),
  [ADR 0003](../adr/0003-session-daemon-owns-state.md),
  [ADR 0004](../adr/0004-ndjson-control-socket.md),
  [ADR 0009](../adr/0009-no-animation-budget.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md)
- **Implements:** [INTERFACES.md §1](../INTERFACES.md) (`WmBackend`) and
  [§4](../INTERFACES.md) (the socket's server half)
- **Supersedes / Superseded by:** —

> Written before the code, as S14 requires. The **Test** column below is
> deliberately empty: those tests get written next, watched to fail, and only
> then implemented against.
>
> **On verification.** Every claim below about river's behaviour is marked
> *(verified)* where it was read from the protocol XML at
> `codeberg.org/river/river`, branch `main`, `protocol/*.xml`, or *(assumed)*
> where it is an inference the XML does not state. ADR 0013 exists because a
> plausible summary of this protocol was wrong in three places; four further
> corrections are recorded in *Behaviour §2 and §3* below, and the ADR and
> `INTERFACES.md` need amending for them.

## Purpose

`helm-session` is the process that makes helm a desktop rather than a library.
It holds the one authoritative `Ledger`, derives the one `HelmState` every
client draws from, serves the control socket that makes the desktop scriptable,
and — under river 0.4 — *is* river's window manager. While it is running,
windows are where the ledger says they are and keys do what the keymap says.
While it is not, river has no window management at all: windows are unplaced,
unfocused and unresized, and no keybinding works. A user notices its absence
within one keystroke.

## Scope

**In:** connecting to river and negotiating protocol versions; owning `Ledger`
and deriving `HelmState`; driving a `WmBackend` and translating its events back
into ledger mutations; implementing the window-manager half of
`river-window-management-v1` and *serving* `river-layer-shell-v1`,
`river-xkb-bindings-v1` and `river-input-management-v1`; the keymap, the mode
machine and key repeat; the control-socket server and subscriber fan-out; the
right-hand bar modules; ledger persistence and restart recovery; client
lifecycle.

**Out:**

| Not this component's job | Whose it is |
|---|---|
| Drawing anything | `helm-bar` (SPEC forthcoming, [ADR 0008](../adr/0008-layer-shell-rendering-stack.md)); the bar is an ordinary `wlr-layer-shell` client |
| Template expansion, atomic writes, reload fan-out | `helm-theme` ([SPEC 0002](0002-theme-pipeline.md)); the session only *calls* `helm_theme::apply` and only off the input path |
| The ledger's mutation rules, the layout projection, colour maths, the wire types | `helm-core` ([SPEC 0001](0001-helm-core-contracts.md)) |
| The environment handshake, systemd ordering, portals, cursor theme | the session entry contract ([ADR 0011](../adr/0011-session-integration-contract.md)) and `packaging/` |
| The CLI surface | `helm-ctl`; it is a client of this socket and holds no privilege |

The boundary with `helm-core` is a rule, not a suggestion: `helm-session` must
not reimplement or second-guess ledger policy. When a control-socket request
arrives it calls the corresponding `Ledger` method and re-projects. If a
mutation's outcome looks wrong, the fix belongs in `helm-core` with a test,
not in a special case here. Two sources of window-order truth is the exact
failure [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md) exists to
prevent.

## Behaviour

### 1. Lifecycle

**Start-up order.** `helm-session` runs as `helm-wm`, started by
`helm-wm.service` after the environment import in ADR 0011 step 3. It
refuses to start without `WAYLAND_DISPLAY` (already enforced by the unit's
`ConditionEnvironment`).

1. **Bind the control socket first.** Determine the path with
   `helm_core::ipc::socket_path()`. If a socket file already exists, try to
   connect to it. If something answers, another session is live: exit non-zero
   with that message and change nothing. If nothing answers, the file is stale
   from a dead process: unlink it and bind. Binding before touching Wayland
   means a client that raced the session gets `ECONNREFUSED` and retries
   (ADR 0004 requires clients to have a retry loop) rather than hanging on a
   socket nobody is listening to.
2. **Connect to river and negotiate versions.** Bind, from the registry:

   | Global | Bind at | Refuse below | Because |
   |---|---|---|---|
   | `river_window_manager_v1` | 5 | **4** | `river_window_v1::identifier` and `river_window_manager_v1::exit_session` are `since="4"`; `set_content_clip_box` is `since="3"` *(verified)* |
   | `river_xkb_bindings_v1` | 3 | **3** | `modifiers_watch` / `modifiers_update` are `since="3"` and carry the mode badge and chord echo; `get_seat`, `ensure_next_key_eaten` and `ate_unbound_key` are `since="2"` *(verified)* |
   | `river_layer_shell_v1` | 1 | **1** | Its only version *(verified)* |
   | `river_input_manager_v1` | 2 | **1** | `set_repeat_info` is v1; the `done` event is `since="2"` *(verified)* |

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
   that helm supports layer shell: "If the window manager does not bind this
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
   `default` always exists and need not be created *(verified)*; helm does not
   create seats in M2. (ADR 0013's "Seat creation" is available but unused.)
6. **Create bindings and enter the loop.** §3 covers the keymap. The loop is
   §2's manage/render cycle plus the poll set in §4.
7. **Report ready.** Once the socket is bound *and* the first `manage_finish`
   has been made, the session is usable. `helm-wm.service` currently uses
   `Type=exec`; M2 switches it to `Type=notify` with `sd_notify READY=1` at
   exactly this point, so `helm-bar.service` can order after a session that
   answers.

**Shutdown.** `Request::Quit` and `Action::Quit` mean the user asked to log out.
Broadcast `Event::Shutdown` to every subscriber, flush, then make
`river_window_manager_v1::exit_session`, which is documented as being for
user-requested logout only *(verified)*. Exit 0 — the unit's
`Restart=on-failure` deliberately does not restart a clean exit, which is what
makes quitting possible.

### 2. The manage sequence, and which state is which

This is the part that is a protocol error rather than a style question. The
protocol declares "two disjoint categories of state" *(verified, verbatim)*,
and modifying either outside its sequence is
`river_window_manager_v1::error::sequence_order`.

The real loop, from the interface description *(verified)*:

```
  server: … state events …  manage_start
  helm:   window-management requests [+ rendering requests]  manage_finish
  server: sends new state to windows, waits for replies
  server: river_window_v1::dimensions × n           render_start
  helm:   rendering requests                        render_finish
  server: if dimensions changed again → back to render_start
          if WM-relevant state changed, or helm sent manage_dirty → back to manage_start
```

Two consequences the repo's current docs understate:

- **`INTERFACES.md` §1 says "`apply()` maps onto one `manage` sequence". That is
  half of it.** A single `apply()` spans a manage sequence *and* the render
  sequence that follows it, because the size half of a `Placement` is
  window-management state and the position half is rendering state. helm cannot
  send `manage_finish` and then immediately send positions; it must wait for
  `render_start`.
- **helm cannot start a manage sequence at will.** It can only ask, with
  `manage_dirty`, and river starts one "as soon as possible" *(verified)*. So a
  control-socket mutation is: mutate the ledger → `manage_dirty` → wait →
  apply in the sequence river then opens.

**The category table.** Every row was read from the request's own "This request
modifies … state" sentence *(all verified)*.

| helm operation | river request | Category |
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
better for helm rather than worse: a stowed window stays managed and stays in
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
So helm proposes the new size in the manage sequence and issues `show` only in
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
> surfaces *helm itself* creates through `get_shell_surface`, which do get a
> `river_node_v1` *(verified)*. helm's bar is a separate `wlr-layer-shell`
> client (ADR 0008), and `river-layer-shell-v1` exposes **no node object and no
> ordering request at all** — only `non_exclusive_area`, `set_default` and the
> three focus events *(verified)*. So bar-over-fullscreen is river's wlr-layer
> semantics, decided by the layer the bar picks, and is **not** helm's to order.
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
> For helm's own bar and which-key strip the two agree, and the struct's fields
> are public so the general case is expressible; but a left- or right-anchored
> layer surface from some other client cannot be expressed through
> `Workarea::new` and must be constructed field-wise.

helm must never compute the workarea from `metrics.bar_height` itself. The bar
declares its exclusive zone, river subtracts it, and helm learns the result.
Computing it locally is how helm and the bar come to disagree by 26 px the first
time someone toggles the which-key strip.

### 3. The five companion protocols helm must serve

Under river a window manager is not merely a client of the window-management
protocol. Five companion protocols are obligations, and helm ships broken
without them.

**`river-layer-shell-v1` — the bar exists because of this.** Covered in §1
step 3 and in the workarea note above. One further correction:

> **Correction to ADR 0013.** Its launcher row lists
> `river_layer_shell_seat_v1::focus_exclusive` / `focus_non_exclusive` /
> `focus_none` as things helm calls. They are **events**, not requests
> *(verified)*. helm does not grant a launcher exclusive focus; hecate (fuzzel
> in M2) asks for keyboard interactivity through ordinary `wlr-layer-shell`,
> river decides, and helm is *told*. What helm owes each event is a response:
> on `focus_exclusive`, stop trying to set focus — "all window manager requests
> to change focus are ignored" until it clears *(verified)* — and clear the
> focused-window border and `HelmState::focused_title`; on `focus_none`, return
> focus to `Ledger::focused()`.

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
a key not bound in the submap is pressed" *(verified, verbatim)*. helm's model
maps onto it exactly:

1. `mod+r` fires `Action::EnterMode(Mode::Resize)`. In the *same* manage
   sequence as the `pressed` event, helm swaps the enabled set as above and
   makes one `river_xkb_bindings_seat_v1::ensure_next_key_eaten`.
2. The next non-modifier key press is not delivered to the focused surface.
   - If it triggers an enabled binding, helm gets `pressed` on that binding,
     acts, and calls `ensure_next_key_eaten` again to stay in the submap.
   - If it does not, helm gets `ate_unbound_key`, and **leaves the submap**:
     mode back to `Mode::Nav`, Nav set re-enabled, `chord_echo` cleared, and
     no further `ensure_next_key_eaten`. The stray key is swallowed, not
     delivered — which is the point, since delivering a stray `x` into the
     user's editor is the failure this request exists to prevent.
3. `cancel_ensure_next_key_eaten` backs out a pending chord on a timeout. helm
   does **not** use a chord timeout in M2: the mode badge and chord echo make
   the pending state visible, `Escape` is bound to `EnterMode(Nav)`, and the
   protocol warns that a timeout needs a `manage_dirty` round trip and may race
   the `ate_unbound_key` event anyway *(verified)*.

Two honest notes on the current keymap. `Keymap::default()` has every binding
in `Mode::Nav`, so today `Mode::Resize` has an *empty* enabled set and the first
key pressed in it produces `ate_unbound_key` and exits immediately. That is a
gap in `helm-core`'s keymap, not in this design; the resize bindings are M2 work
in `helm-core` with their own test. And helm's chord model is presently one
modifier plus a mode rather than a multi-key prefix; a future `mod+g` then `w`
prefix uses the identical mechanism with a one-key submap.

**The mode badge and chord echo.** `modifiers_watch(mod4)` in a manage sequence
asks for `modifiers_update(old, new)` whenever `mod4` changes state *(verified)*,
which is how "hold `mod` to see the which-key strip" is implemented without
polling. Every such event opens a manage sequence, so holding a modifier costs
one round trip per press and one per release — bounded, and inside budget, but a
reason not to watch modifiers helm does not use.

**Key repeat is helm's job.** Because bound keys never reach a surface, river
does not repeat them; `river_xkb_binding_v1::stop_repeat` exists precisely to
tell a window manager "that has been repeating some action" to stop *(verified,
verbatim)*. So helm owns the repeat timer for held bindings, honouring the rate
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
finite." The enforcement is
`river_window_manager_v1::error::unresponsive`.

So: **input stops while helm thinks.** A stall is not a slow frame, it is a dead
session, and the frame budgets in
[ARCHITECTURE.md §4](../ARCHITECTURE.md) stop being comfort targets.

**Forbidden between `manage_start` and `manage_finish`, and between
`render_start` and `render_finish`:**

- any blocking `write(2)` to a control-socket client, however small;
- any filesystem access — no palette read, no template render, no
  `helm_theme::apply`, no ledger snapshot write, no `stat` of a spawn target;
- any process creation or `waitpid`;
- any D-Bus call, DNS lookup or network I/O;
- any mutex acquisition (there are none — see below);
- `HelmState` derivation, JSON encoding and fan-out, which are cheap but are
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
  bounded channel: `helm_theme::apply`, ledger snapshot writes, process spawning
  and any future D-Bus work. It reports back over a channel and signals the
  `eventfd`. The event loop never blocks on a send to it; a full worker queue
  drops the oldest pending duplicate of the same job (there is never a reason to
  queue two theme applies).
- **No locks.** A single owner means the input path cannot contend, which
  matters because a lock held for 40 ms is a stall nobody sees in review.

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
session derives `HelmState`, encodes it, or writes a byte to any subscriber.
This is the single rule that makes a wedged `helm ctl` unable to wedge the
desktop.

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
   still comes back short, helm accepts the reported dimensions, logs once per
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

A dead `helm-session` leaves windows unplaced and keys dead — a sharper failure
than a crashed bar, and one the user cannot work around. `helm-wm.service`
already specifies `Restart=on-failure`, `RestartSec=1` and a five-in-thirty-
seconds start limit; this section specifies what a restart must restore.

**`WinId` is not river's identifier.**

> **Correction to ADR 0013 and `INTERFACES.md` §1**, both of which map `WinId`
> onto `river_window_v1::identifier` and call it faithful. `WinId` is
> `pub struct WinId(pub u64)`; `identifier` is "a string that contains up to 32
> printable ASCII bytes" *(verified)*. They are different types. `helm-session`
> therefore maintains a bijection and allocates `WinId`s from a monotonic
> counter that is never reused within or across a session. The
> *non-reuse property* ADR 0013 relies on is real and does carry over — but it
> comes from helm's counter, backed by river's guarantee that the identifier it
> is keyed on never repeats *(verified: "The identifier must not be reused. This
> avoids races around window creation/destruction when identifiers are used in
> out-of-band IPC")*.

**The snapshot.** Written to `$XDG_RUNTIME_DIR/helm/ledger.json` — runtime
state, not configuration, because a ledger from last week's boot is worse than
none. Written by the worker thread, never the event loop, using the same
temp-file-plus-`rename(2)` discipline as [SPEC 0002](0002-theme-pipeline.md). It
contains the serialised `Ledger`, the `WinId → identifier` map, the active
orbit, and `PROTOCOL_VERSION` as a schema guard. A snapshot whose version does
not match is discarded, not migrated.

**Undo history does not survive a restart.** `Ledger`'s `history` and `redo`
fields are `#[serde(skip)]` *(verified in `crates/helm-core/src/ledger.rs`)*, so
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
| Snapshot identifier does not reappear | Drop it; the window closed while helm was dead |
| River reports a window not in the snapshot | Allocate a fresh `WinId` above the highest restored one and `Ledger::summon` it into the active orbit, in river's report order |

`Ledger::summon` inserts after the focused window, so the result is
deterministic given a deterministic report order.
`ledger::tests::summoning_a_known_window_twice_is_ignored` means the reconcile
pass is safe to run more than once.

**What a restart must *not* restore:** the input mode (reset to `Mode::Nav` — a
restart with a dangling `ensure_next_key_eaten` in the compositor would eat the
user's next keystroke), the chord echo, module state, and any pending worker
job.

**Clients.** Subscribers see EOF, retry, re-`Subscribe`, and get a full snapshot
as their first frame (§7). A crashed bar has never been able to take the session
with it (ADR 0003); this makes the reverse also survivable.

*(assumed, and it matters)* This all rests on river replaying a
`river_window_manager_v1::window` event for every already-existing window before
the first `manage_start` of a newly connected manager. The XML documents the
event only as "A new window has been created" and does not state the replay. A
manager that connected to a running compositor could not otherwise manage
anything, so the inference is strong — but it is an inference, and M2 must
confirm it against river's source or by experiment before A18 below can be
called passing. If river does *not* replay, restart recovery becomes impossible
as specified and the answer changes to "restart is a fresh session", which is
one of the open questions.

### 7. The control-socket server

Serves `helm_core::ipc` over the path from `ipc::socket_path()`, one JSON value
per line, encoded through `ipc::encode` and decoded through `ipc::decode`
([ADR 0004](../adr/0004-ndjson-control-socket.md)).

**Handshake.** `Request::Hello { version, client }` is always answered with
`Response::Hello { version: PROTOCOL_VERSION, session }` — including on
mismatch, so the client can print something useful — and on mismatch the
connection is then closed. **`Hello` is optional.** ADR 0004's own headline
example is `echo '{"cmd":"switch-orbit","arg":3}' | socat - $HELM_SOCKET` with
no handshake at all, and requirement 1 of that ADR is scriptability; a
connection that never says hello is served at `PROTOCOL_VERSION`. The M0 helper
continues to define its path/override/fallback behavior until an accepted IPC
specification replaces it.

**Deferred admission hardening.** Same-uid `SO_PEERCRED` checks, socket/directory
creation modes, and connection/frame/queue limits are M2 proposals. They cannot
alter the public M0 helper until SPEC 0001/0003 and failing tests define the
override-fixture and fallback behavior together.

**Requests.** A mutating `Request` is applied to the `Ledger` by calling the
corresponding `helm-core` method, after which the session makes `manage_dirty`
and answers `Response::Ok` **immediately** — `Ok` means "the ledger changed",
not "the frame is on screen". Coupling the reply to the compositor round trip
would put a `helm ctl` client on the input path, which §4 forbids.
`Request::ShowLedger` answers `Response::Ledger(Vec<OrbitLedger>)` built from
the ledger plus the per-window `app_id` and `title` last reported by river (both
nullable in the protocol *(verified)*, rendered as empty strings).
`Request::ReloadTheme` and `Request::Spawn` are handed to the worker and
answered `Response::Ok` on acceptance. A frame that does not decode is answered
`Response::Error { message }` and the connection stays open —
`ipc::tests::unknown_frames_are_an_error_not_a_panic` is the shape.

**Subscribers.** `Request::Subscribe` turns a connection into a subscriber. Its
first frame is an immediate `Event::State` snapshot, so a restarted bar draws at
once without a round trip; thereafter it gets exactly one `Event::State` per
change (§8).

**Back-pressure, and how a slow subscriber does not become a session failure.**
`Event::State` is a *snapshot*, which makes the correct queue depth one:

- Each subscriber has a single pending-frame slot plus a cursor into a
  partially written frame.
- On a new state, if a partial write is in flight it is finished first and the
  new frame replaces whatever is in the pending slot; otherwise the new frame
  goes straight to the slot. A subscriber that reads slowly therefore skips
  intermediate states and always converges on the latest one. It never queues.
- Writes are non-blocking. `EAGAIN` leaves the cursor where it is and waits for
  the fd to become writable in the next `poll`.
- If a subscriber's cursor has not advanced for `SUBSCRIBER_STALL_LIMIT`
  (2 seconds), it is closed with a log line naming the client from its `Hello`.
  A client that cannot read for two seconds is wedged, and the desktop does not
  wait for it.
- `Event::Shutdown` is the one frame that is not droppable; it is written with
  a short bounded deadline and the session exits regardless.

### 8. State derivation

`HelmState` is derived on the event-loop thread, after `manage_finish`, from the
ledger plus the session's own mode and module state.

| Field | Derived from |
|---|---|
| `orbits` | One `OrbitCell` per `OrbitId::all()`: `number = o.human()`, `rune = o.rune()`, `windows = ledger.orbit(o).windows.len()`, `display` = `Active` when `o == ledger.active()`, else `Occupied` when `Orbit::occupied()`, else `Empty` |
| `layout` | `ledger.active_orbit().layout` |
| `mode` | The session's mode machine (§3) |
| `focused_title` | The last `river_window_v1::title` for `ledger.focused()`; empty when nothing is focused, when the title is null, or while a layer surface holds exclusive focus |
| `chord_echo` | Non-empty exactly while a submap is pending or `mod4` is held; cleared on `ate_unbound_key`, on leaving the submap, and on restart |
| `whichkey` | Toggled by `Action::ToggleWhichKey`. Changing it changes the bar's exclusive zone, so the new `Workarea` arrives from river as a `non_exclusive_area` event rather than being computed here |
| `modules` | Owned by the session. Push-driven except the clock, which ticks once a second |
| `revision` | See below |

**When `revision` increments.** The session derives a candidate state and
compares it with the last broadcast one using `HelmState::renders_same_as`,
which ignores `revision` by construction. If they render the same, **nothing
happens**: no increment, no encode, no socket write, no wake-up for any
subscriber. If they differ, `revision += 1` and one `Event::State` goes to every
subscriber. So `revision` counts *visible* changes, monotonically and without
reset, and `state::tests::revision_alone_does_not_force_a_redraw` describes the
bar's belt-and-braces check rather than the primary gate — the primary gate is
here, one process upstream, where it also saves the serialisation and the
syscall.

**A module update never touches river.** The clock tick, a CPU sample or a
battery change alters no window-management and no rendering state, so it must
not make `manage_dirty`. A session that took a compositor round trip once a
second would multiply its own input latency for a clock.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a river advertising `river_window_manager_v1` v5, `river_xkb_bindings_v1` v3, `river_layer_shell_v1` v1, `river_input_manager_v1` v2, `river_xkb_config_v1`, and `river_libinput_config_v1`, when `helm-session` starts, then it binds all six, seeds six orbits with orbit 1 active, and reports `Capabilities` with `exact_geometry`, `server_side_borders`, `hide_show`, `explicit_ordering` and `fullscreen` all true and `unsupported` empty | |
| A2 | Given a river advertising `river_xkb_bindings_v1` at version 2, when `helm-session` starts, then it exits non-zero with a message naming the interface, the version advertised and the version required, and makes no window-management request | |
| A3 | Given a projection placing two windows, when `apply` runs, then every `propose_dimensions` is sent before `manage_finish`, every `set_position` is sent after `render_start` and before `render_finish`, and no `set_position` is sent inside the manage sequence | |
| A4 | Given two tiled windows, when `Swap(Dir::Next)` is applied, then both windows' new positions are sent in a single render sequence terminated by exactly one `render_finish` | |
| A5 | Given a projection already applied, when `apply` runs again with identical placements, then no `propose_dimensions`, no `set_position` and no `manage_dirty` request is made | |
| A6 | Given three tiled windows, when `Focus(Dir::Next)` is applied, then no `propose_dimensions` is sent and the only requests are `focus_window` and two `set_borders` | |
| A7 | Given a tiled window, when it is stowed, then `hide` is sent inside a render sequence, no `propose_dimensions` is sent for it, and the window is still present in `Response::Ledger` | |
| A8 | Given `helm-session` bound to `river_layer_shell_v1`, when a `wlr-layer-shell` client maps a top-anchored surface with a 32 px exclusive zone, then the surface is not closed and the resulting `Workarea` has `tiles.y == 32` and `tiles.h == output height − 32` | |
| A9 | Given `Keymap::default()`, when the first manage sequence completes, then one `river_xkb_binding_v1` exists per binding, each created with the xkbcommon keysym of its `Binding::key`, and exactly the `Mode::Nav` bindings are enabled | |
| A10 | Given `Mode::Nav` and the binding for `r`, when its `pressed` event arrives, then in that same manage sequence the `Mode::Resize` set is enabled, the `Mode::Nav` set is disabled, and exactly one `ensure_next_key_eaten` request is made | |
| A11 | Given `Mode::Resize` with `ensure_next_key_eaten` outstanding, when `ate_unbound_key` arrives, then the mode returns to `Mode::Nav`, the Nav set is re-enabled, `chord_echo` is empty and no further `ensure_next_key_eaten` is made | |
| A12 | Given a subscriber that has stopped reading and whose socket buffer is full, when a bound key is pressed, then `manage_finish` is made within the key-press budget and before any byte is written to that subscriber | |
| A13 | Given a subscriber whose write cursor has not advanced for `SUBSCRIBER_STALL_LIMIT`, when the next state change occurs, then that subscriber is closed and every remaining subscriber receives exactly one `Event::State` | |
| A14 | Given a client that sends `Request::Hello` with a version other than `PROTOCOL_VERSION`, when the session receives it, then it answers `Response::Hello` carrying its own version and then closes the connection | |
| A15 | Given an idle session, when the clock module's tick changes the clock text, then exactly one `Event::State` is broadcast and no `manage_dirty` and no other river request is made | |
| A16 | Given a module that recomputes to the text it already had, when derivation runs, then `revision` does not increment and no `Event::State` is sent | |
| A17 | Given a client that quantises its dimensions down to a multiple of a 9×18 cell, when a triptych of three such clients is applied, then after at most one corrective `propose_dimensions` per window each `set_content_clip_box` equals that window's projected rect and the clip boxes tile the workarea exactly | |
| A18 | Given a session holding three windows across two orbits which is killed and restarted while all three windows survive, when the first manage sequence after restart completes, then each window is back in its recorded orbit and ledger position, focus is restored, and the broadcast `HelmState` equals the pre-crash one apart from `revision` | |

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md); no number here is new.

| Path | Budget | This component's share |
|---|---|---|
| Key press → new geometry submitted | **< 4 ms** | The whole of it. Measured from the `pressed` event being read off river's fd to `manage_finish` being flushed |
| State change → bar redraw | **< 8 ms** | The first part: derive, `renders_same_as`, encode, non-blocking write |
| Bar idle CPU | **~0%** | Nothing polls. The clock schedules the next minute boundary; one shared 1 Hz sampler runs off the input path; the key-repeat timer exists only while a key is held |
| Cold session start → usable | **< 900 ms** | Socket bound, six globals bound, ledger seeded or recovered, first `manage_finish` made |
| `helm ctl theme apply` | **< 150 ms** | Runs on the worker thread. The budget is [SPEC 0002](0002-theme-pipeline.md)'s; what this spec adds is that it must not appear on the input path |

**Which of these become correctness bounds under river, and why.**

The **4 ms** bound does, unambiguously. river "should wait for the manage
sequence to complete before processing further input events" and its input
buffer "is finite" *(verified)*, with
`river_window_manager_v1::error::unresponsive` as the enforcement. Exceeding it
is not a slow desktop, it is dropped keystrokes and then a disconnected window
manager. The measurement changes with it: the number to hold is a **worst case
under adversarial conditions** — a wedged subscriber, a theme apply in flight, a
window opening — not a median on an idle machine. ADR 0013's planned M2 liveness
test is exactly that scenario.

The **idle CPU** bound becomes semi-correctness for a second-order reason: every
`modifiers_update` and every binding press opens a manage sequence, so any
periodic `manage_dirty` would sit in front of the user's next keystroke.
"Event-driven" stops being about battery life and starts being about latency.

The other three keep their original character.

**What the protocol does not tell us.** The XML declares the `unresponsive`
error but names no threshold; that is river's implementation policy, not
protocol *(verified — the string appears exactly once, in the error enum)*. So
4 ms is helm's budget, not river's limit, and the relationship between them is
unmeasured. See *Open questions*.

## Failure modes

From [PITFALLS.md](../PITFALLS.md). This component **owns** every row of the
"Being the window manager (river)" section:

| Row | Guard specified here |
|---|---|
| A client quantises its proposed size | §5: propose, observe the shortfall, re-propose once, `set_content_clip_box` to the exact tile, give up loudly rather than loop. A17 |
| `helm-session` stalls | §4: one owner, no locks, non-blocking writes, all filesystem and process work on the worker, `manage_finish` before any subscriber byte. A12, A13 |
| `helm-session` dies | §6: snapshot to `$XDG_RUNTIME_DIR/helm/ledger.json`, identifier-keyed reconciliation on restart, supervised by `helm-wm.service`. A18 |
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
coordinates and hands the bar its scale), and "half-applied theme" (SPEC 0002
owns atomicity; this component owns only not blocking on it).

## Open questions

**1. Ledger persistence: per-mutation or periodic?**
Per-mutation is exact — a crash loses nothing — but puts a worker job behind
every keystroke and writes to `$XDG_RUNTIME_DIR` at input rates. Periodic (say
every 2 s, and always on a clean shutdown) is cheap but loses the last few
mutations. A third option is per-mutation with coalescing: the event loop marks
the ledger dirty and the worker writes at most once every 250 ms.
*Recommendation: the third.* It is bounded work, it never blocks the event loop,
and the worst case is a quarter-second of lost window moves after a crash —
which is less than the user will lose noticing the restart.

**2. What happens to windows that existed before a restart?**
§6 specifies identifier-keyed reconciliation, and that is the right answer *if*
river replays a `window` event per existing window to a newly connected manager.
The XML does not say it does *(assumed, flagged in §6)*. The alternatives if it
does not: (a) a restart is a fresh session — windows are summoned into the
active orbit in whatever order river reports and the user reassembles their
layout by hand; (b) helm keeps a `river_window_v1` object alive across the
restart, which is impossible since the Wayland connection dies with the process.
*Recommendation: confirm the replay against river's source before implementing
§6; if it does not replay, ship (a) and say so plainly in the release notes.*
This must be settled before A18 can be written honestly, which is one of the two
reasons this spec is Draft.

**3. How much of `Capabilities` can river actually populate — and what does
`exact_geometry` mean?**
Four of the five fields are unambiguous at v5: `server_side_borders`,
`hide_show`, `explicit_ordering` and `fullscreen` are all true, from
`set_borders`, `hide`/`show`, `place_*` and `fullscreen` *(all verified)*.
`exact_geometry` is the problem, because it can mean "helm can place a window at
an arbitrary rect" (true) or "the window will be that size" (false, always, on
every compositor). With §5's clipping the *rendered* rectangle is exact while
the client's own buffer is not.
*Recommendation: define `exact_geometry` in `INTERFACES.md` as "the rendered
rectangle is exactly the projected rectangle", report it `true` when
`river_window_v1` is at version ≥ 3 and `false` below, and push
`"unclipped-dimension-quantisation"` into `unsupported` in the `false` case.*
This is a documentation change to a committed interface, so it wants a second
opinion rather than a unilateral edit.

**4. Should helm eagerly size windows in inactive orbits?**
§2 specifies `apply(&[Placement])` as "the visible set", so a hidden window
keeps its old size until it is shown, and the show-after-dimensions rule can
cost an extra render sequence on the first orbit switch after a workarea change.
The alternative is to project all six orbits and propose dimensions for every
window whenever the workarea changes, making every orbit switch exactly one
frame. Projection is pure integer arithmetic over short lists, so six of them is
not the cost; the cost is that `apply`'s slice would have to carry a hidden flag
or the seam would need a second method.
*Recommendation: ship the simple contract in M2 and measure. If the first switch
into an orbit visibly lags after a resolution change, revisit — and revisit it
in `INTERFACES.md`, not with a special case here.*

**5. What is river's actual `unresponsive` threshold, and how does it relate to
4 ms?** The protocol declares the error and names no number *(verified)*. helm's
budget is 4 ms; river's tolerance might be 50 ms or 5 s, and helm's worst case
under an adversarial subscriber is presently unmeasured. Until someone reads
river's source or asks upstream, the M2 liveness test can assert helm's budget
but cannot assert that helm never trips river's error.
**`needs-human`** (standing order S3): someone must read `river`'s
implementation or ask its maintainer, and then decide whether helm needs its own
watchdog — a self-imposed deadline after which the session logs, abandons the
in-flight work and finishes the sequence with whatever it has, rather than being
disconnected. This is the second reason this spec is Draft: it changes what A12
asserts.

**6. `helm-wm.service`: `Restart=on-failure` or `Restart=always` plus an
explicit success status?** The unit file already carries this as a `needs-human`
note deferred to M2, and this spec does not close it: it depends on whether
`helm-wm` can exit 0 for any reason other than a deliberate quit, which
needs a daemon to observe. Recorded here so the two places agree.
**`needs-human`.**

---

**To reach Accepted**, questions 2 and 5 need answers, because they change
acceptance criteria rather than implementation detail. Questions 1, 3, 4 and 6
have recommendations that are safe to build against and can be settled by the
implementation.
