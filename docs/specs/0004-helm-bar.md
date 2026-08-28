# SPEC 0004 — helm-bar

- **Status:** Accepted (2026-08-26) — not yet implemented
- **Milestone:** M2
- **Decisions:** [ADR 0008](../adr/0008-layer-shell-rendering-stack.md),
  [ADR 0009](../adr/0009-no-animation-budget.md),
  [ADR 0012](../adr/0012-font-fallback-is-a-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md)
- **Implements:** [INTERFACES.md §3](../INTERFACES.md)
- **Supersedes / Superseded by:** —

> Written before the code, as S14 requires. The **Test** column below is
> deliberately empty: those tests get written next, watched to fail, and only
> then implemented against. Filling the column in is what moves this spec to
> Implemented.

## Purpose

The bar is the only place helm tells the user what state it is in. Which orbit
is shown, what layout it holds, which mode the keyboard is in, what a
half-typed chord is waiting for, which window has focus, and whether the
machine is on fire. A tiling desktop without it is not a desktop a person can
use for a week ([MVP.md](../MVP.md), capability 3), and the which-key strip is
the difference between a chorded keymap and a chorded keymap nobody remembers
(capability 4).

It is also the surface where the two rules that are easiest to break quietly
live: draw only when something changed, and never draw a glyph the font stack
cannot render.

## Scope

**In:** the three layer-shell surfaces (32 px bar, 26 px which-key strip,
grimoire sheet); their layers, anchors and exclusive zones; every segment's
source of truth and palette token; the render, damage and gating rules; font
chain resolution and the glyph probe; how the bar behaves when it does not fit
the output.

**Out:** producing `HelmState` at all — that is `helm-session`, which owns the
ledger, samples the modules and broadcasts snapshots (ADR 0003). Serving
`river-layer-shell-v1`, without which none of these surfaces exist
(ADR 0013, `helm-session`). Generating the palette (`helm-theme`, SPEC 0002).
The hecate launcher, which is a separate layer-shell client on the same stack
(M4). Window headers, which are drawn by the compositor's borders and by each
client, not by the bar.

## Behaviour

### 1. Surfaces

Three `wlr-layer-shell` surfaces, all `wl_shm`, all CPU-rasterised, no GPU
context anywhere (ADR 0008).

| Surface | Layer | Anchors | Size | Exclusive zone | Keyboard |
|---|---|---|---|---|---|
| Bar | `Bottom` | top, left, right | height `metrics.bar_height` (32) | `metrics.bar_height` | none |
| Which-key strip | `Bottom` | bottom, left, right | height `metrics.whichkey_height` (26) | `metrics.whichkey_height` | none |
| Grimoire | `Overlay` | all four | full output | `-1` (reserves nothing, ignores others' zones) | none |

`Bottom` rather than `Top` is a deliberate choice and it is the whole of the
bar's fullscreen behaviour; see below.

None of the three takes keyboard interactivity. Every surface is a pure view of
`HelmState`; the keys that toggle the strip and the grimoire are ordinary
keymap bindings (`Action::ToggleWhichKey`, `Action::Grimoire`) resolved by
`helm-session`, which flips a field in the state and broadcasts it. A bar that
grabbed the keyboard would be a second input path, and helm has one.

**These surfaces exist at river's discretion, and river has none of its own.**
`wlr-layer-shell` works under river 0.4 only if the window manager implements
`river-layer-shell-v1`, and helm's window manager is `helm-session`
(ADR 0013). If `helm-session` does not serve that protocol, the bar does not
appear at all — and the symptom reads as a broken bar rather than a missing
protocol, which is why `docs/PITFALLS.md` carries it as its own row and why
`helm ctl doctor` checks it by name (SPEC 0006).

**The exclusive zone is the only thing that reserves space.** river turns the
zones into `river_layer_shell_output_v1::non_exclusive_area`, which reaches
`helm-session` as a free rectangle in global coordinates and is converted into
a `Workarea` by the backend ([INTERFACES.md §1](../INTERFACES.md)).
`helm-session` uses that rectangle as given. It must not *also* subtract
`metrics.bar_height` and `metrics.whichkey_height` from the output, or the
tiles lose the strips twice and every window sits 58 px short with the void
showing at the bottom.

Toggling the which-key strip off destroys its surface, which releases its zone,
which grows the non-exclusive area, which relayouts the tiles. That chain is
the intended mechanism and not a side effect: the bar never tells the session
how tall it is by any other route.

Buffers are allocated at the output's integer scale and all geometry is
computed in device pixels — every metric from `palette.toml` is logical and
multiplied by the scale factor, never rounded from a float. Fractional scaling
(`wp_fractional_scale_v1`) is out of scope for M2; on a fractional output the
bar renders at the next integer scale up and the compositor downscales, which
is the documented behaviour rather than a surprise (`docs/PITFALLS.md`,
"fractional scaling blur").

**Fullscreen, and why the strips are on `Bottom`.** When the active orbit has a
fullscreen window it is placed at `Workarea::output` — the whole output,
including the strips' rectangles. Which of the two draws on top is decided by
the bar's chosen **layer** and by nothing else: `river-layer-shell-v1` exposes
no node and no ordering request at all ([INTERFACES.md §1](../INTERFACES.md)),
so there is no node order for `helm-session` to pick. On `Top` the bar would
draw over a fullscreen video; on `Bottom` the window covers it, which is what
`mod+f` is asking for.

`Bottom` costs nothing the rest of the time, because helm *is* the window
manager: every window's rectangle comes from `layout::project` and lands inside
the non-exclusive area, so nothing but a fullscreen window can ever be over the
strips' rectangles. There is no misbehaving client to defend against, which is
the usual reason a bar sits on `Top`.

The bar is not unmapped while a window is fullscreen and its exclusive zone
does not change, so entering and leaving fullscreen costs no relayout of the
other orbits.

### 2. Segments

Left to right. "Field" is the `palette.toml` path, read through
`helm_core::Palette`; a literal in the bar is the same bug as a colour outside
the palette. Prototype values are quoted from `design/Desktop v3.dc.html`,
whose styling is inline on each element; where the prototype and `palette.toml`
disagree the palette wins (`design/README.md`), and every such case is listed
under [Where the design was silent or inconsistent](#where-the-design-was-silent-or-inconsistent).

**Background.** A vertical gradient from `background.bar_top` to
`background.bar_bot`, a bottom rule 1 px (`metrics.border_width`) in
`border.bar_bottom` at `border.bar_bottom_alpha`, and four static 1 px
starfield dots. The prototype's bar `<div>` carries
`background: radial-gradient(1px 1px at 18% 40%, rgba(210,225,255,.6) …), … ,
linear-gradient(180deg, #0a0c15, #080911); border-bottom:1px solid
rgba(166,146,236,.4)`. The four dots sit at 18%/40%, 43%/70%, 67%/30% and
88%/60% of the bar's width and height, at alphas 0.60, 0.40, 0.50 and 0.35,
rounded to integer device pixels. They are decoration and they never move.

**Logo.** `✦ helm` in `accent.violet` at `typography.size_body`, 14 px padding
either side, letter spacing 1 px, followed by a 1 px right rule in
`border.seam` at `border.seam_alpha`. Source of truth: nothing — it is
constant. The `✦` is `glyphs::inventory()`'s `"logo"` entry and is drawn
through `Probe::resolve`.

**Six orbit runes.** Source of truth: `HelmState::orbits`, always six
`OrbitCell`s, each carrying `number`, `rune`, `display` and `windows`. The
group has 6 px padding either side; each cell is 28 logical px wide and the
full height of the bar. The rune drawn is `Probe::resolve(cell.rune)` — never
`cell.rune` itself, and never `OrbitId::rune()` recomputed locally.

| `OrbitDisplay` | Treatment |
|---|---|
| `Active` | glyph in `text.bright`; a 2 px bottom border in `accent.violet`; the cell filled with `accent.violet` at alpha 0.14; plus the glow described below |
| `Occupied` | glyph in `text.mid`, no fill, no border |
| `Empty` | glyph in `text.faint`, no fill, no border |

The 2 px border is the one place in helm where a border is not 1 px, and it is
the prototype's `border-bottom:2px solid #a692ec`. `cell.windows` is not drawn:
it exists for `helm ctl orbit list` (SPEC 0006) and for a tooltip helm does not
have.

**Layout indicator.** `⌗ ` + `Layout::label()` in `text.dim`, preceded by a
1 px left rule in `border.neutral` at `border.neutral_alpha` and 12 px padding.
Source of truth: `HelmState::layout`. `label()` yields `triptych`, `mono` or
`even`; the bar does not spell those words itself.

**Mode badge.** `⌨ ` + `Mode::badge()` in `accent.starlight` on a fill of
`accent.starlight` at alpha 0.14, at `typography.size_meta`, padded 1 px by
8 px, letter spacing 1 px, 10 px after the layout indicator. Source of truth:
`HelmState::mode`. `badge()` yields the bare word — `NAV`, `RESIZE`, `MOVE` —
and the bar prepends the resolved `⌨`.

**Centre: focused title and chord echo.** A flexible region, contents centred,
14 px between the two. The title is `HelmState::focused_title` verbatim in
`text.soft`. The chord echo is `HelmState::chord_echo` in `text.faint` at
`typography.size_meta`, and is empty when no chord is pending — the prototype's
`mod+ ▸ awaiting chord…` is what the session puts there mid-chord, not a
string the bar composes. Both are arbitrary text from the session; see
[Font handling](#6-font-handling) for what happens to a codepoint no font
covers.

**Right-hand modules.** Source of truth: `HelmState::modules`, in the order the
session sends them, laid out right to left with 16 px between modules and 16 px
of padding at the right edge. Each `Module` carries `id`, `text`, an optional
`accent` name and `urgent`.

- `text` is drawn verbatim; the bar formats nothing.
- `accent` is looked up with `Accents::by_name`; `None` means `text.mid`.
  An unknown name means `text.mid` and one warning in the log, never a panic
  and never a fallback colour invented on the spot.
- `urgent` fills the module's box with its accent at alpha 0.14, the same wash
  the mode badge and the active rune use. It does not blink, pulse or fade
  (ADR 0009).

The prototype's row is `↑ 18k ↓ 1.2M` (default), `cpu 31%`
(`accent.starlight`), `mem 9.8G`, `gpu 44°`, `♪ 64%`, `⚡ 87%`
(`accent.gold`), `26·08·2026 ☾ 14:32` (`text.bright`). Those strings and their
accents are the session's business, not the bar's — except the clock, which is
the bar's own; see [Module data sources](#7-module-data-sources).

### 3. The which-key strip

Content is `Keymap::strip()` in iteration order. That order is already locked
by `keys::tests::strip_matches_the_reference_which_key_row`, which asserts it
against the mockup row: `↵ thoth · d hecate · b hermes · j/k focus · h/l swap ·
1-6 orbit · s stow · m mono · f full · r resize · q banish`. The bar renders
that iteration and must not sort, filter or re-order it. Changing the row means
changing the keymap and that test, not the bar.

Layout, from the prototype's strip `<div>`: height `metrics.whichkey_height`,
background `background.bar_bot`, a 1 px top rule in `border.seam` at
`border.seam_alpha`, 12 px horizontal padding. Then `⊞ mod` — `Keymap::modifier`
prefixed by the resolved `⊞` — in `accent.violet` with 12 px right padding and
a 1 px right rule in `border.neutral` at `border.neutral_alpha`; then the hints
14 px further in, 18 px apart, each `Binding::hint_key` in `text.bright`
followed by a space and `Binding::label` in `text.mid`; then, right-aligned,
`? grimoire — full spellbook` in `text.dim` with the `?` in `accent.gold`.

Visibility is `HelmState::whichkey`. False destroys the surface (§1).

**When it does not fit.** The design does not say, and it is a real case: at
the reference type sizes the strip's natural width is roughly 1020 logical px,
so it fits at 1366 px with room to spare and does not fit at 1024 px, in a
small VM window, or on a portrait display. The specified behaviour, in order:

1. Drop the right-hand `? grimoire — full spellbook` label.
2. Then shed hints from the **right** of the row, one at a time — `q banish`
   first — because the order is locked and shedding from the middle or
   re-ordering to fit would break the one thing the row is for: the same key is
   always in the same place.
3. Append the elision glyph `…` after the last hint that fits, so the row says
   plainly that there are more. The grimoire has all of them.
4. The strip never unmaps itself and never changes its height. At any width it
   shows at least `⊞ mod` and the elision, and its exclusive zone is constant,
   so a resize never relayouts the tiles.

Fit is recomputed when the state, the palette, the probe or the output width
changes — never per frame.

### 4. The grimoire

The full keybinding sheet, on `?` (`Action::Grimoire`). There is no prototype
for it: `design/HANDOFF.md` names it twice and
`design/Desktop v3.dc.html` only advertises it from the which-key strip. It is
specified here from the vocabulary the rest of the design already uses.

- Source of truth: `HelmState::grimoire` (a new field, see
  [Additions required in helm-core](#additions-required-in-helm-core)) for
  whether it is shown, and the session's `Keymap` for what it lists.
- A scrim over the whole output in `background.void` at alpha 0.60, matching the
  hecate overlay's weight.
- A panel `metrics.portal_width` (720) logical px wide, centred horizontally,
  its top at `2 × metrics.bar_height` from the top of the output, its height
  the content's, capped at the output height less `4 × metrics.bar_height`.
  Background `background.pane`, a 1 px border in `border.focused` at
  `border.focused_alpha`. No radius, no shadow, no blur.
- A header row `metrics.header_height` tall: `✦ GRIMOIRE` in `accent.violet` at
  `typography.size_body`, and `esc dismiss` right-aligned in `text.faint` at
  `typography.size_micro`, over a 1 px bottom rule in `border.seam` at
  `border.seam_alpha`.
- The body lists **one row per binding whose `hint_key` is non-empty**, in
  `Keymap::bindings` order, grouped by `Binding::mode` with the mode's
  `badge()` as a group heading in `text.dim`. A binding with an empty
  `hint_key` is already represented by its pair — `k` by `j/k`, `l` by `h/l`,
  orbits 2 to 6 by `1-6` — and must not get a row of its own. Against the
  default keymap that is 17 rows: the 11 in the strip plus `t triptych`,
  `u undo`, `w which-key`, `? grimoire`, `p theme` and `esc nav`.
- Each row: `⊞ mod` + `hint_key` in `text.bright`, `label` in `text.mid`, and
  the action spelled out in `text.dim` at `typography.size_micro`
  (`spawn helm-term`, `orbit 1-6`, `set-layout mono`). Two columns at 720 px.
- Footer in `text.faint` at `typography.size_micro`: the binding count and
  `? or esc dismiss`.

Dismissal is `?` again or `esc`, both handled by the session, which clears the
field. The grimoire holds no state of its own, takes no keyboard focus, and
does not dim or alter the bar or the strip — it is an `Overlay` surface and
simply draws above them.

### 5. Rendering rules

These are the heart of the spec and they are already committed in
[INTERFACES.md §3](../INTERFACES.md):

```rust
fn render(state: &HelmState, palette: &Palette, probe: &Probe, canvas: &mut Pixmap) -> Damage;
pub struct Damage(Option<Rect>);
```

**Pure function.** For the same `(HelmState, Palette, Probe)` and the same
canvas dimensions, `render` produces byte-identical pixels. It performs no I/O,
consults no clock, queries no font database and reads no environment. Anything
that varies with time enters through the state, never through the drawing code.

**Gated before any drawing.**

```rust
if state.renders_same_as(&last_drawn) { return Damage(None); }
```

`HelmState::renders_same_as` compares everything visible and ignores
`revision`, so a module that recomputes to the same string costs nothing and a
bumped counter costs nothing
(`state::tests::revision_alone_does_not_force_a_redraw`). `Damage(None)` means
no buffer is attached and no frame is committed — not a committed frame with an
empty damage region.

The session applies the same comparison before it broadcasts at all
([SPEC 0003](0003-helm-session.md) §8), so in a correct session this check
never fires. It stays anyway, one process downstream, because it costs an
equality test on a struct the bar is holding either way and it is the last
thing between a careless producer and a frame the user cannot see.

**Damage, not repaint.** `render` returns the union of the bounding boxes of
the segments whose inputs changed since the last committed frame. A clock tick
damages the clock. A focus change damages the centre. An orbit switch damages
the rune group, and usually the centre too, because the focused title changed
with it — a union spanning two distant segments approaches a full repaint, and
that is accepted: it is the rare case, and the frequent cases are small.

Because the right-hand modules are laid out right to left, a module whose
*width* changes shifts everything to its left. Its damage is therefore the
union of its old box, its new box, and every module to its left. The clock is
the rightmost module and its format is fixed-width, so a minute rollover
changes no width and damages nothing but the clock — which is the case the rule
exists for.

**Buffer age.** The bar keeps one shadow pixmap holding the complete current
frame and a double-buffered `wl_shm` pool. When re-using a pool buffer, the
region copied from the shadow pixmap is the union of this frame's damage and
the damage of every frame committed since that buffer was last used; the region
reported to `wl_surface::damage_buffer` is this frame's damage alone. Copying
less than the union leaves a stale strip in the buffer; reporting more than the
frame's damage costs the compositor a needless upload.

**Every glyph goes through `Probe::resolve`.** A raw `char` from the inventory
in drawing code bypasses the fallback contract and is how tofu ships
([INTERFACES.md §3](../INTERFACES.md), rule 3; ADR 0012). No non-ASCII literal
may appear in the bar's drawing code at all; every chrome glyph comes from
`glyphs::inventory()`.

**No timers.** The bar has none to justify; the clock's is the session's (§7).

Shaping results for chrome strings — the logo, the runes, the badges, the
which-key row — are shaped once and cached, and the cache is invalidated only
when the palette or the probe changes. Text that comes from the state is
shaped when it changes.

### 6. Font handling

At startup, before the first frame:

1. Build a `cosmic-text` font database and an explicit family list from
   `palette.toml`: `typography.family` first, then `typography.fallback` in
   order — `IBM Plex Mono`, `Symbols Nerd Font Mono`, `Noto Sans Symbols 2`,
   `Noto Sans Egyptian Hieroglyphs`, `Symbola`, `DejaVu Sans Mono`. The chain
   is ordered and explicit precisely so that fontconfig's system default cannot
   put an emoji font in front of it and render the runes in colour at the wrong
   size (`docs/PITFALLS.md`, "emoji font hijacks symbols"; ADR 0012 point 5).
   `Palette` already refuses an empty `typography.fallback` at parse time.
2. `let probe = Probe::run(|c| /* some face in the configured chain covers c */)`.
   That is 37 codepoint lookups, once, on the cold-start path.
3. Log `probe.summary()` once — `fonts: 37/37 glyphs covered`, or
   `fonts: 24/37 glyphs covered; substituting for ᚠᚢᚦ…`. `helm ctl doctor`
   runs the same probe and prints the same line (SPEC 0006).
4. Draw. `Probe::resolve(ch)` returns the glyph when covered and the
   inventory's documented ASCII fallback when not.

**On a machine with no symbol font at all** — a container, a live ISO, a
minimal server install — the chain still resolves through `DejaVu Sans Mono` or
whatever last-resort monospace the system has, and every non-ASCII chrome glyph
is missing. The bar then draws `*` for the logo, `1` to `6` for the runes, `#`
for the layout indicator, `:` for the mode badge, `>` for the chord echo, `)`
for the moon, `^` for the battery, `~` for the volume, `+` for the modifier and
`<` for return. It is plain and it is ugly and it is entirely legible; nothing
is a box, nothing is missing, and because every substitution is exactly one
character wide, no segment reflows and the bar looks the same shape as on a
fully-fonted machine. `doctor` names the glyphs and the chain that was tried,
so the user knows what to install.

If **no** family in the chain resolves to an installed face, the bar takes the
text stack's last-resort face, logs a warning naming every family it tried, and
renders. It does not refuse to start: a bar that will not start is a black
screen, which is worse than a bar in the wrong typeface.

**Text from the state is not covered by the inventory and cannot be.** Window
titles are arbitrary; so are module strings and the chord echo. Those are
passed to `cosmic-text`, which walks the configured chain per run. A codepoint
that no face in the chain covers is replaced with `?` **before shaping**, so
the bar never rasterises a `.notdef`. This is the one place the bar makes a
substitution the inventory did not specify, and it is unavoidable.

### 7. Module data sources

**The bar owns no timer at all**, and it holds no module of its own. Every
`Module` — the clock included — is produced by `helm-session` and arrives in
`HelmState::modules`; the bar draws them verbatim, in the order they were sent
([SPEC 0003](0003-helm-session.md) §8). That is the ownership question
[INTERFACES.md §3](../INTERFACES.md) left open: rule 1 says "no timers except
the clock" without saying whose it is, and SPEC 0003 settles it in the session,
where a `timerfd` sits in the event loop beside everything else.

Two consequences for this spec. The bar satisfies rule 1 trivially, having no
timers to justify. And the primary no-op gate is one process upstream — the
session compares candidate states with `renders_same_as` and does not broadcast
at all when they render the same — so the bar's own check before drawing is
belt and braces rather than the only defence. It stays: a subscriber that
reconnects, or a future producer that is less careful, must not cost a frame.

What the bar receives, and why each source is honest about being an event or a
sample. The ladder is the `helm-perf-budget` skill's: an event if one exists, a
producer that can be asked to push if not, a sample only if the data is
genuinely not observable any other way.

| Module | Source | Mechanism | Ladder |
|---|---|---|---|
| clock | wall clock | `timerfd` scheduled to the next minute boundary in `helm-session` | minute-aligned display update (ADR 0009) |
| battery | `power_supply` | udev uevent | step 1 — an event exists |
| vol | PipeWire / WirePlumber | node param change | step 1 — an event exists |
| net | `/proc/net/dev` | sampled | step 3 |
| cpu | `/proc/stat` | sampled | step 3 |
| mem | `/proc/meminfo` | sampled | step 3 |
| gpu | hwmon temperature | sampled | step 3 |

**The justification the four sampled modules owe.** `cpu` and `net` are ratios
over an interval; there is nothing to signal, because the value does not exist
until two samples have been taken and divided. `mem` and the hwmon temperature
have a current value and no notification mechanism at all — nothing in the
kernel offers to tell a userspace process that `MemAvailable` moved. Netlink
and udev were checked first and answer a different question: netlink reports
link state, not throughput, and thermal uevents fire on trip points, not on
degrees. So all four reach step 3, are sampled at the coarsest interval that
still reads correctly, and are sampled in **one** pass rather than four, so the
cost is one wake rather than four.

Two properties of that arrangement the bar depends on, both owed by SPEC 0003:
a sample that recomputes to the same string must not be broadcast, and a module
update must never take a compositor round trip. Under river a stalled window
manager is a dead session rather than a slow frame (ADR 0013), and a 1 Hz
round trip for a clock would multiply the session's own input latency.

**What this costs in frames.** A minute-granular clock changes its text once a
minute, so the tick produces about one committed frame per minute rather than
one per second. ADR 0008's planned idle test — sixty seconds with no state
change, at most sixty committed frames — passes with one.

### Additions required in `helm-core`

None of these are workarounds; the types do not yet carry what the bar needs.
Each is a revision of [SPEC 0001](0001-helm-core-contracts.md), made in the
same commit as the change, per `docs/specs/README.md`.

| Addition | Why |
|---|---|
| `HelmState::grimoire: bool`, compared in `renders_same_as` | `Action::Grimoire` exists in `keys.rs`, but no field tells the bar the sheet is open. Without it the bar cannot draw the grimoire at all |
| `glyphs::inventory()` gains `…` — name `"elision"`, `Surface::Bar`, fallback `'.'` | The strip and the title both elide, and no glyph in the inventory says "there is more". The fallback is one character wide, so the elided layout is the same shape on an ASCII-only machine |
| `ipc::Request::GetKeymap` → `ipc::Response::Keymap(Box<Keymap>)` | The which-key strip and the grimoire are both rendered from `Keymap`, which the session owns and the bar has no way to ask for. It does not belong in `HelmState`: it changes only when the configuration does, and putting it in a per-change broadcast would add kilobytes to every frame's worth of traffic. The bar fetches it once after `Hello` |

`Keymap` already derives `Serialize` and `Deserialize`, so the wire type needs
no other change.

### Where the design was silent or inconsistent

Recorded here rather than resolved in code, so nobody has to re-derive them.
`design/README.md`'s rule applies throughout: where a prototype and
`palette.toml` disagree, the palette wins.

| Case | What the prototype says | What this spec specifies |
|---|---|---|
| Bar overflow at narrow widths | nothing; the reference is 1920×1080 | the collapse ladder below |
| Which-key overflow | nothing | §3, shed from the right and append `…` |
| The grimoire | named, never drawn | §4, specified from the existing vocabulary |
| Active rune glow | `text-shadow:0 0 8px rgba(166,146,236,.9)` | a blur, which ADR 0009 bans and `tiny-skia` cannot do. The glyph's alpha mask is composited four times at ±1 device pixel in `accent.violet` at alpha 0.35 beneath the bright glyph — a static 1 px dilation, no blur, no animation |
| Starfield dot colour | `rgba(210,225,255,…)` | `#d2e1ff` is in no palette field. Drawn from `text.bright` at the prototype's four alphas; the difference is imperceptible at 1 px and it keeps every colour in `palette.toml` |
| Divider alphas | violet at `.35` (logo rule, which-key top rule), grey at `.35` (layout group rule) | there is no 0.35 in `palette.toml`. All internal dividers use `border.seam`/`border.neutral` at their 0.30 alphas; only the bar's bottom rule is `border.bar_bottom` at 0.40, which the prototype already has |
| Which-key type size | `font-size:11.5px` | 11.5 is not in the type scale. `typography.size_meta` (11.0) |
| Hecate scrim, reused by the grimoire | `rgba(4,5,10,.6)` — `#04050a` | `background.void` (`#05060c`) at alpha 0.60 |
| `Module::urgent` | not shown | the module's accent as a 0.14 wash behind it, matching the mode badge and active rune. It does not blink |
| Clock ownership | "clock 1s tick", owner unstated | settled by [SPEC 0003](0003-helm-session.md) §8 in `helm-session`; ADR 0009 refines the displayed clock to the next minute boundary. The bar has no timer (§7) |
| Bar over a fullscreen window | not drawn; ADR 0013 assumed node ordering would decide | `river-layer-shell-v1` has no node ordering, so the layer decides: both strips sit on `Bottom` and a fullscreen window covers them (§1) |
| Uncoverable codepoints in titles | not considered | replaced with `?` before shaping (§6) |

**The bar's collapse ladder.** At the reference type sizes — IBM Plex Mono's
0.6 em advance, `size_body` 12.5 and `size_meta` 11.0 — the fixed segments
(logo, runes, layout, badge, the seven prototype modules and their padding)
measure roughly 1005 logical px, leaving about 360 px of centre at 1366 px and
nothing at all below about 1100 px. Those numbers must be **measured from the
resolved face at runtime**, not compiled in: the advance depends on which
family in the chain actually loaded. Applied in order until the row fits:

1. Elide `focused_title` from its middle with `…`, down to a floor of eight
   characters plus the elision.
2. Below that floor, drop `focused_title` entirely if `chord_echo` is
   non-empty. A pending chord is more urgent than a window title, and the title
   is one glance away in the window's own header.
3. Drop right-hand modules from the left of the group in this fixed priority
   order, lowest first: `gpu`, `net`, `mem`, `vol`, `cpu`. A dropped module
   leaves no gap. `battery` and `clock` are never dropped.
4. Drop the layout indicator's label, leaving the resolved `⌗` alone; then the
   logo's text, leaving `✦` alone.
5. The six orbit runes and the mode badge are never dropped at any width. They
   are what the bar exists to show.

The bar never unmaps itself and never changes height, so no width change ever
relayouts the tiles.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a 1920×1080 output and the shipped palette, when the bar starts, then it maps a `Bottom` layer surface anchored top/left/right, `metrics.bar_height` tall, with an exclusive zone of `metrics.bar_height` | |
| A2 | Given `whichkey` true, when it becomes false, then the strip's surface is destroyed, its exclusive zone is released, and the reported non-exclusive area grows by `metrics.whichkey_height` | |
| A3 | Given a state with orbit 1 `Active`, 2–3 `Occupied` and 4–6 `Empty`, when a frame is rendered, then cell 1 is drawn in `text.bright` over an `accent.violet` wash at 0.14 with a 2 px `accent.violet` bottom border, cells 2–3 in `text.mid`, cells 4–6 in `text.faint`, each cell 28 logical px wide | |
| A4 | Given `Layout::Mono` and `Mode::Resize`, when a frame is rendered, then the bar shows `Layout::label()` in `text.dim` after the resolved `⌗`, and `Mode::badge()` in `accent.starlight` on a starlight wash at 0.14 at `typography.size_meta` | |
| A5 | Given a rendered frame, when a state arrives for which `renders_same_as` is true, then `render` is not called, no buffer is attached and no frame is committed | |
| A6 | Given a frame differing only in the clock module's text at an unchanged width, when it is rendered, then the returned `Damage` contains the clock module's box and no other module's box | |
| A7 | Given a module with `urgent` true and accent `gold`, when a frame is rendered, then its box is filled with `accent.gold` at alpha 0.14 and its text drawn in `accent.gold`, with no other module's box changed and nothing blinking | |
| A8 | Given a `Probe` built from an ASCII-only coverage predicate, when a full frame is rendered, then every chrome glyph drawn is the inventory's documented ASCII fallback, no `.notdef` mask is rasterised, and every segment keeps its width | |
| A9 | Given a `focused_title` containing a codepoint no configured face covers, when it is rendered, then that codepoint is drawn as `?` and no `.notdef` mask is rasterised | |
| A10 | Given `whichkey` true on a 1920 px output, when the strip is rendered, then it lists exactly `Keymap::strip()` in order, `hint_key` in `text.bright` and `label` in `text.mid`, followed by `? grimoire — full spellbook` with the `?` in `accent.gold` | |
| A11 | Given a 640 px output, when the strip is rendered, then it shows the leading hints that fit in `Keymap::strip()` order followed by `…`, drops the grimoire label first, re-orders nothing, and keeps its exclusive zone | |
| A12 | Given a 1024 px output and a long focused title, when the bar is rendered, then the title is elided from its middle and modules are dropped in the specified priority order until the row fits, while the six runes, the mode badge, the battery and the clock are all still drawn | |
| A13 | Given `grimoire` true and the default keymap, when the overlay is rendered, then it lists one row per binding with a non-empty `hint_key`, grouped by mode in `Keymap::bindings` order, and no row for a binding whose `hint_key` is empty | |
| A14 | Given identical `(HelmState, Palette, Probe)` and canvas dimensions, when `render` runs twice into two canvases, then the two pixmaps are byte-identical | |
| A15 | Given an output at integer scale 2, when the bar is rendered, then the buffer is allocated at twice the logical size and every metric from `palette.toml` appears at exactly twice its logical value, with no fractional geometry | |
| A16 | Given a window taking the whole output as fullscreen, when it is mapped, then both strips stay mapped on the `Bottom` layer with unchanged exclusive zones and the window covers them, and leaving fullscreen produces no relayout | |

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md); no new numbers are invented here.

| Path | Budget | How it is held |
|---|---|---|
| State change → bar redraw | **< 8 ms** | `renders_same_as` gates before any drawing; damage-tracked CPU rasterising at 1920×32; chrome shaping cached |
| Bar idle CPU | **~0 %** | the bar has no timers at all; every module, the clock included, is pushed by the session, which drops no-op states before broadcasting. With minute resolution the clock commits about one frame per minute |
| Cold session start → usable | **< 900 ms** (whole session) | no GPU context (ADR 0008); the font probe is 37 database lookups, once; no icon cache, no thumbnailer |

Measured in release on the reference runner, against the same `HelmState`
fixtures the unit tests use. ADR 0008's guards apply: a cold-start benchmark, a
redraw benchmark, a sixty-second idle frame count, and a link-time assertion
that the binary links no EGL, GL or Vulkan.

`render` performs no I/O, no font-database queries and no allocation beyond the
shadow pixmap it was handed.

## Failure modes

From [PITFALLS.md](../PITFALLS.md), this component owns:

- **Redraw on a timer.** One timer, the clock, gated by `renders_same_as`.
- **Missing Nerd Font** and **exotic glyph assumed present.** Every chrome
  glyph through `Probe::resolve`; documented ASCII fallbacks; the probe runs
  before the first frame.
- **Emoji font hijacks symbols.** The chain from `palette.toml` is explicit and
  ordered; fontconfig's default is never consulted.
- **Fractional scaling blur.** Integer geometry, buffers at the output's real
  scale; fractional outputs render at the next integer scale up and say so.
- **Session dies with a client.** The bar holds no state beyond its surfaces
  and its probe, so a crash loses nothing and a restart redraws from the next
  snapshot.

It contributes to, but does not own: **layer-shell not served** (helm-session
must serve `river-layer-shell-v1`, and `doctor` checks it), and **focus causes
relayout** (the bar's exclusive zone is constant at every width, so nothing it
does moves a tile).

## Open questions

- **`Damage` is one rectangle.** A change touching both a rune and a module
  unions to nearly the whole bar. Widening `Damage` to a small `Vec<Rect>`
  would help that case and complicate every other.
  *Recommendation: leave [INTERFACES.md §3](../INTERFACES.md) as it stands
  until a measurement shows the union costing a frame.*
- **Whether the starfield deserves a palette token.** Drawing it from
  `text.bright` keeps every colour in `palette.toml`, but the prototype's
  `#d2e1ff` is a slightly cooler white.
  *Recommendation: `text.bright` for M2; add `background.starfield` only if it
  reads wrong on a real panel.*
- **The rune glow.** The specified 1 px dilation is a rasteriser-shaped
  substitute for an 8 px CSS blur, and nobody has looked at it on hardware.
  *Recommendation: implement it, then compare against the prototype at 1920
  before deciding whether it needs more.*
- **The keymap after M4.** `GetKeymap` is fetched once at startup because the
  keymap is not configurable in M2. When it becomes configurable, the bar needs
  telling. *Recommendation: an `Event::Keymap` at that point, not a field in
  `HelmState`.*
- **Multi-output.** The bar assumes one output, which is the tested path
  ([MVP.md](../MVP.md): multi-monitor beyond "it does not break" is M6). One
  bar per output, each with its own exclusive zone, is the obvious shape and is
  not specified here.
