# ADR 0013 — helm is the window manager, on river's window-management protocol

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers, repo owner
- **Supersedes / Superseded by:** **Supersedes [ADR 0002](0002-borrow-a-compositor-first.md)**

## Context

[ADR 0002](0002-borrow-a-compositor-first.md) chose niri and recorded, honestly,
that niri's scrollable-tiling model is not a ledger model: triptych, even, stow
and undo were all marked **lossy**, and the ADR flagged two `needs-human`
questions about what to do with the gaps. It also considered river and rejected
it, on the grounds that `river-layout-v3` delivers dimensions only.

That reasoning was correct about `river-layout-v3` and is now obsolete.

**river 0.4.0 removed window management from the compositor entirely.** An
external window manager process drives it over `river-window-management-v1`, a
different and far larger protocol than the layout protocol 0002 assessed. The
compositor keeps rendering, input and the low-level plumbing; layout, focus,
ordering, stacking, borders and keybindings all move out.

The shapes match. helm mutates an ordered ledger, projects it to exact integer
rectangles, and wants those applied atomically with no intermediate frame
(ADR 0001). The protocol is globally double-buffered around a two-phase
`manage` → `render` sequence whose stated purpose is "frame perfect state
changes involving multiple windows". A ledger swap becomes two `set_position`
requests in one render sequence, applied in one frame.

The consequence is a change of identity, not just of backend. `helm-session`
does not talk to a window manager any more. It **is** the window manager. The
Zig objection from 0002 dissolves with it: river becomes a package we depend
on, not code in our workspace.

## Decision

1. Phase 1 targets `river-window-management-v1`. `helm-session` implements the
   window manager side of it directly, behind the existing `WmBackend` seam.
2. `NiriBackend` is dropped. `NativeBackend` against `helm-compositor` (M5) is
   unchanged as the long-term destination.
3. **Three companion protocols are obligations, not options.** The brief that
   prompted this ADR listed only the window-management protocol; verification
   against river's `protocol/` directory found that a usable helm needs all of:

   | Protocol | Why helm cannot ship without it |
   |---|---|
   | `river-window-management-v1` (ifaces at v5) | Layout, focus, ordering, borders, fullscreen |
   | `river-layer-shell-v1` (v1) | **The bar does not work at all otherwise.** river supports `wlr-layer-shell` only if the window manager implements this. `helm-bar` and `helm-hecate` stay ordinary `wlr-layer-shell` clients (ADR 0008 is unaffected); `helm-session` must serve the manager half |
   | `river-xkb-bindings-v1` (v3) | The entire keymap, and the chord model specifically |
   | `river-input-management-v1` (v2) | Seat creation, keyboard repeat rate |

4. Packaging vendors a pinned river 0.4.x. Ubuntu 24.04 and Fedora 41 ship
   0.3.x or river-classic, neither of which speaks the protocol. *(Distro
   versions are the repo owner's finding, recorded here as an assumption for
   the packaging CI to confirm, not as a verified fact.)* The flake pins the
   same input, consistent with [ADR 0010](0010-nix-flake-as-reference-build.md).

## Mapping: helm onto `river-window-management-v1`

This table replaces the niri mapping table in ADR 0002. Every row was checked
against the protocol XML rather than against a summary; three rows below correct
the description helm was working from.

| helm concept | river mechanism | Fidelity |
|---|---|---|
| Orbit (6, fixed, runes ᚠᚢᚦᚨᚱᚲ) | None. helm holds all six ledgers in process and renders one; windows in inactive orbits are `hide`-den | **Faithful.** Better than niri, where orbits had to be bent onto dynamic workspaces we did not control |
| Ledger order | helm projects it itself and expresses the result as `river_node_v1::set_position` plus `river_window_v1::propose_dimensions` per window | **Faithful** — but see the correction below: the ledger is *not* the render list |
| Render/stacking order | `place_top`, `place_bottom`, `place_above(other)`, `place_below(other)` | **Faithful.** Used for mono occlusion and overlay stacking, not for tiling order |
| `focus_step(Next/Prev)` | `river_seat_v1::focus_window`; wrapping is helm's own logic | **Faithful.** No end-of-strip special case, unlike niri |
| `swap(Dir)` | Ledger swap, re-project, two `set_position` calls in one render sequence | **Faithful and frame-perfect** — one frame, no intermediate state |
| Triptych, Even | `set_position` + `propose_dimensions` in absolute logical coordinates | **Faithful**, subject to the quantisation caveat below |
| Mono | Every tile the same rect; `place_top` on the focused window | **Faithful.** `Placement::occluded` becomes a real compositor property |
| Stow | `river_window_v1::hide` / `show` | **Faithful, natively.** And the semantics match exactly: `hide` is *rendering* state, so a stowed window stays managed and stays in the ledger, which is precisely what `Orbit::stowed` means |
| Fullscreen (`mod+f`) | `fullscreen(output)` plus `inform_fullscreen`. Shell surfaces above the fullscreen window still render, so bar visibility over fullscreen is our choice of node order | **Faithful** |
| Workarea | `river_layer_shell_output_v1::non_exclusive_area` → `Workarea::tiles`; `river_output_v1::position`/`dimensions` → `Workarea::output` | **Faithful, and direct.** This is `Workarea::new(w, h, top, bottom)` arriving as an event |
| 1px seams | `set_borders(edges, width, r, g, b, a)`, drawn by the compositor, premultiplied RGBA | **Faithful, and better than niri.** The design's seams become real borders. ADR 0005's decision to store alpha beside each colour pays off directly here |
| Undo | Restore a ledger snapshot, re-project, apply in one render sequence | **Faithful.** This was "lossy by construction" on niri, needing a visible replay. It is now one atomic frame |
| Chords (`mod` prefix, submaps) | `ensure_next_key_eaten` + `ate_unbound_key` | **Faithful, and purpose-built.** The protocol's own rationale names chorded bindings and submap exit as the reason this request exists |
| Mode badge, chord echo | `modifiers_watch` + `modifiers_update(old, new)` | **Faithful** |
| Keymap | `river_xkb_bindings_v1::get_xkb_binding(seat, keysym, modifiers)`, `enable`/`disable` per binding | **Faithful.** Per-mode keymaps are enable/disable sets |
| `WinId` | `river_window_v1::identifier` — up to 32 printable ASCII bytes, unique, never reused | **Faithful.** Non-reuse is what makes it safe in `helm ctl ledger show` (ADR 0004) |
| Launcher focus | `river_layer_shell_seat_v1::focus_exclusive` / `focus_non_exclusive` / `focus_none` | **Faithful** |
| **Exact tiling** | `propose_dimensions` is a *proposal*. A window may take different dimensions and reports them back via `dimensions` | **Approximate — the one real gap that remains.** See below |

### Three corrections to the description this decision was made on

1. **`place_*` orders the render list, not the ledger.** The ledger's order is
   *layout* order, which helm computes and expresses through positions and
   dimensions. For gapless tiling, where nothing overlaps, render order is
   nearly irrelevant; it matters for mono and for overlays. Treating the render
   list as the ledger would be a category error.
2. **`hide`/`show` are rendering state, not window-management state.** This is
   better for us, not worse, and is why stow maps exactly.
3. **The protocol is declared stable**, not unstable. See *Needs a human*.

### The remaining gap: dimension quantisation

`propose_dimensions` explicitly says the window "may not take the exact
dimensions proposed", giving a terminal quantising to its cell size as the
example. helm's central invariant is that projected rectangles cover the
workarea exactly, with no cracks
(`layout::tests::every_layout_tiles_exactly_for_every_plausible_size`). A
terminal that rounds down leaves the void showing, which is the first pitfall in
`docs/PITFALLS.md`.

This is not a river problem; it is true of every compositor, and niri had it
too. river gives us a tool for it that niri did not:
`set_content_clip_box` clips window content to a box, and borders are drawn
around the intersection of the content and that box. So helm can propose a size
at or above the tile and clip to the exact projected rect, keeping the seams
where the projection put them. Whether that reads acceptably for a terminal
whose last row is clipped is an empirical question for M2.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Stay on niri** (ADR 0002's decision) | Maturity and packaging, and both are real. niri is a settled, widely used compositor, packaged in the distros we target, with a stable release history; river 0.4.0 is days-old by comparison. Staying costs nothing new, keeps `NiriBackend` work already scoped, and avoids vendoring a compositor. The niri approximation may also have been good enough: most of what a user touches in a week is orbits, focus and fullscreen, all of which mapped well | It cannot express the product. Triptych is the reference desktop; on niri it is not achievable, and stow and undo were lossy. ADR 0002 had to open two `needs-human` questions about which compromise to ship. river makes all of them disappear rather than choosing between them. The owner weighed maturity against fidelity explicitly and chose fidelity |
| **Both backends behind `WmBackend`** | The seam already exists precisely for this; users on Ubuntu and Fedora could run helm on packaged niri today with no vendoring, while river users get the faithful desktop; it hedges the bet on a very new release | Two window models to keep correct, and they are not the same shape: one is a full window manager implementation, the other a projection onto someone else's model with known gaps. Every layout change would need testing twice, and the niri path would permanently produce a different desktop. It also doubles the surface at the point in the project with the least capacity to test it. The owner rejected this explicitly |
| **Pull `helm-compositor` (Smithay) forward** | No external dependency, no vendoring, no protocol to track, and it is the stated destination anyway. If we are writing a window manager regardless, writing the compositor around it is a smaller step than it was | Still twelve months of plumbing that river already provides: DRM, input, XWayland, session lock, screencopy, fractional scaling. river's split means we get to write the *interesting* half now and defer the rest. Rejected explicitly for the same reason as in ADR 0002: it violates the MVP cut line |

## Consequences

### Good

- Every row ADR 0002 marked lossy becomes faithful. The reference desktop from
  `Desktop v3.dc.html` is achievable in phase 1, not deferred to M5.
- Atomic, frame-perfect application matches ADR 0001 exactly. Undo is one frame.
- Compositor-drawn borders mean the 1px seams are real rather than approximated
  inside each window.
- The chord model has protocol support built for it. `ensure_next_key_eaten`
  and `ate_unbound_key` are the submap mechanism helm would otherwise have had
  to fake.
- Writing a real window manager against a documented protocol is directly
  transferable work: it is most of what `NativeBackend` will need at M5.

### Bad

- **A stall is now a session failure, not a slow frame.** The protocol has an
  `unresponsive` error, and `modifiers_update` warns that "the capacity of the
  compositor to buffer incoming input events is finite". `helm-session` sits on
  the compositor's input path with a hard liveness requirement. Nothing in it
  may block: not a theme apply, not a socket write to a wedged subscriber, not a
  slow `helm ctl` client. This sharpens ADR 0003's single-point-of-failure and
  ADR 0009's budgets from performance goals into correctness requirements.
- If `helm-session` dies, river has **no** window management at all. Under niri
  a dead session left a usable compositor; here it leaves windows unmanaged.
  Restart policy stops being hygiene and becomes essential.
- Vendoring a compositor is a real packaging burden: a pinned Zig build in the
  deb and rpm, rebuilt on every river security fix.
- river 0.4.0 is very recent and extremely breaking. The `river-classic` fork
  exists specifically for users who did not want this change, which is a signal
  about migration friction and about third-party tooling that assumes the old
  model.
- We implement four protocols, not one — a larger phase-1 surface than ADR 0002
  scoped, with `river-layer-shell-v1` on the critical path for the bar
  appearing at all.
- The dimension-quantisation gap above is unresolved and needs an M2 experiment.

### Neutral

- `session_locked` / `session_unlocked` events arrive on the window manager,
  which gives helm a hook to restrict bindings while locked. This is useful to
  ADR 0011's lock-screen question but does not settle it.
- Only one window-management client may connect; river sends `unavailable`
  otherwise. Fine for a session that owns the machine.
- `exit_session` exists and is explicitly for user-requested logout only, which
  maps onto `Request::Quit`.

## Reversal

**Back to niri:** medium, and it gets more expensive over time. `helm-session`
would revert from being a window manager to being a client of one, which is a
different architecture rather than a different module. ADR 0002's mapping table
is preserved precisely so that this path stays documented. Estimated two to
three weeks, and the result is the lossy desktop 0002 described.

**Forward to Smithay:** unchanged, and cheaper than before. The window
management logic written against river is the logic `NativeBackend` needs; what
`helm-compositor` adds is the plumbing river currently provides. Writing a
window manager first is a strictly better position than ADR 0002 left us in.

Signals to reconsider: river breaking the protocol despite the pledge below;
river 0.4.x failing to reach the target distros so that vendoring becomes
permanent; or the quantisation gap proving unfixable and visibly breaking
gapless tiling.

## Guard

- *Planned (M2):* the seam guard, updated from ADR 0002 — a CI grep asserting
  that `river` appears nowhere in the workspace outside
  `crates/helm-session/src/backend/`, `packaging/` and `docs/`. `niri` must now
  appear nowhere outside `docs/`.
- *Planned (M2):* a headless-river integration test driving a scripted sequence
  of ledger mutations and asserting the resulting positions and dimensions
  equal `layout::project`'s output exactly. This makes
  `layout::tests::triptych_matches_the_reference_desktop` an end-to-end
  assertion rather than a unit one.
- *Planned (M2):* a protocol-version test asserting the interface versions helm
  binds against are those of the pinned river, so a vendored bump that moves
  them fails the build rather than the session.
- *Planned (M2):* a liveness test asserting `helm-session` completes a
  `manage`/`render` round trip within budget while a subscriber is deliberately
  wedged, guarding the `unresponsive` failure above.
- *Planned (M2):* a quantisation test tiling a cell-quantising terminal and
  asserting exact coverage with clipping applied.
- `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` and
  `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` remain the
  definition of correct geometry, unchanged by the backend.

## Needs a human

**The protocol's stability classification — and the brief was wrong about it.**
helm was asked to record `river-window-management-v1` as "registry-classified
unstable, against which the maintainer pledges not to break window managers",
and to cite both without picking. Checking the sources, those are not two live
positions:

- Issue [#1299](https://codeberg.org/river/river/issues/1299) describes the
  protocol as WIP and warns that "the 0.4.0 release will be extremely breaking".
  That issue **predates 0.4.0** and its final comment records the protocol as
  landed.
- The 0.4.0 announcement states plainly: "the river-window-management-v1
  protocol is stable, we do not break window managers", and "all window managers
  written for river 0.4.0 will remain compatible with river 1.0.0 and beyond".
- The protocol XML carries no instability marker: no `z` prefix, no `unstable`
  in the interface names, no `unstable/` directory. The `-v1` is a protocol
  namespace, and the interfaces are already at version 5.

So the honest reading is that the protocol is **declared stable by its author,
backed by a forward-compatibility pledge, but is pre-1.0 in a compositor whose
previous release cycle was extremely breaking.** The residual risk is
reputational trust in a single maintainer, not a formal instability marker.

**What a human must decide: what we do if the pledge does not hold.**

1. **Pin and follow.** Vendor a known-good river, upgrade deliberately, accept
   occasional porting work. Cheapest; leaves us exposed to a hard break.
2. **Pin and fork on break.** Same, but commit in advance to forking river at
   the last good version if the protocol moves under us. Expensive insurance
   in a language we do not otherwise use.
3. **Accelerate `helm-compositor`.** Treat river as explicitly temporary and
   fund M5 sooner, so a break is an inconvenience rather than a crisis.

**Recommendation: option 1, with option 3 as the standing mitigation.** The
pledge is explicit and public, the interfaces are already at version 5 without
having broken their own numbering, and ADR 0002's core insight still holds: the
compositor is not the product. What makes this materially safer than 0002's
position is that the window management logic is now ours regardless of who
renders it.

A human should also confirm the **distro version claim** in the Decision above
before packaging work starts, since the vendoring decision rests on it.

Tracked as a `needs-human` issue (standing order S3).
