# Design handoff

The source material helm is built from. **None of this is production code, and
none of it is a screenshot of running software.**

| File | What it is |
|---|---|
| `HANDOFF.md` | The brief: target stack, screens, interactions, design tokens. Human-authored. |
| `Desktop v3.dc.html` | **Canonical.** Hi-fi HTML prototype of the desktop — bar, tiles, which-key, hecate launcher, all five panes. |
| `Files & Theming.dc.html` | charon file manager, charon portal open dialog, and the GTK theming reference with its reach-and-limits notes. |
| `Desktop v2.dc.html`, `Desktop.dc.html` | Earlier iterations, kept for provenance. v3 supersedes both; v1 carries a deprecated cyberpunk direction that was abandoned. |

## How to read the prototypes

They are **design references drawn in HTML**, not an implementation to port.
All styling is inline on each element, so opening one in a browser and
inspecting is the fastest way to get an exact value. They reference a
`support.js` that is not in this repository, so they will not behave exactly as
authored — the styling, which is what matters, is unaffected.

Where a prototype and `palette.toml` disagree, **`palette.toml` wins**: it is the
single source of truth for colour, and the prototypes predate it.

Where a prototype and an ADR disagree, the ADR wins and should say why. The
clearest case is contrast: the prototype implements it as a
`backdrop-filter: contrast()`, and helm derives it per-colour in OKLab instead
(see [ADR 0006](../docs/adr/0006-oklab-contrast-not-filters.md)).
