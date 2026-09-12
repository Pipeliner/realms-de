# SPEC 0001 — realm-core contracts

- **Status:** Accepted (2026-09-10)
- **Milestone:** M0
- **Decisions:** [ADR 0001](../adr/0001-ledger-as-single-source-of-truth.md),
  [ADR 0004](../adr/0004-ndjson-control-socket.md),
  [ADR 0005](../adr/0005-palette-toml-single-source.md),
  [ADR 0006](../adr/0006-oklab-contrast-not-filters.md),
  [ADR 0009](../adr/0009-no-animation-budget.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md),
  [ADR 0012](../adr/0012-font-fallback-is-a-contract.md)

> **Written after the fact.** `realm-core` was implemented before the spec-first
> rule (S14) was adopted. This spec is retro-fitted from the code and its tests
> rather than the other way round, and is recorded as such so the trail is
> honest. The original acceptance criteria name tests that existed and passed.
> A12a is the later Accepted M2 repeat-policy amendment and follows the current
> spec-first order: its named test must be observed failing before implementation
> in the same change. From SPEC 0002 onward the order is spec first.

## Purpose

`realm-core` is the vocabulary every realm component agrees on. Without it, the
compositor, the bar, the launcher and the CLI each invent their own notion of
"which window is where" and drift apart — the failure that makes most
assembled-from-parts desktops feel like a pile of programs rather than one thing.

It is deliberately free of I/O, timers and toolkit dependencies, so the
interesting logic is testable without a Wayland socket. The historical M0
`ipc::socket_path()` helper is an acknowledged exception slated for removal by
Accepted SPEC 0007; it is not a supported `realm-core` contract.

## Scope

**In:** the ledger and its undo history; the layout projection; colour maths and
the palette file format; the keymap model; the control-protocol wire types; the
bar state snapshot; the glyph inventory and fallback contract.

**Out:** anything that talks to something. Socket transport, runtime-directory
resolution, filesystem validation, and peer credentials belong to the Linux
session/client transport defined by SPEC 0007; rendering belongs to `realm-bar`,
template expansion to `realm-theme`, and process spawning to `realm-ctl`.

## Behaviour

### Ledger

Six orbits, each an ordered `Vec<WinId>` with an optional focus index, a stow
list, a layout and an optional fullscreen window. Every desired mutation
checkpoints the whole state first, bounded to `HISTORY_DEPTH` (64) snapshots,
so undo restores an earlier desired ledger exactly rather than replaying
inverse operations.

Observed window lifecycle is not undoable. A successful `summon` or `banish`
changes the current window set without creating a checkpoint and clears both
undo and redo history. This is a deliberate MVP history boundary: retaining an
older snapshot across an observed open could make Undo remove a live window,
while retaining one across an observed close could resurrect a nonexistent
`WinId`. The current ledger, every historical snapshot, session metadata, and
backend identity map must never disagree about which windows exist.

`summon` inserts **after the focused window**, not at the end: opening a terminal
beside the current one is what muscle memory expects. Mutations that change
nothing (switching to the orbit already shown, setting the layout already set,
or stepping focus in a singleton orbit) do not consume undo depth.

### Layout projection

`project(&Orbit, Workarea, TriptychParams) -> Vec<Placement>` is pure. The same
inputs always produce byte-identical output, so the compositor can skip a
relayout entirely when nothing changed.

Rectangles must **tile the workarea exactly**: no gaps, no overlaps, every pixel
accounted for. Integer splits use largest-remainder (`partition`), never
division, because division loses up to `n - 1` pixels and those losses show up
as hairline cracks of void between tiles.

### Colour and palette

Contrast is a *derivation*, not a filter: foreground lightness is pushed away
from the background in OKLab and capped at the sRGB gamut boundary, so accents
keep both hue and chroma instead of drifting toward blue and washing out.

`palette.toml` is parsed strictly. Out-of-range contrast, a non-zero border
radius, a wrong rune count and an unknown pantheon accent are all rejected at
parse time. `lint()` reports every readability and hue-separation finding at
once rather than the first.

### Keymap, IPC, state, glyphs

One modifier, bindings resolved per mode, conflicts detectable. Each `Binding`
has an explicit `repeatable: bool`; repeatability is policy data and is never
inferred from a backend event. In the MVP default only directional Focus and
Swap bindings repeat. Every toggle, close, launch, mode, orbit, layout, undo,
theme, grimoire, and Quit binding is non-repeatable. Wire types are
adjacently-tagged serde enums framed one-per-line as JSON. `RealmState` is a
plain snapshot with `renders_same_as`, which is how the bar avoids redrawing
when a module recomputes to the same string. Every glyph realm draws is in an
inventory with a documented ASCII fallback.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Summoning a window while another is focused places it immediately after the focused one | `ledger::tests::summon_inserts_after_focus_not_at_the_end` |
| A2 | Swapping carries focus with the moved window | `ledger::tests::swap_carries_focus_with_the_window` |
| A3 | Undo restores the exact previous desired ledger, and history stays bounded | `ledger::tests::undo_restores_the_exact_previous_ledger`, `::undo_history_is_bounded` |
| A3a | An observed summon or banish clears undo and redo without allowing Undo to remove a live window or restore a closed one | `ledger::tests::window_lifecycle_is_a_hard_undo_boundary` |
| A3b | Stepping focus in a singleton orbit is a no-op and consumes no undo depth | `ledger::tests::singleton_focus_step_does_not_checkpoint` |
| A4 | Stowing removes a window from the projection but not from the ledger | `ledger::tests::stow_removes_from_projection_but_not_from_the_ledger` |
| A5 | The triptych reproduces the reference desktop's geometry at 1920×1080 | `layout::tests::triptych_matches_the_reference_desktop` |
| A6 | Every layout tiles the workarea exactly at every plausible size and window count | `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` |
| A7 | Moving focus changes no rectangle | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` |
| A8 | Integer partition sums exactly to the total and stays balanced | `layout::tests::partition_is_exact_and_stable` |
| A9 | Raising contrast increases separation without desaturating or rotating accents | `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating`, `::contrast_preserves_accent_hue` |
| A10 | The shipped palette parses, round-trips and passes its own lint across the whole contrast range | `palette::tests::shipped_palette_survives_the_whole_contrast_range` |
| A11 | An invalid palette is rejected at parse time, not at render time | `palette::tests::out_of_range_values_are_rejected_at_parse_time` |
| A12 | The which-key strip matches the reference row exactly | `keys::tests::strip_matches_the_reference_which_key_row` |
| A12a | The default keymap marks only directional Focus and Swap bindings repeatable; all other actions are non-repeatable | `keys::tests::only_directional_focus_and_swap_bindings_repeat` |
| A13 | Every protocol message survives a round trip through a single-line frame | `ipc::tests::requests_round_trip_through_a_frame` |
| A14 | An ASCII-only font degrades to documented substitutes instead of tofu | `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu` |
| A15 | A revision bump alone does not force a bar redraw | `state::tests::revision_alone_does_not_force_a_redraw` |

## Budgets

No I/O, so no frame budget of its own. The projection is the hot path behind the
"key press → new geometry < 4 ms" budget in
[ARCHITECTURE.md §4](../ARCHITECTURE.md); it is pure integer arithmetic over a
list that is realistically under twenty elements.

## Failure modes

Responsible for not causing, from [PITFALLS.md](../PITFALLS.md): rounding loss
between tiles, off-by-one at odd resolutions, focus causing relayout, redraw on
a timer, tofu glyphs, contrast-as-a-filter, an unreadable palette, and protocol
version skew.

## Implementation-status note

The acceptance table covers the implemented M0 ledger, layout, palette, wire,
state, and glyph behaviour. It does not claim that the legacy impure
`ipc::socket_path()` helper is accepted or passing as an M2 transport contract;
SPEC 0007 supersedes that helper before session transport work begins.

## Open questions

- `TriptychParams` currently has no per-output override. Users on mixed-DPI
  multi-monitor setups may want one; deferred to M6 rather than guessed at now.
- `Layout::Even` exists as a fallback for ledgers that outgrow the triptych's
  shape. Whether users actually want it, or would rather the triptych degrade
  differently, is unanswered until someone has used realm for a week.
