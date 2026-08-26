# helm-layout reference

Exact contracts for `crates/helm-core/src/layout.rs` and
`crates/helm-core/src/ledger.rs`. Open this when you need a signature or are
picking which test to extend; the decision procedure lives in `SKILL.md`.

## Contents

- [Ledger types](#ledger-types)
- [Ledger mutations](#ledger-mutations)
- [Layout types](#layout-types)
- [Projection](#projection)
- [Tests and the invariant each one holds](#tests-and-the-invariant-each-one-holds)

## Ledger types

| Item | Contract |
|---|---|
| `ORBIT_COUNT: usize = 6` | Six orbits, runes `ᚠᚢᚦᚨᚱᚲ`. `palette.toml`'s `glyphs.runes` is rejected at parse time if it does not list exactly this many. |
| `HISTORY_DEPTH: usize = 64` | Undo snapshots kept. Oldest is dropped, never the newest. |
| `WinId(pub u64)` | Compositor handle. Opaque; helm never derives meaning from the number. |
| `OrbitId` | Private `u8`. `new(index)` and `from_human(1..=6)` return `Option`; `index()` is 0-based, `human()` is 1-based, `rune()` is the bar glyph, `all()` iterates all six. |
| `Dir::{Next, Prev}` | Direction for focus and swap. Wraps at both ends. |
| `Orbit` | `id`, `windows: Vec<WinId>` (index 0 is the master slot), `focus: Option<usize>`, `stowed: Vec<WinId>`, `layout`, `fullscreen: Option<WinId>`, `name`. No rectangle anywhere. |
| `Orbit::visible() -> Vec<WinId>` | Ledger order minus the stow set. This, not `windows`, is what a layout sees. |
| `Orbit::focused() -> Option<WinId>` | Resolves `focus` against `windows`. |
| `Orbit::occupied() -> bool` | Drives `OrbitDisplay::Occupied` in the bar. |
| `Ledger` | `orbits`, `active`, plus `history`/`redo` which are `#[serde(skip)]` — undo history is deliberately not part of the wire format. |

Read-only accessors: `orbit(id)`, `orbits()`, `active()`, `active_orbit()`,
`focused()`, `orbit_of(win)`, `len()`, `is_empty()`, `undo_depth()`.

## Ledger mutations

Every one of these checkpoints first, and every one of them must be a no-op that
does *not* checkpoint when it would change nothing.

| Method | Returns | Notes |
|---|---|---|
| `summon(win, orbit)` | `()` | Inserts **after the focused window**, not at the end, so a new window lands next to what spawned it. Summoning a known `WinId` again is ignored. |
| `banish(win) -> bool` | `false` if unknown | Focus is reseated to a neighbour inside the same orbit. |
| `focus_window(win) -> bool` | `false` if unknown | Also switches the active orbit if the window lives elsewhere. |
| `focus_step(dir)` | `()` | Wraps. Must not panic on an empty orbit. |
| `swap(dir) -> bool` | `false` at the ends or with nothing focused | Focus follows the window it moved. |
| `move_to_orbit(target) -> bool` | | Transfers the focused window and refocuses both orbits sensibly. |
| `switch_orbit(target)` | `()` | Changes `active` only. |
| `toggle_stow() -> bool` | | Removes from the projection, never from `windows`. |
| `toggle_fullscreen() -> bool` | | Sets `Orbit::fullscreen`; the projection then covers `Workarea::output`. |
| `set_layout(layout)` | `()` | Per-orbit, not global. |
| `undo() / redo() -> bool` | `false` when the stack is empty | Whole-snapshot restore. A new mutation clears the redo stack. |

## Layout types

| Item | Contract |
|---|---|
| `Rect { x, y, w, h: i32 }` | Output-local integer rectangle. `area() -> i64`, `intersects(&Rect)`, `contained_by(&Rect)`. |
| `Workarea::new(width, height, top_reserved, bottom_reserved)` | Produces `output` (whole screen, for fullscreen) and `tiles` (screen minus the 32px bar and 26px which-key strip). Negative results are clamped to zero. |
| `Layout::{Triptych, Mono, Even}` | `Triptych` is `Default`. `label()` feeds the bar's `⌗` indicator; `parse()` accepts the name plus an alias (`t`, `m`, `e`/`grid`); `next()` cycles for a single-key toggle. Serialises kebab-case over IPC. |
| `TriptychParams` | `master_ratio`, `stack_columns`, `stack_primary_ratio`, `primary_row_ratio`. Ratios, never pixels. Defaults reproduce the 1920x1080 mockup: `640/1920`, 2 columns, `700/1280`, `580/1022`. |
| `TriptychParams::sanitised()` | Clamps every ratio to `[0.15, 0.85]` and columns to `1..=4`. Call it before use; `project()` already does. |
| `Placement { win, rect, focused, occluded }` | The projection's output element. `focused` and `occluded` are paint hints only. |

## Projection

```rust
pub fn project(orbit: &Orbit, area: Workarea, params: TriptychParams) -> Vec<Placement>
pub fn partition(total: i32, weights: &[f32]) -> Vec<i32>
```

`project()` in order: fullscreen short-circuit (returns one placement covering
`area.output`), empty/degenerate short-circuit (returns `Vec::new()`), then
`match orbit.layout` for the rectangles, then zips them with `orbit.visible()`.

`partition()` is the only sanctioned way to divide a length. It returns
`vec![0; n]` for a non-positive total, treats an all-zero weight vector as
uniform, and never returns a negative part. Its parts always sum exactly to
`total`, and for uniform weights the largest and smallest differ by at most one.

Private helpers: `triptych(&[WinId], Rect, TriptychParams)` (master column plus
grid of the remainder), `even(n, Rect)` (`ceil(sqrt(n))` columns), and
`grid(n, Rect, cols, Option<&TriptychParams>)` — a short final row stretches to
full width, because gapless tiling has no concept of an empty cell.

## Tests and the invariant each one holds

Run one with
`cargo test -p helm-core layout::tests::<name>` (or `ledger::tests::<name>`).

### `layout::tests`

| Test | Holds |
|---|---|
| `partition_is_exact_and_stable` | Parts sum to the total, stay non-negative, and differ by at most one; the 1920 master split is exactly `[640, 1280]`; degenerate inputs do not panic. |
| `triptych_matches_the_reference_desktop` | Five windows at 1920x1080 reproduce the mockup's rectangles exactly, `640/700/580` wide and `580/442` tall. |
| `every_layout_tiles_exactly_for_every_plausible_size` | **The gate.** Triptych and Even, one to twelve windows, at 1920x1080, 2560x1440, 3840x2160, 1366x768 and 1281x801: exact coverage, no overlap, nothing degenerate, nothing escaping the workarea. |
| `single_window_owns_the_whole_workarea` | Every layout gives a lone window `area.tiles` — no special case leaks a margin. |
| `mono_stacks_and_marks_everything_but_the_top_occluded` | Mono gives every window the full area and marks exactly one non-occluded, and that one is the focused window. |
| `fullscreen_covers_the_output_including_the_bar` | Fullscreen uses `output`, not `tiles`. |
| `stowed_windows_are_not_projected` | Stowing removes a window from the projection while the remainder still tiles exactly. |
| `empty_orbit_and_degenerate_output_project_to_nothing` | Zero windows, or bars taller than the output, produce an empty vector rather than a panic or a negative rectangle. |
| `projection_is_pure_and_focus_only_moves_the_flag` | Two calls with identical inputs are equal, and moving focus leaves every rectangle byte-identical. |
| `absurd_params_are_clamped_rather_than_producing_negative_rects` | `master_ratio: 9.0`, `stack_columns: 99`, a negative ratio and `f32::NAN` still tile exactly. |

The shared helper `assert_exact_tiling(&[Placement], Rect)` is what encodes
invariant 3; a new tiling layout's test should call it rather than assert
individual rectangles.

### `ledger::tests`

| Test | Holds |
|---|---|
| `runes_cover_every_orbit` | Six orbits, six runes, `human()`/`index()` agree. |
| `summon_inserts_after_focus_not_at_the_end` | The insertion rule the master slot depends on. |
| `summoning_a_known_window_twice_is_ignored` | Idempotent summon. |
| `focus_wraps_in_both_directions` | `Dir::Next`/`Dir::Prev` wrap. |
| `focus_step_on_empty_orbit_does_not_panic` | Empty-orbit safety. |
| `swap_carries_focus_with_the_window` | Focus follows the moved window. |
| `banish_keeps_focus_inside_the_ledger` | No dangling focus index. |
| `move_to_orbit_transfers_and_refocuses` | Cross-orbit move leaves both ends consistent. |
| `stow_removes_from_projection_but_not_from_the_ledger` | Stow is a view concept. |
| `undo_restores_the_exact_previous_ledger` | Snapshot undo, not inverse ops. |
| `undo_history_is_bounded` | `HISTORY_DEPTH` is honoured. |
| `a_new_mutation_clears_the_redo_stack` | No branching timeline. |
| `no_op_mutations_do_not_consume_undo_depth` | Pressing `mod+1` twice does not cost an undo step. |
