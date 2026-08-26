# ADR 0008 — `smithay-client-toolkit` + `tiny-skia` + `cosmic-text` for layer-shell clients

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

helm has three layer-shell surfaces: the 32px top bar, the 26px which-key strip
and the hecate overlay (640px panel over a dimmed scrim). Between them they must
draw a vertical gradient with four static starfield dots, six runes with a text
shadow glow, a mode badge, a centred title, right-aligned modules with braille
sparklines and `▰▱` meters, a which-key row, and a result list.

Four constraints:

- **Cold session start to usable, under 900 ms** (`docs/ARCHITECTURE.md` §4).
  The budget line for this explicitly says "no GPU context for the bar".
- **Bar redraw under 8 ms**, event-driven, idle CPU at roughly zero.
- **Font fallback is not optional.** The bar draws elder futhark runes, several
  Unicode symbol blocks, braille, and the design deliberately verifies one
  Egyptian hieroglyph. ADR 0012 makes the glyph inventory a contract. A text
  stack that cannot walk a fallback chain cannot implement it.
- **Rust-first**, and the workspace sets `#![forbid(unsafe_code)]` in
  `helm-core`. We would like clients to stay close to that.

Creating an EGL context, compiling shaders and uploading a texture atlas costs
tens of milliseconds and a few tens of megabytes of address space. For a surface
that is 1920×32, redrawn perhaps twice a second, that is the wrong trade in
every direction.

## Decision

Layer-shell clients (`helm-bar`, `helm-hecate`) use:

| Layer | Crate | Version | Role |
|---|---|---|---|
| Wayland protocol | `smithay-client-toolkit` | 0.21 | `wlr-layer-shell`, `wl_shm` buffer pool, seat and output handling |
| Rasteriser | `tiny-skia` | 0.12 | CPU rendering of rects, the bar gradient, borders and glyph masks |
| Text | `cosmic-text` | 0.19 | Font discovery, shaping, and — the reason it is here — fallback chain resolution |

- Buffers are `wl_shm`, allocated at the output's real scale, redrawn only on
  state change. No EGL, no GPU context, no shaders in any helm client.
- `cosmic-text`'s font database backs the glyph probe (ADR 0012): the coverage
  predicate passed to `glyphs::Probe::run` is a query against the resolved font
  stack from `palette.toml`'s `typography.fallback`.
- Damage is tracked: `wl_surface::damage_buffer` is given only the regions that
  changed, and `HelmState::renders_same_as` drops frames that would be identical.

Versions are pinned deliberately. These three crates move quickly and their
Wayland-facing APIs are not stable across minor releases.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **`iced` with layer-shell support** | A real widget toolkit: layout, event handling, text input and a styling system, all of which we would otherwise hand-roll. `helm-hecate` in particular is a list plus a text field, which is exactly what a toolkit is good at | Brings a GPU context (wgpu) for a 32px bar, which the cold-start budget forbids by name. Its software backend exists but is not the maintained path. A retained widget tree also fits badly with "redraw on state change, never on a timer", and the dependency footprint is an order of magnitude larger |
| **`eww` as a permanent solution** | Already works; the design is CSS-shaped, so the prototypes translate almost directly; no Rust client to write at all; the fastest possible route to a visible bar | Not a stopgap we can live with: it means a GTK process, a CSS engine and a scripting layer in the session's critical path, and it polls by default. It also puts helm's most visible surface outside our own theming pipeline. Worth keeping as a *scaffold* during early M2 to see the design on screen, but not shipped |
| **GTK4 with `gtk4-layer-shell`** | Mature, excellent text rendering via Pango, accessible out of the box, and every distro already ships it | Drags GTK into the session's critical path, which means the bar's startup depends on GTK's theme loading, icon cache and settings daemon — the exact machinery we are trying not to need. GTK is also a target of our theming (ADR 0005), so the bar would be simultaneously theming GTK and themed by it |
| **`smithay-client-toolkit` + `femtovg` or `vello`** | Better quality output and a path to GPU acceleration later | Both want a GPU context, which is the thing we are avoiding. `vello` also has no CPU path worth relying on yet |
| **`fontdue` or `ab_glyph` instead of `cosmic-text`** | Much smaller; faster for simple ASCII; fewer dependencies | Neither does font *fallback*. They rasterise a face you hand them. Walking a chain of six families to find the one that has `𓂃` is exactly the work `cosmic-text` exists to do, and it is the requirement ADR 0012 turns into a contract. Rolling our own fallback resolution on top of `fontdb` would be reimplementing the interesting half of `cosmic-text` |

## Consequences

### Good

- No GPU context anywhere in helm's own clients, so cold start stays inside 900
  ms and the bar works identically on a 2015 laptop, in a VM and over a remote
  session with no acceleration.
- Real font fallback, which is what makes the glyph inventory implementable
  rather than aspirational.
- Pure Rust from the socket to the pixel. No C toolkit in the session's path.
- CPU rasterising 1920×32 pixels is genuinely cheap and predictable, with no
  driver variance to make the 8 ms budget probabilistic.
- Small dependency surface, which matters for three distro packagings.

### Bad

- We hand-roll layout, hit testing and text input. `helm-hecate` needs a text
  field with a cursor and selection, and we will write it ourselves.
- Three fast-moving dependencies with unstable APIs. Upgrading any of them is a
  real task, and `smithay-client-toolkit` in particular has broken across minor
  versions before.
- No accessibility for free. A GTK bar would expose an AT-SPI tree; ours exposes
  nothing until we do the work, which is on the M6 list.
- CPU rasterising does not scale to large surfaces. This stack is right for a
  bar and an overlay and would be wrong for a full-screen client.
- Text shaping quality is `cosmic-text`'s, and complex scripts in window titles
  will only be as good as it is.

### Neutral

- `tiny-skia` has no blur, no shadow and no rounded corners. The design has none
  of those either (the "inset glow" is a static 1px inset border). The
  rasteriser's limits and the design's rules coincide.
- `helm-odin` is a `ratatui` TUI in a terminal and is unaffected by this ADR.

## Reversal

Medium. The stack is confined to `crates/helm-bar` and `crates/helm-hecate`, but
it is most of what those crates *are*: a swap is a rewrite of their drawing and
event layers, roughly one to two weeks each. Nothing in `helm-core`,
`helm-session` or `helm-theme` would change, because clients hold no state
(ADR 0003) and read colours from a generated file (ADR 0005).

Swapping the text stack alone is much cheaper than swapping the whole thing, and
swapping the rasteriser alone is cheaper still.

Signals to reconsider: a measured cold start or redraw miss attributable to CPU
rasterising; `smithay-client-toolkit` becoming unmaintained; or an accessibility
requirement that a toolkit would satisfy and we cannot.

## Guard

- *Planned (M2):* a cold-start benchmark in CI asserting the bar's first frame
  lands within the 900 ms session budget on the reference runner.
- *Planned (M2):* a redraw benchmark asserting a full bar repaint stays under
  8 ms, run against the same `HelmState` fixtures the unit tests use.
- *Planned (M2):* an idle test that runs the bar for sixty seconds with no state
  change and asserts the number of committed frames is at most sixty — one per
  clock tick and not one more.
- *Planned (M2):* a link-time assertion that no helm client binary links EGL,
  GL or Vulkan. This is the guard for "no GPU context", which is the part of
  this decision most likely to erode by accident.
- `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu` — the
  contract the text layer must satisfy (ADR 0012).
