---
name: helm-layout
description: Use when working on helm's window ordering, orbits or tiling geometry - editing crates/helm-core/src/layout.rs or ledger.rs, adding or changing a Layout variant (triptych, mono, even), touching project(), partition(), TriptychParams, Placement, Rect or Workarea, changing summon/banish/swap/focus/stow/fullscreen/undo, or debugging hairline cracks, overlapping tiles, off-by-one rectangles or windows that twitch when focus moves. Also use before changing anything that could make geometry depend on focus or on the clock.
---

# helm layout and ledger

The ledger is the only state that describes window order; geometry is recomputed
from it on demand. Undo, damage tracking and free focus movement all fall out of
that one property, so the invariants below are not stylistic — break one and
three features break silently. ADR 0001 (`docs/adr/0001-ledger-as-single-source-of-truth.md`)
is the long form.

## The four invariants

1. **Positions are never stored.** `Ledger` holds `Vec<WinId>` per orbit plus
   focus index, stow set and an optional fullscreen window. No `Rect` is ever a
   field of `Ledger` or `Orbit`. If you find yourself caching a projection to
   mutate it, you have two sources of truth — adjust `TriptychParams` instead
   (that is how resize is meant to work).
2. **`project()` is pure.** `layout::project(&Orbit, Workarea, TriptychParams)`
   reads no clock, no environment and no interior mutability. Same inputs, same
   bytes out. Guarded by `projection_is_pure_and_focus_only_moves_the_flag`.
3. **The rectangles tile the workarea exactly.** Areas sum to `area.tiles.area()`,
   nothing overlaps, nothing escapes, no rectangle is degenerate. One lost pixel
   is a visible hairline crack showing the void through the seam. Guarded by
   `every_layout_tiles_exactly_for_every_plausible_size`.
4. **Focus never moves a pixel.** `focused` and `occluded` are flags on
   `Placement`. They change what is painted, never where. Same guard as (2).

Mono is the deliberate exception to (3): every window gets the full tile area and
all but the top one are marked `occluded`. That is why the exact-tiling sweep
iterates `[Layout::Triptych, Layout::Even]` and not `Layout::Mono`.

## Why `partition()` and not division

`partition(total, weights)` splits an integer by the largest-remainder method:
floor each share, then hand the leftover pixels to the largest fractional losses,
ties broken by index. Plain `w / n` loses up to `n - 1` pixels and repeated
rounding drifts, and both show up as 1px cracks on exactly the resolutions nobody
tests — which is why `1281x801` is in the sweep on purpose.

Anything that divides a length in this file goes through `partition()`. If you
write `/` on a width or height, you have almost certainly introduced a crack.

## Adding a layout

Standing order S14 applies: write the behaviour down first — a note in
`docs/specs/` for the layout's shape and acceptance criteria, an ADR if the
choice has alternatives worth recording — then the tests, then the code. Layout
maths is one of the two standing exceptions to "minimal TDD": it gets invariant
tests rather than example tests, because an example test passes happily while
the algorithm is off by a pixel at some resolution nobody tried.

1. Add the variant to `enum Layout` in `layout.rs`, then fix the three
   non-exhaustive matches the compiler points at: `label()`, `parse()`,
   `next()`. `parse()` takes a lower-case name and a short alias; `label()` is
   what the bar's `⌗` indicator prints.
2. Write the geometry as a private `fn(n_or_wins, Rect, ...) -> Vec<Rect>` that
   returns rectangles in ledger order. Reuse `grid()` if the shape is a grid;
   reuse `partition()` for every split. Return `Vec::new()` for zero windows or
   a non-positive area rather than panicking.
3. Wire it into the `match orbit.layout` inside `project()`. If it stacks rather
   than tiles, also extend the `top` / `occluded` computation and say why in a
   comment.
4. If it needs tunables, add them to `TriptychParams` (or a sibling struct) as
   **ratios, not pixels**, and clamp them in `sanitised()`. The reference desktop
   is 1920x1080; the same shape has to survive 2560x1440 and 3840x2160.
5. Add the variant to the list inside
   `every_layout_tiles_exactly_for_every_plausible_size` — unless it stacks, in
   which case write a `mono`-style test asserting occlusion instead.
6. Add a keybinding in `keys.rs` only if the design calls for one; the which-key
   strip is asserted against the mockup by
   `keys::tests::strip_matches_the_reference_which_key_row`.

## Checklist — "I added a layout"

```
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test -p helm-core layout::tests::every_layout_tiles_exactly_for_every_plausible_size
cargo test -p helm-core layout::tests::projection_is_pure_and_focus_only_moves_the_flag
cargo test -p helm-core layout::tests::partition_is_exact_and_stable
cargo test -p helm-core layout::tests::absurd_params_are_clamped_rather_than_producing_negative_rects
cargo test -p helm-core                      # the whole crate, last
```

The sweep runs your layout at 1920x1080, 2560x1440, 3840x2160, 1366x768 and
1281x801, at one to twelve windows. It is the gate; a layout that has not been
added to its list is untested no matter how many other tests pass.

## Traps

- **Serde.** `Layout` is `#[serde(rename_all = "kebab-case")]` and crosses the
  IPC boundary inside `HelmState`. A new variant is a protocol change: bump
  `ipc::PROTOCOL_VERSION` if an older client would misread it.
- **`Workarea::new(w, h, top, bottom)`** already subtracts the 32px bar and the
  26px which-key strip. Do not subtract them again inside a layout.
- **Fullscreen uses `area.output`, tiling uses `area.tiles`.** Fullscreen
  deliberately covers the bar.
- **Stowed windows are absent from `orbit.visible()`**, so they must never
  appear in a projection; `stowed_windows_are_not_projected` guards it.
- **No-op mutations must not checkpoint.** `mod+1` while already on orbit 1 may
  not burn an undo step — `ledger::tests::no_op_mutations_do_not_consume_undo_depth`.
- **The consumer of the projection is `WmBackend::apply(&[Placement])`**
  (`docs/INTERFACES.md` §1), which is called only when the projection changed
  and must be idempotent. A layout that returns a different-but-equivalent
  ordering on each call defeats both properties.

## Reference

`reference.md` in this directory lists every public item in `layout.rs` and
`ledger.rs` with its contract, and every test with the invariant it protects.
Open it when you need the exact signature or are choosing which test to extend.
