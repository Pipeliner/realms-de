# ADR 0012 — Font fallback is a contract, not a hope

- **Status:** Accepted (ratified 2026-08-28); the missing guard remains tracked
  implementation work
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

helm draws a lot of characters that IBM Plex Mono does not contain. Counted from
`helm-core::glyphs::inventory`, there are 37 of them across six surfaces:

- Six elder futhark runes, `ᚠᚢᚦᚨᚱᚲ`, one per orbit, in the bar.
- Pantheon sigils in window headers: `ᛟ ☽ ⚚ ◉ ✶`, plus `⚷` for hecate and `☍`
  for charon.
- Bar and which-key furniture: `✦ ⌗ ⌨ ▸ ☾ ⚡ ♪ ⊞ ↵ ⇥`.
- Window controls `◈ ─ ✕`, meters `▰ ▱`, and an eight-step braille sparkline
  ramp `⡀⣀⣄⣤⣦⣶⣷⣿`.
- One Egyptian hieroglyph, `𓂃`, in the shell prompt. The handoff singles it out:
  "verify `𓂃` (Egyptian) renders or substitute `~`."

The handoff's design tokens say the fallback chain "must include a Nerd
Font/Symbola for runes". `palette.toml` lists six families in order, ending at
DejaVu Sans Mono.

The failure mode is well known and looks terrible. On a machine without the
right fonts the bar draws six tofu boxes where the orbits should be, before the
user has typed anything. `docs/PITFALLS.md` lists it twice: "missing Nerd Font"
and "exotic glyph assumed present". A desktop that looks broken on first boot
does not get a second boot.

The tempting non-solution is to declare a font dependency in the packaging and
move on. That fails in the cases that matter most: a container, a live ISO, a
minimal server install someone is trying helm on, a user who installed from a
tarball, or a machine where an emoji font sits earlier in fontconfig's ordering
and hijacks the symbol ranges into colour glyphs at the wrong size.

## Decision

The glyph inventory is data, verified at startup, with a documented ASCII
fallback for every entry.

1. **`helm-core::glyphs::inventory()` is the complete list.** Every glyph helm
   draws appears there with its name, the surface it appears on, and its
   fallback. No drawing code may contain a glyph literal that is not in the
   inventory.
2. **Every glyph has an ASCII fallback.** Not "most". `Glyph::fallback` is
   `Option<char>` so that a future essential glyph *could* be declared fatal,
   but today every entry is `Some`, every fallback is ASCII, and the test
   enforces both. `𓂃` falls back to `~`, exactly as the handoff asks. The runes
   fall back to the digits `1`–`6`, which is the only substitution that still
   reads as a workspace indicator.
3. **A `Probe` runs at startup, before the first frame.**
   `Probe::run(|c| ...)` takes a coverage predicate — in practice a query
   against the resolved `cosmic-text` font database (ADR 0008) built from
   `palette.toml`'s `typography.fallback` list. It returns covered and missing
   sets.
4. **`Probe::resolve(ch)` is the only way to get a character to draw.** It
   returns the glyph if covered and the documented fallback if not. Drawing code
   never touches a raw literal.
5. **The fallback chain is explicit and ordered**, from `palette.toml`, never
   fontconfig's system default. This is what stops an emoji font hijacking the
   symbol ranges.
6. **`helm ctl doctor` runs the same probe** and reports it with
   `Probe::summary()`, so a user with a missing font gets a sentence naming the
   glyphs rather than a screen of boxes.
7. **Helm does not redistribute Symbola or a generic Nerd Font.** Their
   provenance is not one unambiguous, project-owned licensing decision. Helm
   packages and release artifacts contain no font bytes from either family.
   Target packaging may only recommend distribution-reviewed symbol-font or
   Nerd Font packages; it must not make that optional visual enhancement a hard
   runtime dependency. A recommendation never substitutes for the startup probe
   and ASCII fallback.
8. **Helm ships only unmodified IBM Plex Mono under SIL OFL 1.1.** Any
   first-party artifact that contains it must carry the upstream OFL licence
   and required notices. The Nix reference module declares the reviewed
   `nixpkgs` IBM Plex package as this approved delivery mechanism. Debian and
   Fedora packages do not embed it and must only recommend their reviewed IBM
   Plex package instead. This does not authorize embedding or patching another
   font: that needs an explicit licensing ADR amendment covering source,
   licence, notices, update and removal policy.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Declare a font package dependency and assume it** | Simplest possible; correct on any properly installed system; every distro packages Nerd Fonts or Symbola | Assumes helm is always installed by a package manager we control, which is false for containers, live images, tarball installs, and NixOS home-manager setups where fonts are configured separately. It also does nothing about fontconfig ordering, where an emoji font earlier in the chain claims the symbol ranges and renders them in colour at the wrong size. Worst of all it fails silently: the dependency being satisfied does not prove the glyph renders |
| **Bundle and embed the fonts in the binary** | Guaranteed coverage; no external dependency; identical output everywhere | Licensing must be checked per family and some are not redistributable in a binary. Full Unicode symbol coverage is tens of megabytes. It also fights the user's own font configuration, which a keyboard-first desktop's users will reasonably want to control |
| **Render at runtime and detect tofu by inspecting the glyph mask** | Catches the failure at the exact moment it would be visible; no inventory to maintain | Detecting a `.notdef` box by looking at rasterised pixels is a heuristic and a bad one. It is also per-frame work against an 8 ms budget, versus one probe at startup |
| **Substitute nothing; draw the glyph and accept tofu** | Zero code; the user sees exactly what their fonts provide and knows to install one | This is the status quo of every glyph-heavy TUI, and it is the pitfall we are writing the ADR to avoid. It also gives the user no idea *which* font to install |
| **A fallback string per glyph rather than a single char** | Richer substitutions: `[1]` for a rune, `cpu` for a sigil | Every helm surface is on a fixed grid — a 32px bar, 26px headers, cell-quantised TUIs. A multi-character substitution changes the width of everything after it. One character in, one character out is what keeps the layout stable |

## Consequences

### Good

- helm never draws tofu. On a machine with nothing but a bare ASCII font it
  degrades to a plain, ugly, entirely legible bar.
- The failure is diagnosable: `doctor` names the missing glyphs and the
  configured chain, so the user knows what to install.
- The inventory doubles as documentation of what helm draws and where, which is
  useful to anyone writing a template or a new client.
- Because it is data, adding a glyph forces a fallback to be chosen. The test
  fails otherwise.
- Fixed-width substitution keeps every layout stable in the degraded case, so
  the bar does not reflow on a machine with different fonts.

### Bad

- Every glyph must be added to the inventory before it can be drawn, which is
  friction and will occasionally be forgotten. Only a lint can catch that, and
  we do not have one yet.
- The ASCII fallbacks are genuinely worse. `1` for `ᚠ` loses the design entirely,
  and a user who never installs a font never sees helm as designed.
- The probe is only as good as the renderer's coverage predicate. If
  `cosmic-text` reports a face as covering a codepoint but renders a blank, the
  probe passes and the user still sees nothing.
- The runes are declared non-essential, so helm starts and looks wrong rather
  than refusing to start. That is the right default but it is a judgement call.
- One startup cost: 37 codepoint lookups against the font database. Negligible,
  but it is on the cold-start path.

### Neutral

- `Glyph::fallback` being `Option` leaves room for a genuinely fatal glyph
  later. Nothing uses it today and the test asserts nothing does.

## Reversal

Low. The inventory and the probe live in `helm-core::glyphs`, and the resolve
call sites live in the clients. Abandoning the mechanism means deleting the
module and drawing literals directly, which is an afternoon and a regression.

Extending it is the more likely change: adding a fallback *chain* per glyph
rather than a single character, or letting the user override a fallback in
`palette.toml`. Both are additive.

The signal to reconsider is a report that the probe passed and the user still
saw a box, which would mean the coverage predicate is wrong and the mechanism
needs to move from font-database queries to actual rasterisation.

## Guard

- `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu` — the
  named guard. Runs the probe with `|c| c.is_ascii()` and asserts that every
  glyph in the inventory resolves to an ASCII character, that `𓂃` resolves to
  `~` and `ᚠ` to `1`, and that the summary says it is substituting.
- `glyphs::tests::every_glyph_is_unique_and_has_a_fallback` — no duplicates,
  every fallback present, every fallback ASCII.
- `glyphs::tests::a_complete_font_substitutes_nothing` — the other direction:
  with full coverage nothing is substituted.
- `palette::tests::empty_typography_fallback_is_rejected` — asserts the
  validation guard that rejects an empty `typography.fallback`, so the chain
  can never be absent. **Acceptance requirement:** this guard must exist,
  fail against a fixture whose only defect is `typography.fallback = []`, and
  pass for the shipped palette before ADR 0012 may move from Proposed to
  Accepted. The current parse-range test does not prove the empty-list case.
- *Planned (M2):* a render test that rasterises the whole inventory through the
  real `cosmic-text` stack against the shipped font list and asserts no
  `.notdef` mask is produced. This closes the gap between "the database says it
  is covered" and "it actually draws".
- *Planned (M2):* a CI lint asserting that no non-ASCII character literal
  appears in client drawing code outside the inventory.
