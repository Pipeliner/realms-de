---
name: helm-ui-fidelity
description: Use when implementing or reviewing any helm surface the mockups define - the 32px top bar, orbit runes, the mode badge and chord echo, the which-key strip, window headers, the hecate launcher overlay, the charon file manager or portal dialog, or anything in crates/helm-bar. Also use when reading design/HANDOFF.md or the .dc.html prototypes, choosing a size, weight, glyph or spacing value, wondering whether something may have a rounded corner or an animation, or asking "does this match the design".
---

# helm UI fidelity

The mockups are the brief, not an inspiration (standing order S1). Colours,
spacing, type and copy are final unless an ADR overrides them with a reason.
This skill is about hitting them exactly and about the two rules people break
without noticing: motion and glyph fallback.

**Start in the spec** (S14). `helm-bar` has no spec yet — it is **M2** work, and
writing `docs/specs/` for the surface you are about to build is the first slice
of that work, not paperwork after it. The rendering contract it must satisfy is
already written: `docs/INTERFACES.md` §3, with ADR 0008 (layer-shell +
`tiny-skia` + `cosmic-text`) and ADR 0009 (no animation budget) behind it.

## Reading the prototypes

`design/*.dc.html` are hi-fi HTML prototypes. Rules for using them:

- **`Desktop v3.dc.html` is canonical.** `Desktop v2.dc.html` and
  `Desktop.dc.html` are provenance only; v1 carries a deprecated
  cyberpunk/scanline direction that must not resurface.
- **`Files & Theming.dc.html`** holds three sections, addressable by their
  element ids: `1a` charon (file manager), `1b` charon portal (open dialog),
  `1c` the GTK theming reference with the reach-and-limits notes (those belong
  to the `helm-theming` skill).
- **They are references, not production code.** All styling is inline on each
  element; there is no stylesheet to port. Read a value out of the markup, then
  implement it in the real stack. Never transliterate CSS into Rust.
- The file names contain spaces and an ampersand — quote them in shell.
- Anything a prototype expresses as a CSS trick has a production answer that is
  *not* that trick. The clearest case: the contrast slider is a
  `backdrop-filter` in the prototype and a palette derivation in production.
  See the `helm-theming` skill.

## Tokens

Do not retype a value from the handoff into code. Every size, colour, glyph and
font in `palette.toml` is already parsed into `helm_core::Palette`, so read
`metrics.bar_height`, `typography.size_body`, `accent.violet` and so on.
A literal in a client is the same bug as a colour outside the palette.

The three that catch people out:

- **No border radius anywhere.** `metrics.radius` is `0` and a non-zero value is
  rejected at parse time. The one exception in the design is the circular gauge
  rings in urania, which are shapes, not corners.
- **1px borders only.** `metrics.border_width` is `1`. Seams are drawn *inside*
  the window, which is why the layout projection must tile exactly — see the
  `helm-layout` skill.
- **The bar is 32px, headers and the which-key strip are 26px.** These are the
  same numbers `Workarea::new` reserves; a surface that draws itself taller than
  it reserved will overlap a tile.

`reference.md` has the full token tables, keyed to the `palette.toml` fields
they come from.

## No animation

Zero animations in v1 — not "few", not "fast" (ADR 0009). State changes are
instant: orbit switch, focus, launcher open and close, preview toggle. No
transitions, no blur, no shadows beyond the 1px border and the static inset glow
on the focused window. The later minimal-motion pass is **M6** and opt-in, and
even then it is bounded: opacity-only fades of at most 120 ms on overlays, never
moving or scaling a window. Do not pre-build for it; a motionless implementation
is the design, not a placeholder.

## Glyph fallback is a contract

helm draws runes, planetary symbols, braille ramps and one Egyptian hieroglyph.
On a machine without the right fonts these are tofu boxes and the desktop looks
broken before the user types anything.

- Every glyph helm draws is in `helm_core::glyphs::inventory()`, with a
  documented ASCII fallback. A glyph that is not in the inventory must not be
  drawn — add it there first.
- **Draw through `Probe::resolve(ch)`, never a raw `char`.** Bypassing the probe
  is exactly how tofu ships (`docs/INTERFACES.md` §3, rule 3).
- The probe runs once at start-up against the resolved font stack; `helm ctl
  doctor` prints `Probe::summary()`.
- Guards: `glyphs::tests::every_glyph_is_unique_and_has_a_fallback` and
  `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu`.

## Fidelity checklist — before calling a surface done

1. Side-by-side with the canonical prototype at its reference size (1920×1080
   for the desktop, 1400×900 for charon). Compare, in this order: heights,
   1px borders, spacing, type sizes and weights, then colour.
2. Every colour, size and glyph came from `Palette`, not a literal. Grep your
   diff for `#` followed by six hex digits, and for bare numeric sizes.
3. Copy matches the mockup **word for word**, including the separators. The
   which-key row is asserted against it by
   `keys::tests::strip_matches_the_reference_which_key_row`; extend that test
   rather than adjusting the strip by eye.
4. Every glyph went through `Probe::resolve`; render once with a probe that
   claims ASCII only and confirm the surface still reads.
5. Nothing animates, nothing is rounded, nothing is blurred, no shadow beyond
   the 1px border and its static inset glow.
6. Redraw happens only on a state change — see the `helm-perf-budget` skill and
   `HelmState::renders_same_as`.
7. Damage is reported, not a full-surface repaint. A clock tick damages the
   clock.

## Reference

`reference.md` holds the complete token tables (metrics, type scale, colour
roles, glyph inventory by surface) with the `palette.toml` field each value
comes from, plus the per-surface anatomy of the bar, which-key strip, window
headers, launcher and charon. Open it while implementing a surface.
