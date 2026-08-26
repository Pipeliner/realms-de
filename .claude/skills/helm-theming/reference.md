# helm-theming reference

Open this when theming a new app, or when you need an exact signature from
`crates/helm-core/src/color.rs` or `palette.rs`. The rules and the decision
procedure live in `SKILL.md`.

## Contents

- [What the theme reaches, and what it cannot](#what-the-theme-reaches-and-what-it-cannot)
- [Palette schema](#palette-schema)
- [Colour API](#colour-api)
- [Lint rules](#lint-rules)
- [Tests](#tests)

## What the theme reaches, and what it cannot

From `design/HANDOFF.md` §1c and the annotated panel in
`design/Files & Theming.dc.html` (section `1c`). This is a realistic reach, not
an aspiration: quote the limits to the user rather than promising a fix.

### Reaches

| Surface | Mechanism | Notes |
|---|---|---|
| GTK 3 and GTK 4 | one generated `gtk.css` | colours, borders, square corners, flat headerbars, font forced to IBM Plex Mono |
| Qt 5 and Qt 6 | `qt6ct` colour scheme plus a Kvantum SVG theme, both fed from `palette.toml` | MVP ships the qt6ct colours only; full Kvantum is deferred to M6 (`docs/MVP.md`) |
| libadwaita | named colours via `~/.config/gtk-4.0/gtk.css` | accent, window and view backgrounds |
| Terminals and TUIs | 16-colour ANSI scheme | the native citizens — foot, yazi, btop, starship, helix all land here |
| Electron / Chromium | force dark plus an injected palette where supported | best-effort by construction |
| Cursor and icons | a single mono-line icon set, no full-colour mimetypes | cursor theme and size must also be set in the session environment — see the `wayland-session-integration` skill |

### Limits — do not fight these

| Limit | Why | The honest answer |
|---|---|---|
| Apps with hardcoded colours or bundled themes | nothing to override | document it; it keeps its own colours |
| libadwaita widget *geometry* (pills, large paddings) | shapes are not exposed as CSS the way colours are | colours yes, geometry mostly no |
| CSD headerbars | client-drawn by definition | helm recolours them and adds its 1px border; the shape stays theirs |
| Flatpak apps | sandboxed away from `~/.config` | needs a per-app grant, `--filesystem=xdg-config/gtk-4.0`. Listed as a packaging pitfall in `docs/PITFALLS.md` |

A limit discovered in the wild is a new row in `docs/PITFALLS.md`, not a
workaround buried in a template.

## Palette schema

`palette.toml`, parsed by `Palette::from_toml` into
`crates/helm-core/src/palette.rs`.

| Section | Holds | Range checks at parse time |
|---|---|---|
| top level | `name`, `variant`, `contrast` | `contrast` must be within `[0.85, 1.40]` |
| `[background]` | `void`, `pane`, `focused`, `raised`, `bar_top`, `bar_bot` | `#rrggbb` only |
| `[text]` | `bright`, `normal`, `mid`, `soft`, `dim`, `faint` | |
| `[accent]` | `violet`, `starlight`, `gold`, `teal`, `cyan` | `Accents::by_name` and `named()` enumerate exactly these five |
| `[border]` | colour plus a separate `_alpha` for each of `focused`, `seam`, `neutral`, `bar_bottom`, and `pantheon_alpha` | alpha is carried separately so alpha-less formats can flatten |
| `[pantheon]` | tool → accent *name* (`odin`, `thoth`, `hermes`, `horus`, `urania`, `hecate`, `charon`) | an unknown accent name is a parse error |
| `[typography]` | `family`, ordered `fallback` list, three weights, four sizes, `line_height` | `fallback` must not be empty |
| `[metrics]` | `bar_height` 32, `header_height` 26, `whichkey_height` 26, `border_width` 1, `radius`, `launcher_width` 640, `portal_width` 720 | `radius` must be `0` |
| `[glyphs]` | `runes`, `controls`, meter and sparkline characters, `prompt_sigil` and `prompt_sigil_fallback` | `runes` must contain exactly `ORBIT_COUNT` (6) entries |

Helpers: `Palette::load(path)`, `to_toml()`, `derived()`,
`pantheon_color(tool)` (unknown tool falls back to violet), `lint()`,
`is_applicable()` (true when no finding is fatal).

## Colour API

`crates/helm-core/src/color.rs`.

| Item | Use |
|---|---|
| `Rgb::parse(&str)` | accepts `#rrggbb` or `rrggbb`; anything else is `Error::BadColor` |
| `Rgb::hex()` / `hex_bare()` | `#a692ec` / `a692ec` |
| `Rgb::css_rgba(alpha)` | `rgba(166, 146, 236, 0.3)` |
| `Rgb::argb(alpha)` | `0xAARRGGBB`, the layout `tiny-skia` and Kvantum want |
| `Rgb::flatten_over(bg, alpha)` | composite for formats with no alpha channel |
| `Rgb::to_oklab()` / `Oklab::to_rgb()` | round-trips within one 8-bit step |
| `Rgb::luminance()` / `contrast_ratio(other)` | WCAG 2.1, ratio in `[1.0, 21.0]` |
| `Rgb::hue_degrees()` / `chroma()` | hue is noise below ~0.02 chroma — gate assertions on chroma |
| `Oklab::in_gamut()` | representable in sRGB? |
| `Oklab::to_rgb()` | clamps per channel — **rotates hue**, use only when hue does not matter |
| `Oklab::gamut_mapped()` | reduces chroma, holds hue and lightness — the safe conversion |
| `apply_contrast(fg, bg, factor)` | the one public entry point for contrast |
| private `reachable_lightness(base, target)` | the gamut-boundary cap; why accents stay accents at 1.40 |

## Lint rules

`Palette::lint()` returns `Vec<Finding { path, message, fatal }>`, linting the
**derived** palette against `background.pane`.

- `MIN_BODY_CONTRAST = 4.5` — `text.bright`, `text.normal`, `text.soft`.
  Failing this floor is **fatal**: the palette is refused, not warned about.
- `MIN_META_CONTRAST = 2.6` — `text.mid`, `text.dim`, `text.faint`, and every
  accent against `background.pane`. Non-fatal; these are meta surfaces.
- `MIN_ACCENT_HUE_SEPARATION = 25.0` — degrees between any two accents, so a
  new near-duplicate accent is reported rather than quietly ambiguous. Pairs
  below 0.02 chroma are skipped, because hue is numerical noise there.
- `background.void` versus `background.pane` above 1.6:1 — the surfaces differ
  so strongly that 1px seams start reading as gaps.

`Finding` renders as `error path: message` or `warn  path: message`, and
`is_applicable()` is simply "no fatal finding".

## Tests

Run with `cargo test -p helm-core <name>`.

| Test | Holds |
|---|---|
| `palette::tests::shipped_palette_parses_and_round_trips` | the shipped file parses and survives `to_toml` → `from_toml` |
| `palette::tests::shipped_palette_passes_its_own_lint` | no fatal findings at the shipped contrast |
| `palette::tests::shipped_palette_survives_the_whole_contrast_range` | no fatal findings at eleven steps from 0.85 to 1.40 |
| `palette::tests::pantheon_maps_every_tool_to_a_real_accent` | every pantheon entry resolves; unknown tools fall back to violet |
| `palette::tests::out_of_range_values_are_rejected_at_parse_time` | contrast 2.5, radius 6 and an invented accent name all fail to parse |
| `palette::tests::lint_catches_muddy_text_and_duplicate_accents` | muddy body text is fatal; a duplicated accent is reported |
| `palette::tests::derived_palette_is_idempotent` | `derived()` sets contrast to 1.0 and applying it twice changes nothing |
| `color::tests::contrast_preserves_accent_hue` | hue survives the whole contrast range |
| `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating` | accents stay saturated instead of washing out |
| `color::tests::gamut_mapping_holds_hue_where_clamping_would_not` | the difference between `gamut_mapped()` and `to_rgb()`, asserted |
| `color::tests::identity_contrast_is_a_no_op` | `factor == 1.0` returns the input verbatim |
| `color::tests::flatten_matches_manual_composite` | `flatten_over` agrees with hand arithmetic |
| `color::tests::known_contrast_ratios` | WCAG ratios against known pairs |
