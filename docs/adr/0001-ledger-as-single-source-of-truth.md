# ADR 0001 — The ledger is the single source of truth

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

The design handoff describes helm's window model in one sentence: "strict window
ordering per orbit/workspace; layouts are pure projections of the ledger onto
the workarea, never mutations. Undo = restore an earlier ledger."

That is unusual. Most tiling window managers keep authoritative geometry: a
window *has* an x, y, w, h, and a layout function mutates those numbers in
place. It works, and it has three costs helm cannot afford:

- **Undo becomes an inverse-operation log.** Every mutation needs a matching
  un-mutation, and the pair must stay correct as new operations are added. In
  practice the log drifts from the screen and undo starts producing states the
  user never saw.
- **Focus becomes a geometry event.** If focus can be an input to layout, moving
  focus can move pixels, and windows twitch as you navigate. The handoff asks
  for a focused pane background (`#0b0d16`) and a focused border, which are
  paint changes, not geometry changes.
- **Damage tracking becomes guesswork.** With mutable geometry there is no cheap
  way to know that nothing moved, so the compositor resubmits configures it did
  not need to.

helm's budget for "key press to new geometry submitted" is under 4 ms
(`docs/ARCHITECTURE.md` §4). That budget is only comfortable if relayout is
integer arithmetic over a short list with no allocation-heavy bookkeeping.

## Decision

`helm-core::ledger::Ledger` holds an ordered `Vec<WinId>` per orbit and nothing
else that describes position. Six orbits, each with its own layout, focus index,
stow list and optional fullscreen window. `TriptychParams` is an external
projection input: M0 passes it to `layout::project` and the ledger does not own
or persist it.

1. Every user action — summon, banish, swap, focus, stow, fullscreen, move to
   orbit, set layout — is a mutation of the ledger and of nothing else.
2. Geometry is produced by `layout::project(&Orbit, Workarea, TriptychParams)
   -> Vec<Placement>`, a pure function. No clock, no interior mutability, no
   I/O. The same inputs always produce byte-identical output.
3. Undo is a stack of whole ledger snapshots (`HISTORY_DEPTH = 64`), not an
   inverse-op log. `Ledger::checkpoint` runs before each mutation; no-op
   mutations do not checkpoint, so `mod+1` while already on orbit 1 does not
   burn an undo step. Undo history and the current `TriptychParams` are
   session-local and are not restored across a new login; no accepted
   requirement promises cross-session undo or persistent layout parameters.
4. `focused` and `occluded` are flags on a `Placement`. They may change what is
   painted; they may never change a rectangle.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Geometry as truth (i3, sway, bspwm, most of the field) | Well-trodden, easy to reason about locally, trivially supports mouse-dragged resizes and floating windows | Undo degrades into an inverse-op log that drifts; focus and geometry become entangled; "did anything change?" needs a full rect diff rather than a `Ledger` equality check |
| Scene graph as truth (a retained tree of nodes with computed layout, as toolkits do) | Natural fit for a Smithay compositor later; incremental invalidation comes free; handles nesting well | The tree becomes the state that must be serialised, undone and broadcast over IPC, and it is far larger than a `Vec<WinId>`. It also makes the *interesting* logic untestable without a compositor, which would have blocked M0 entirely |
| Ledger as truth, but cache the last projection as authoritative | Would let us mutate a rect for a drag-resize without a ledger change | Two sources of truth is the failure mode this ADR exists to prevent. Resize instead adjusts `TriptychParams`, which is an input to the projection |

The scene-graph option is not dead. It is what `helm-compositor` (M5) will build
*downstream* of the ledger, fed by the projection, rather than in place of it.

## Consequences

### Good

- Undo is exact by construction. `ledger::tests::undo_restores_the_exact_previous_ledger`
  asserts equality of the whole orbit, not of a summary.
- Focus is free. `layout::tests::projection_is_pure_and_focus_only_moves_the_flag`
  asserts that stepping focus leaves every rectangle identical.
- Relayout is skippable only when the cache identity contains the ledger,
  `TriptychParams`, and workarea. The ledger snapshots contain only ledger
  state; a future requirement to undo parameter changes must first extend the
  accepted ledger specification and its tests.
- The whole model is testable without a Wayland socket. `helm-core` has no I/O
  dependencies at all, which is why M0 landed before any compositor work.
- The IPC state broadcast is small: `HelmState` is a handful of strings and six
  orbit cells, not a geometry tree.

### Bad

- Floating windows do not fit. They are not in the ledger's shape and helm has
  no story for them beyond fullscreen and stow. Dialogs that expect to float
  will be tiled, which some applications handle badly.
- Mouse-driven arbitrary resize does not fit either. Resize must be expressed as
  a change to layout parameters, which is coarser than dragging a single edge.
- Snapshot undo costs memory proportional to `HISTORY_DEPTH` times ledger size.
  At 64 snapshots of a few dozen `u64`s this is irrelevant, but it would not be
  if the ledger ever grew to hold per-window state.
- Any state that genuinely is per-window and persistent (a remembered size, a
  user-pinned position) has nowhere to live without extending the ledger, which
  weakens the "order only" invariant.

### Neutral

- Stow is a second list rather than a flag on the entry. It keeps `windows` a
  clean order but means `visible()` is an O(n·m) filter. At six orbits and
  realistic window counts this is not worth optimising.
- The projection is recomputed rather than incrementally patched. For twelve
  windows this is cheaper than tracking what would need patching.

## Reversal

Structural. The ledger shape is assumed by `helm-core::layout`,
`helm-core::state`, `helm-core::ipc`, every `WmBackend` implementation, the bar's
redraw logic and the undo command. Reversing means rewriting the core crate and
every consumer of it: weeks, not days, and the result would be a different
desktop environment.

The signal to reconsider would be sustained, real demand for arbitrary floating
window placement — for instance if the DE turns out to be unusable with common
GTK dialogs. The cheaper response to that signal is a scratch "floating layer"
kept *outside* the ledger and composited above the projection, which preserves
this ADR. Try that first.

## Guard

- `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` — fails if
  focus ever becomes an input to geometry.
- `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` — five
  resolutions (including 1281×801) by twelve window counts by two layouts;
  fails if a projection stops covering the workarea exactly.
- `layout::tests::partition_is_exact_and_stable` — fails if the largest-remainder
  split ever loses or duplicates a pixel.
- `ledger::tests::undo_restores_the_exact_previous_ledger`,
  `ledger::tests::undo_history_is_bounded`,
  `ledger::tests::a_new_mutation_clears_the_redo_stack`,
  `ledger::tests::no_op_mutations_do_not_consume_undo_depth`.
- `state::tests::revision_alone_does_not_force_a_redraw` — fails if the
  no-change fast path stops working.
