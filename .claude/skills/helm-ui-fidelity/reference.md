# helm-ui-fidelity reference

Token tables and per-surface anatomy. Every value here is a **field in
`palette.toml`**, parsed into `helm_core::Palette` — read it from there rather
than retyping it. The column headed "Field" is the path to use.

The decision procedure and the checklist live in `SKILL.md`.

## Contents

- [Metrics](#metrics)
- [Type scale](#type-scale)
- [Colour roles](#colour-roles)
- [Borders and seams](#borders-and-seams)
- [Glyphs by surface](#glyphs-by-surface)
- [Surface anatomy](#surface-anatomy)

## Metrics

| What | Field | Value |
|---|---|---|
| Top bar height | `metrics.bar_height` | 32 |
| Window header height | `metrics.header_height` | 26 |
| Which-key strip height | `metrics.whichkey_height` | 26 |
| Border width | `metrics.border_width` | 1 |
| Corner radius | `metrics.radius` | 0 — rejected at parse time if changed |
| Launcher panel width | `metrics.launcher_width` | 640 |
| Portal dialog width | `metrics.portal_width` | 720 |

`Workarea::new(w, h, top_reserved, bottom_reserved)` is given
`bar_height` and `whichkey_height`; a surface that draws taller than it reserved
overlaps a tile.

## Type scale

| Role | Field | Value |
|---|---|---|
| Family | `typography.family` | IBM Plex Mono |
| Fallback chain, in order | `typography.fallback` | IBM Plex Mono, Symbols Nerd Font Mono, Noto Sans Symbols 2, Noto Sans Egyptian Hieroglyphs, Symbola, DejaVu Sans Mono |
| Regular / medium / bold | `typography.weight_regular` / `_medium` / `_bold` | 400 / 500 / 600 |
| Body | `typography.size_body` | 12.5 |
| Meta | `typography.size_meta` | 11.0 |
| Micro (epithets, stat blocks) | `typography.size_micro` | 10.5 |
| Page title | `typography.size_title` | 20.0 |
| Line height | `typography.line_height` | 1.62 |

The fallback chain is ordered and explicit on purpose: leaving it to the system
default lets an emoji font hijack the symbol range and render runes in colour at
the wrong size (`docs/PITFALLS.md`).

## Colour roles

| Role | Field | Value |
|---|---|---|
| Root / void | `background.void` | `#05060c` |
| Unfocused window body | `background.pane` | `#0a0c14` |
| Focused window body | `background.focused` | `#0b0d16` |
| Inset blocks, code previews, result rows | `background.raised` | `#0e111b` |
| Bar gradient | `background.bar_top` → `background.bar_bot` | `#0a0c15` → `#080911` |
| Active orbit, clock, primary values | `text.bright` | `#f2f5fb` |
| Body copy | `text.normal` | `#c2cbde` |
| Which-key labels, occupied orbits | `text.mid` | `#93a1ba` |
| Secondary body | `text.soft` | `#aab6cc` |
| Meta | `text.dim` | `#66748e` |
| Micro, epithets, empty orbits, chord echo | `text.faint` | `#4a5670` |
| Arcane — focus, odin, hecate, charon | `accent.violet` | `#a692ec` |
| Info — urania, cpu, mode badge | `accent.starlight` | `#a3bff2` |
| Warn — horus, battery, the `:` prompt | `accent.gold` | `#d9b06a` |
| Ok — thoth, marks | `accent.teal` | `#7fd4c1` |
| hermes | `accent.cyan` | `#7fc9e8` |

Per-pane accents come from `Palette::pantheon_color(tool)`, not from picking an
accent by hand: `odin` violet, `thoth` teal, `hermes` cyan, `horus` gold,
`urania` starlight, `hecate` violet, `charon` violet.

## Borders and seams

Alpha is stored beside its colour so formats without an alpha channel can
flatten with `Rgb::flatten_over` (or the template's `.over(...)` placeholder).

| Role | Fields | Value |
|---|---|---|
| Focused window border | `border.focused` + `border.focused_alpha` | violet at 0.60, plus a **static** inset glow |
| Seam between tiles | `border.seam` + `border.seam_alpha` | violet at 0.30 |
| Unfocused border | `border.neutral` + `border.neutral_alpha` | `#66748e` at 0.30 |
| Bar bottom border | `border.bar_bottom` + `border.bar_bottom_alpha` | violet at 0.40 |
| Window-header underline | `border.pantheon_alpha` | the pane's pantheon colour at 0.30 |

## Glyphs by surface

`helm_core::glyphs::inventory()` is the complete list; `Surface` tags where each
appears. Every entry has an ASCII fallback and is drawn through
`Probe::resolve`.

| Surface | Glyphs | Notes |
|---|---|---|
| `Bar` | `✦` logo, `⌗` layout, `⌨` mode badge, `▸` chord echo, `☾` clock, `⚡` battery, `♪` volume, and the six runes `ᚠᚢᚦᚨᚱᚲ` | runes fall back to the digits `1`–`6`, loudly |
| `WhichKey` | `⊞` modifier, `↵` return | |
| `Header` | `◈ ─ ✕` controls, and the pantheon marks `ᛟ` odin, `☽` thoth, `⚚` hermes, `◉` horus, `✶` urania | |
| `Instruments` | `▰` / `▱` meters, the eight-step braille ramp `⡀⣀⣄⣤⣦⣶⣷⣿` | ramp degrades to ` . . : : \| \| #` |
| `Overlay` | `⚷` hecate, `☍` charon, `⇥` tab | |
| `Prompt` | `𓂃` | falls back to `~`; the handoff singles this one out |

## Surface anatomy

Read the canonical prototype for the exact arrangement; this is the checklist of
parts so nothing is missed.

### Top bar (32px) — `Desktop v3.dc.html`

Left to right: `✦ helm` logo in violet with a 1px right border; six orbit runes
at 28px each (active — bright text, 2px violet bottom border, violet wash at
0.14, text glow; occupied — `text.mid`; empty — `text.faint`); layout indicator
`⌗ triptych`; mode badge `⌨ NAV` at meta size on a starlight wash at 0.14;
centred focused-window title and chord echo `mod+ ▸ awaiting chord…` in
`text.faint`; then right-hand modules 16px apart — net, cpu, mem, gpu, vol,
battery, datetime. Background is a vertical gradient with four static 1px
starfield dots and a violet bottom border at 0.40.

`Mode::badge()` yields the word only — `NAV`, `RESIZE`, `MOVE` — and the bar
prepends the `⌨` glyph through `Probe::resolve`.

The bar's data is `HelmState`: `orbits` (six `OrbitCell`s with `OrbitDisplay`),
`layout`, `mode`, `focused_title`, `chord_echo`, `whichkey`, and `modules`
(each `Module` carries `id`, `text`, an optional palette `accent` name, and
`urgent`).

### Which-key strip (26px)

`⊞ mod` prefix in violet with a right border, then key hints 18px apart, key in
`text.bright` and label in `text.mid`; `? grimoire — full spellbook` right
aligned with the `?` in gold. The content is `Keymap::strip()` in order, and
`keys::tests::strip_matches_the_reference_which_key_row` asserts it against the
mockup: `↵ thoth · d hecate · b hermes · j/k focus · h/l swap · 1-6 orbit ·
s stow · m mono · f full · r resize · q banish`.

### Window header (26px)

Pantheon glyph, god name, role epithet at micro size in `text.faint`, and
right-aligned `◈ ─ ✕` controls with 2px letter spacing. The bottom border is the
pane's pantheon colour at `border.pantheon_alpha`.

### Hecate launcher (`metrics.launcher_width`, 640)

Scrim over the desktop; panel bordered violet with a glow; header `⚷ HECATE`,
the query and an 8×18px block cursor in violet; result rows padded 8px by 18px,
the selected row on a violet wash at 0.14 with a 2px left border; result types
tagged `run` / `cmd` / `spell`; footer `↵ invoke · ⇥ cycle · esc dismiss`.
The MVP ships fuzzel themed as hecate; the native client is **M4**.

### charon (`Files & Theming.dc.html` §`1a`) and the portal (§`1b`)

charon: 1400×900 reference, 26px header, columns 240px parents / flex current /
460px preview, marked rows teal `✓`, flagged gold `⚑`, cursor row on a violet
wash at 0.14 with a 2px left border, `:` shell line with a gold prompt, then the
which-key row. **No thumbnails, ever** — previews are text and stat only.
Delivered as a yazi theme and keymap, not a new program (ADR 0007).

Portal: `metrics.portal_width` (720) modal over a dimmed requesting app, a 180px
left rail of HARBORS and RECENT, fuzzy-matched substrings highlighted violet.
The MVP uses the toolkit's own themed dialog; charon portal is **M4**.
