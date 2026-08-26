---
name: helm-theming
description: Use when anything about helm's colour or generated theme files is in play - editing palette.toml, changing a colour or contrast value, adding or editing a template in configs/templates/, theming a new app (gtk.css, Kvantum, qt6ct, ANSI/foot, yazi, btop, starship, fuzzel, helix), touching crates/helm-core/src/color.rs or palette.rs or the helm-theme crate, working on `helm ctl theme apply` or `lint`, or answering "can we make this bit blue" and "why does the accent look washed out at high contrast".
---

# helm theming

One palette file, many generated files, applied atomically. Two rules carry the
whole design and both are easy to break by accident.

**Start in the spec** (S14). `docs/specs/0002-theme-pipeline.md` is the M1
contract for `helm-theme` and is *Accepted with an empty Test column* — the
happy-path tests for A1–A10 are the next thing to be written, watched to fail,
and implemented against. The palette format itself is SPEC 0001; the decisions
are ADR 0005 (one palette file) and ADR 0006 (derive, do not filter).

## Rule 1 — no colour is written outside `palette.toml`

`palette.toml` at the repo root is the single source of truth (ADR 0005). A
hex literal, an `rgba(...)`, a named CSS colour or a 256-colour index anywhere
else — a template's fallback, a Rust constant, a shipped config, a doc example
that gets copied — is a second source that will drift from the first. The
symptom is one app being subtly the wrong shade after a palette edit, and it is
tedious to find because nothing fails.

If a value is missing, **add it to `palette.toml`**, do not inline it. The
shipped palette is `include_str!`-ed into `helm-core`'s tests, so a bad edit
fails `cargo test`, not the user's desktop.

Templates receive the **derived** palette, so no template applies contrast
itself and no template does colour maths. If a template needs a flattened or
alpha'd variant, use the placeholder forms in `docs/INTERFACES.md` §2 —
`{{ accent.violet.rgba(0.3) }}`, `{{ accent.violet.over(background.pane, 0.3) }}`
— rather than pre-computing a hex.

## Rule 2 — contrast is a derivation, never a filter

The `.dc.html` prototype implemented the user's contrast setting as
`backdrop-filter: contrast(0.85–1.4)`. That is prototype scaffolding, not the
design. Reintroducing a filter is forbidden for two independent reasons: it
costs a full-screen GPU pass every frame (blowing the budgets in
`docs/ARCHITECTURE.md` §4), and it rotates hues — helm's violet drifted about
18° toward blue at contrast 1.4 before the current implementation.

How it actually works, in `crates/helm-core/src/color.rs`:

1. `apply_contrast(fg, bg, factor)` converts both to OKLab and moves `fg`'s
   **lightness** away from `bg`'s by `factor`. Chroma is untouched, so hue
   survives. `factor == 1.0` returns `fg` verbatim.
2. Saturated colours run out of gamut before they run out of lightness — there
   is no very light, very saturated violet in sRGB. `reachable_lightness()`
   binary-searches the furthest lightness that keeps hue *and* chroma inside
   sRGB, and stops there.
3. Anything still outside the gamut goes through `Oklab::gamut_mapped()`, which
   reduces chroma while holding hue and lightness. Never `to_rgb()` (plain
   per-channel clamping) on a colour whose hue matters.
4. `Palette::derived()` folds `contrast` into every text and accent value once,
   at theme-apply time, and sets `contrast = 1.0` on the result — so it is
   idempotent and there is nothing left to do at runtime.

Guards: `color::tests::contrast_preserves_accent_hue`,
`color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating`,
`color::tests::gamut_mapping_holds_hue_where_clamping_would_not`,
`palette::tests::derived_palette_is_idempotent`.

Related trap from `.claude/memory/30-gotchas.md`: hue angle is meaningless below
about 0.02 chroma, so gate any hue assertion on `Rgb::chroma()` and test greys
for lightness instead.

## Rule 3 — application is atomic

A half-applied theme (new terminal colours against an old GTK stylesheet, until
relogin) is a failure mode listed in `docs/PITFALLS.md`. The contract in
`docs/INTERFACES.md` §2 is: render every file to `<target>.helm-tmp`, `rename(2)`
each into place, and **only then** fan out reloads once. Never reload per file,
never reload before every file is in place, and never leave a partial write
behind if the process is killed mid-apply. Byte-identical files are reported as
`unchanged` and are neither rewritten nor reloaded — that is what keeps
`helm ctl theme apply` inside its 150 ms budget.

## Adding a template

`helm-theme` and `configs/templates/` do not exist yet — they land in **M1**.
Until then, adding a template means extending the design, not writing code
against an imaginary API. When M1 is under way:

1. Add a `Template` (`docs/INTERFACES.md` §2) with a stable `id`, the source
   text, a `target` relative to `$XDG_CONFIG_HOME`, and a `Reload` variant:
   `None` (read at next start), `Signal { process, signal }`, `Command(..)` for
   things like `gsettings set`, or `HelmClients` for helm's own bar and launcher.
2. Write the source with `{{ path.to.value }}` placeholders only. An unknown
   placeholder must be a hard render error — a silently blank colour is the
   exact bug this design exists to prevent.
3. For a format with no alpha channel (ANSI, Kvantum SVG fills), use
   `.over(background.pane, 0.3)` so a 30%-violet seam still looks like one.
   `Rgb::flatten_over` is the underlying maths.
4. Check the app against the reach-and-limits table before promising anything —
   see `reference.md`. Some things are simply not reachable and the honest
   answer is to document the limit rather than fight it.
5. Verify with `cargo test -p helm-core palette` plus a real before/after look
   at the generated file.

## Changing a colour

```
$EDITOR palette.toml
cargo test -p helm-core palette::tests::shipped_palette_passes_its_own_lint
cargo test -p helm-core palette::tests::shipped_palette_survives_the_whole_contrast_range
cargo test -p helm-core                     # the whole crate
```

`Palette::lint()` enforces WCAG floors (`MIN_BODY_CONTRAST` 4.5,
`MIN_META_CONTRAST` 2.6) and at least 25° of hue separation between accents
(`MIN_ACCENT_HUE_SEPARATION`). It returns *every* finding rather than the first,
so fix one round of complaints instead of ten. The contrast-range test re-lints
at eleven contrast steps from 0.85 to 1.40 — a colour that is legible at 1.08
and grey-on-grey at 0.85 fails there, which is the point.

`metrics.radius` must be `0` and `glyphs.runes` must list exactly six runes;
both are rejected at parse time, not at lint time.

## Reference

`reference.md` holds the reach-and-limits table (what helm's theme can and
cannot touch, from `design/HANDOFF.md` §1c), the full colour API with
signatures, and the palette schema section by section. Open it when theming a
new app or when you need an exact function name.
