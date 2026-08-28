# ADR 0006 — Contrast is derived in OKLab, not applied as a filter

- **Status:** Accepted (ratified 2026-08-28); see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

The handoff asks for an adjustable contrast setting and records how the
prototype implemented it: `backdrop-filter: contrast(0.85–1.4)`, default 1.08.
It also says, in the same breath, "In production: derive palette variants from
`palette.toml` instead of a filter." This ADR is why.

A `contrast()` filter is a per-pixel operation over the whole composited output.
It has two costs.

**Performance.** It is a fullscreen pass, every frame, forever. helm's stated
budgets are a bar redraw under 8 ms and idle CPU at roughly zero
(`docs/ARCHITECTURE.md` §4), and M6's acceptance criterion is holding those on a
2015-era laptop. A fullscreen shader pass on integrated graphics from that era
is not compatible with idle CPU at zero, because there is no such thing as an
idle frame once a filter is in the pipeline. It also forces a GPU context into
processes that otherwise need none (ADR 0008).

**Colour.** The CSS `contrast()` filter operates per channel in gamma-encoded
sRGB: each of R, G and B is pushed away from 0.5 independently, then clamped.
For a neutral grey that is harmless. For a saturated accent it is not: the
channels have different headroom, so they clip at different points and the
ratios between them change. The ratios between channels are the hue.

Measured on the prototype: helm's violet `#a692ec` at contrast 1.4 drifted
roughly 18 degrees toward blue under naive per-channel clamping. *(Recorded as
an observation from the prototype, not as an invariant; the test suite asserts
the direction of the effect rather than the magnitude.)* The palette's own rule
is that saturated accents stay more than 25 degrees apart
(`MIN_ACCENT_HUE_SEPARATION`). An 18 degree drift is most of that budget spent
by a setting whose entire purpose is legibility. High contrast would not be the
same theme read more easily; it would be a different theme.

## Decision

Contrast is a derivation applied once, at theme-apply time, in OKLab.

1. `color::apply_contrast(fg, bg, factor)` converts both colours to OKLab, moves
   the foreground's lightness away from the background's by `factor`
   (`b.l + (f.l - b.l) * factor`), and holds `a` and `b` — that is, hue and
   chroma — fixed.
2. When the requested lightness would leave the sRGB gamut,
   `reachable_lightness` binary-searches for the lightest (or darkest) value
   that keeps the colour's hue *and* chroma representable, and stops there. The
   accent gets as much extra contrast as sRGB can actually give it and stays
   recognisably itself.
3. Anything still out of gamut goes through `Oklab::gamut_mapped`, which reduces
   chroma along a fixed hue and lightness rather than clamping channels.
4. `Palette::derived()` folds the factor into every text and accent value and
   sets `contrast = 1.0` on the result, so the operation is idempotent and
   templates render a plain palette with no knowledge of contrast at all.
5. `contrast` is validated at parse time to `[0.85, 1.40]`, matching the
   prototype's range.
6. There is no compositor-side filter, and no shader in any helm process.

The key insight, and the reason for the gamut cap: saturated colours run out of
gamut before they run out of lightness. There is no very light, very saturated
violet in sRGB. The naive response is to desaturate towards it, which turns the
accent into a near-grey whose hue angle is numerical noise. helm stops pushing
instead.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **`backdrop-filter: contrast()` at the compositor** | What the prototype does, so it is known to look right at 1.08; one line; applies uniformly to every application including ones we cannot theme | A fullscreen pass per frame against a budget that assumes idle frames cost nothing, and per-channel clamping rotates accent hues by roughly the amount the palette reserves to keep them apart. It also affects application content we have no business recolouring |
| **Per-channel contrast in sRGB, but computed ahead of time** | Removes the per-frame cost; keeps the arithmetic trivial; matches the prototype's output exactly at every setting | Keeps the hue rotation, which is the more serious of the two problems. Matching the prototype exactly is not a goal when the prototype's colour maths is the thing being replaced |
| **HSL lightness adjustment** | Familiar, cheap, one function, preserves the H channel by definition | HSL's L is not perceptual. Equal L steps are wildly unequal in perceived lightness across hues, so gold and violet at the same nominal contrast would look nothing alike. It also has no gamut concept, so it clips exactly where it matters |
| **Ship a fixed set of hand-tuned palettes** (low / normal / high) | Perfect control; a designer picks every value; no maths to get wrong | Three times the values to maintain, and the handoff asks for a continuous setting. It also does not compose with a user's own palette, which ADR 0005 says must work |
| **CIELAB instead of OKLab** | Older, more widely implemented, well understood | OKLab's hue lines are markedly straighter, particularly in blue and violet, which is precisely helm's accent region. CIELAB's blue hue shift under lightness change would reintroduce a smaller version of the problem we are solving |

## Consequences

### Good

- Zero runtime cost. The derivation happens once, inside the 150 ms theme-apply
  budget, and the compositor draws plain colours.
- Accent hues survive the whole contrast range. `contrast_preserves_accent_hue`
  asserts under 4 degrees of drift at 0.85, 1.20 and 1.40 for all six accents.
- Accents keep their chroma at the top of the range rather than washing out.
- No GPU context is needed anywhere, which is what makes ADR 0008's CPU
  rasteriser viable and keeps cold start under 900 ms.
- The derived palette is a normal palette, so `Palette::lint` checks the values
  the user will actually see, at their actual contrast setting.

### Bad

- Contrast only reaches what helm themes. An unthemed application does not
  respond to the setting at all, whereas a compositor filter would have. This is
  a genuine loss of reach and it is the strongest argument for the filter.
- Changing contrast requires a theme apply, not a live slider. At under 150 ms
  this is acceptable, but it is not the same interaction.
- The gamut cap means contrast 1.40 does not raise every colour by the same
  amount. Accents near the boundary move less than greys do. This is correct but
  it is not obvious, and users may read it as the setting not working.
- OKLab conversion is `cbrt` and a matrix multiply per colour, plus up to 24
  binary-search iterations for out-of-gamut values. Irrelevant for forty colours
  once; it would not be for a per-pixel operation.

### Neutral

- Backgrounds are not contrast-adjusted, only foregrounds. Pushing surfaces
  apart as well would break the "surfaces differ too strongly; seams will read as
  gaps" lint.

## Reversal

Low. The derivation is confined to `helm-core::color` and
`Palette::derived()`. Reinstating a filter would mean adding a compositor
pass in `helm-compositor` and setting `contrast = 1.0` everywhere else: the
palette pipeline would not need to change at all.

The signal to reconsider is accessibility feedback that users need contrast
applied to *unthemed application content*, which the derivation cannot reach.
The right response to that signal is probably a separate, explicitly opt-in
accessibility filter rather than making the default path a filter again.

## Guard

- `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating`
  — the named guard for this decision. Asserts that at contrast 1.40 every
  accent retains more than 90% of its chroma and never loses contrast.
- `color::tests::contrast_preserves_accent_hue` — under 4 degrees of drift for
  all six accents across the range.
- `color::tests::gamut_mapping_holds_hue_where_clamping_would_not` — asserts
  directly that chroma reduction beats channel clamping on hue fidelity. This is
  the test that encodes the 18 degree observation as a permanent invariant.
- `color::tests::oklab_round_trips_within_one_step`.
- `color::tests::identity_contrast_is_a_no_op`.
- `palette::tests::shipped_palette_survives_the_whole_contrast_range`.
