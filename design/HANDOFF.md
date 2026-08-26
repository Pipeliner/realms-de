# Handoff: helm — a minimal, feature-rich Linux DE

## Overview
"helm" is a keyboard-centric, gapless-tiling desktop environment with a space + magic aesthetic (subtle — starfield bar, rune workspaces, arcane violet focus, gold accents). Tools are named after gods and lightly styled to their pantheons. No wasted space, power-user first. **Rust-first. Snappy: zero animations in v1** (a minimal-motion pass may come later — design for instant state changes now).

## About the Design Files
The bundled `.dc.html` files are **design references created in HTML** — hi-fi prototypes showing intended look and behavior, NOT production code. Recreate them in the real stack below. Open them in a browser to inspect; all styling is inline on each element.

## Fidelity
**High-fidelity.** Colors, spacing, typography, and copy are final unless noted. Recreate pixel-close where the toolkit allows (TUI/eww approximations are expected to quantize to the cell grid).

## Target Stack (Rust-first)
Phase 1 — assemble (daily-drivable in weeks):
- **Compositor**: `river` (Zig, but mature) or `niri` (Rust) as interim. Long-term custom compositor on **Smithay** (Rust) — the mockups' "ledger" model (strict window ordering per orbit/workspace; layouts are pure projections of the ledger onto the workarea, never mutations) maps directly onto a scene graph. Undo = restore an earlier ledger.
- **Bar / which-key / instruments**: custom Rust layer-shell clients (`smithay-client-toolkit` + `tiny-skia` or `iced` layer-shell) — or `eww` as a stopgap. No animation; render on state change only.
- **Launcher (hecate)**: custom Rust layer-shell fuzzy launcher (`nucleo` matcher — same engine as helix), or `fuzzel` themed as stopgap.
- **File manager (charon)**: **yazi** (Rust) — miller columns, toggleable preview, marks, `:` shell already exist. Theme + keymap ≈ 90% of the design.
- **Open dialog (charon portal)**: `xdg-desktop-portal-termfilechooser` pointed at yazi in a centered floating terminal.
- **Monitor (horus)**: `btop` custom theme (stopgap) or Rust TUI via `ratatui` + `sysinfo`.
- **Shell (thoth)**: zsh + starship, themed prompt `nav@caldera :: 𓂃%`.
- **Agent harness (odin)**: custom `ratatui` TUI wrapping your agent runner. Nothing off-the-shelf matches.
- **Theming pipeline**: `helm-theme` crate: reads `palette.toml`, renders templates (gtk.css, Kvantum SVG, ANSI scheme, yazi/btop/rofi/helix themes), `helm ctl theme apply` hot-reloads (gsettings + SIGUSR1s).

Performance rules: no compositor-side animations, no blur, no rounded corners, no shadows except the 1px borders + inset glow on focus (a static box-shadow, cheap). Damage-tracked rendering; bar updates event-driven (not polled) where possible.

## Screens / Views

### 1. Desktop (Desktop v3.dc.html — canonical)
1920×1080 reference. Structure top→bottom:
- **Top bar, 32px**, bg vertical gradient `#0a0c15→#080911` with 4 faint 1px starfield dots (static radial gradients), bottom border `rgba(166,146,236,.4)`. Segments left→right: logo `✦ helm` (violet #a692ec, 1px right border); 6 workspace runes `ᚠᚢᚦᚨᚱᚲ` each 28px wide — active: text #f2f5fb, 2px bottom border #a692ec, bg rgba(166,146,236,.14), text-shadow glow; occupied: #93a1ba; empty: #4a5670; layout indicator `⌗ triptych` + mode badge `⌨ NAV` (bg rgba(163,191,242,.14), text #a3bff2, 11px, 1px 8px padding); centered focused-window title + chord echo `mod+ ▸ awaiting chord…` (#4a5670, 11px); right modules gap 16px: net, cpu (starlight #a3bff2), mem, gpu, vol, battery (gold #d9b06a), datetime (bright #f2f5fb) `26·08·2026 ☾ 14:32`.
- **Tile area**: CSS-grid equivalent, columns 640px / flex / 580px, rows 580px / flex, **1px seams** colored rgba(166,146,236,.30) (in the compositor: 1px border, unfocused rgba borders as below). Focused window: 1px border rgba(166,146,236,.6) + faint inset glow. Unfocused: 1px rgba(102,116,142,.3)-ish.
- **Window headers, 26px**: glyph + god name + role epithet (10.5px, #4a5670), right-aligned `◈ ─ ✕` glyph buttons (letter-spacing 2px). Header accent border-bottom uses the pane's pantheon color at ~.3 alpha.
- **Panes**: odin harness (violet, spans left column: familiar table + timestamped log + status footer with `⌨ mod+a attach` hint), thoth shell (teal), hermes browser (cyan, url bar + reader page), horus monitor (gold header; starlight bars, braille sparklines for net/disk, process table, footer `⌨ mod+x expand`), urania orrery (starlight; three conic-gradient ring gauges 92px, planetary-glyph readout grid, almanac transits, wards footer).
- **Which-key bar, 26px bottom** (toggleable): `⊞ mod` prefix (violet, right border), then key hints — key in #f2f5fb, label in #93a1ba, gap 18px: `↵ thoth · d hecate · b hermes · j/k focus · h/l swap · 1-6 orbit · s stow · m mono · f full · r resize · q banish`, right: `? grimoire — full spellbook` (? in gold).
- **Hecate launcher overlay** (mod+d): dimmed scrim rgba(4,5,10,.6); 640px panel, border rgba(166,146,236,.65), glow shadow; header `⚷ HECATE` + query + 8×18px block cursor #a692ec; result rows 8px 18px padding, selected: bg rgba(166,146,236,.14) + 2px left border; result types tagged `run / cmd / spell`; footer `↵ invoke · ⇥ cycle · esc dismiss`.
- **Adjustable contrast**: user setting, implemented in prototype as backdrop-filter contrast(0.85–1.4, default 1.08). In production: derive palette variants from `palette.toml` instead of a filter.

### 2. File manager — charon (Files & Theming.dc.html, section 1a)
1400×900 reference; yazi theme/keymap target. Header 26px: `☍ charon — ferryman · ~/src/helm`, right: `hidden: off · preview: on (mod+p) · thumbnails: never`. Grid 240px parents / flex current / 460px preview. Current column: 11px column headers (name/size/mode/modified), rows 2px 14px padding; marked rows teal `✓` (#7fd4c1), flagged gold `⚑`; git status suffixes `±M`; cursor row bg rgba(166,146,236,.14) + 2px left border violet; footer: `41 items · 3 marked · 30.4K · sort: mtime ▾ · git: main +2 ~2`. Preview pane: syntax-lite code preview + stat/xattr block (11px #4a5670). Bottom `:` shell line (gold `:` prompt, bright text, block cursor) then which-key row (nowrap). No thumbnails, ever; previews are text/stat only and toggleable.

### 3. Open dialog — charon portal (section 1b)
720px modal over dimmed requesting app (scrim rgba(4,5,10,.45)). Header: `☍ CHARON · OPEN` + typed path with fuzzy segment, note `fuzzy · fd-backed`. Left rail 180px: HARBORS (pinned ⚓) + RECENT. Results: fuzzy-matched substring highlighted violet, right-aligned dir + size (11px); selected row same treatment as charon. Footer keys: `↵ open · ⇥ complete · mod+h hidden · mod+s shell here · esc dismiss`. Status: `preview off (mod+p) · 4 of 218 · scope: ~/src (mod+↑ widen)`.

### 4. Theming existing apps (section 1c)
Reference of a GTK4 app under the helm theme + annotated reach/limits. Implement as the `helm-theme` crate:
- Reaches: gtk3/4 via generated gtk.css (colors, square corners, flat headerbars, IBM Plex Mono); qt5/6 via qt6ct + Kvantum SVG from same palette; libadwaita named colors; terminal 16-color ANSI; electron force-dark; mono-line icon + cursor set.
- Limits (do not fight): hardcoded app colors; libadwaita geometry (colors yes, shapes no); CSD headerbars only recolored + 1px border; flatpak needs per-app config grant.
- Mechanism: `~/.config/helm/palette.toml` → templates → `helm ctl theme apply` hot-reload.

## Interactions & Behavior
- Everything keyboard-first; chords under one `mod`. Mode badge in bar reflects state (NAV / RESIZE / …); chord echo shows pending prefix. `?` opens a full keybind sheet ("grimoire").
- **No animations/transitions in v1.** State changes are instant: workspace switch, focus, launcher open/close, preview toggle. Later minimal-motion pass (opt-in): ≤120ms opacity-only fades on overlays; never move/scale windows.
- Launcher: fuzzy over PATH + desktop entries + "spells" (user scripts) + commands; ⇥ cycles, ↵ invokes.
- Bar modules update event-driven; clock 1s tick.
- Focus follows keyboard (j/k next/prev in ledger order, h/l swap); mouse works but is never required.

## State Management (compositor core)
- `Ledger`: ordered `Vec<WinId>` per orbit (workspace). All mutations (summon/banish/swap) edit the ledger; `relayout()` is a pure projection `fn(ledger, layout, workarea) -> geometries`. Keep ledger history for undo.
- Orbits: 6, runes ᚠᚢᚦᚨᚱᚲ; per-orbit layout (`triptych` master-stack shown; `mono` fullscreen-stack).
- Theme state: single palette struct; contrast variant setting.

## Design Tokens
Fonts: IBM Plex Mono (400/500/600) everywhere; fallback chain must include a Nerd Font/Symbola for runes + `☽⚚◉✶⚷☿♆♄⚔⛨𓂃` glyphs.
Sizes: bar 32px, window headers 26px, which-key 26px; body 12.5px, meta 11–11.5px, micro 10.5px, page title 20px; line-height 1.6–1.65. No border radius anywhere (except gauge circles). 1px borders only.
Colors:
- bg void `#05060c`, pane bg `#0a0c14`, focused pane bg `#0b0d16`, raised bg `#0e111b`, bar bg `#0a0c15→#080911`
- text `#c2cbde`, bright `#f2f5fb`, mid `#93a1ba` / `#aab6cc`, dim `#66748e`, faint `#4a5670`
- violet (arcane/focus/odin/hecate) `#a692ec`, starlight (urania/info) `#a3bff2`, gold (horus/warn/battery) `#d9b06a`, teal (thoth/marks/ok) `#7fd4c1`, cyan (hermes) `#7fc9e8`
- seams/borders: focused rgba(166,146,236,.6), pantheon headers at ~.3 alpha, neutral rgba(102,116,142,.3–.4)
- Keep saturated accent hues >25° apart (they already are) — don't add near-duplicates.
Bar/graph chars: `▰▱` meters, braille `⡀⣤⣶` sparklines, conic-gradient ring gauges.

## Assets
None — no images. All iconography is Unicode glyphs (runes U+16A0 block, planetary/alchemical symbols, ◈ ─ ✕ controls). Ship a tested fallback font stack; verify `𓂃` (Egyptian) renders or substitute `~`.

## Files
- `Desktop v3.dc.html` — canonical desktop (bar, tiles, which-key, hecate launcher, all five god panes)
- `Files & Theming.dc.html` — 1a charon file manager, 1b charon portal open dialog, 1c GTK theming reference + reach/limits notes
- `Desktop v2.dc.html`, `Desktop.dc.html` — earlier iterations, for provenance only (v3 supersedes; v1 has a deprecated cyberpunk/scanline direction)

## Suggested repo layout
```
helm/
├─ crates/
│  ├─ helm-compositor/   # Smithay; ledger + orbits + layouts (phase 3)
│  ├─ helm-bar/          # layer-shell bar + which-key + mode/chord echo
│  ├─ helm-hecate/       # layer-shell fuzzy launcher (nucleo)
│  ├─ helm-odin/         # ratatui agent-harness TUI
│  ├─ helm-theme/        # palette.toml -> gtk.css/kvantum/ansi/yazi/btop templates
│  └─ helm-ctl/          # CLI: orbit/ledger/theme/run commands
├─ configs/              # yazi keymap+theme, portal config, zsh/starship, btop theme
└─ palette.toml
```
