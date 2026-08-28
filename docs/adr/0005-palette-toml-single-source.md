# ADR 0005 — One `palette.toml`, and everything themed is generated from it

- **Status:** Accepted (ratified 2026-08-28); see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

helm is themed across GTK3, GTK4/libadwaita, Qt5/6, a terminal's 16-colour ANSI
table, yazi, btop, starship, fuzzel, the bar, the launcher and window borders.
That is eleven places a hex value could be written down, and the handoff's
palette has around forty distinct values in it.

`docs/PITFALLS.md` records the two failures this produces. "Colour written down
twice: the two drift." And "half-applied theme: terminal is new, GTK is old,
until relogin." Both are ordinary and both are avoidable.

The handoff also fixes the mechanism: `~/.config/helm/palette.toml` → templates
→ `helm ctl theme apply` hot-reload. And it fixes the budget: theme apply under
150 ms (`docs/ARCHITECTURE.md` §4), which rules out anything that recompiles or
regenerates an icon cache.

## Decision

1. `palette.toml` is the only file in the repository or on a user's disk that
   may contain a colour literal. `helm-core::palette::Palette` is its schema:
   backgrounds, a six-step text ramp, six accents, borders with alphas carried
   separately, a `[pantheon]` tool-to-accent map, typography, metrics, glyphs.
2. Every themed file is a template in `configs/templates/`, rendered by
   `helm-theme` from the *derived* palette — that is, from
   `Palette::derived()`, with the contrast setting already folded in (ADR 0006).
3. `helm ctl theme apply` renders the small shipped template set serially to
   temporary files, then `rename(2)`s each into place, and only then performs a
   single reload fan-out: `gsettings` writes for GTK and `SIGUSR1` to clients
   that support it. A helm-client reload notification remains planned session
   integration: its IPC event/ordering contract must be accepted and tested
   before a new `Event` variant is named. Renames are atomic per file only:
   there is no cross-file all-or-nothing publication. Rendering/staging failure
   before the first rename leaves outputs untouched; an interruption during
   publication can expose a mixed generation until the next successful apply.
4. `Palette::lint` runs before anything is written. A fatal finding aborts the
   apply. Nothing is rendered from a palette that fails its own readability
   floors.
5. Alphas are stored beside their colours, so templates for formats without an
   alpha channel can `Rgb::flatten_over` the known background rather than
   guessing.

## What theming reaches, and what it does not

From the handoff, section 1c. This table is a contract with the user as much as
with the implementation: the limits are documented, not fought.

| Surface | Mechanism | Reaches |
|---|---|---|
| GTK3 and GTK4 | generated `gtk.css` | Colours, square corners, flat headerbars, IBM Plex Mono |
| libadwaita | named colours (`@window_bg_color` and friends) | Colours only |
| Qt5 and Qt6 | `qt6ct` colour scheme plus a Kvantum SVG rendered from the same palette | Colours, and widget shapes via Kvantum |
| Terminals | 16-colour ANSI scheme | Full |
| Electron | force-dark plus the ANSI/GTK values it picks up | Approximate; enough to stop one app being bright white |
| Icons and cursor | mono-line icon set and cursor theme | Full, within the set's coverage |
| TUIs (yazi, btop, starship, fuzzel) | generated theme files | Full |

| Limit | Why | What we do instead |
|---|---|---|
| Hardcoded application colours | The application ignores every theming mechanism | Document it. Do not fight it |
| libadwaita geometry | Adwaita exposes named colours but not shapes; corner radii and paddings are compiled in | Take the colours, accept the shapes |
| CSD headerbars | Client-side decorations are drawn by the application | Recolour plus a 1px border. No shape changes |
| Flatpak applications | Sandboxed applications cannot read `~/.config` or `~/.themes` by default | Document the per-app `--filesystem` grant in the install docs, and have `helm ctl doctor` report Flatpak apps without it |
| The lock screen *(provisional)* | Depends on which locker ships; still `needs-human` in [ADR 0011](0011-session-integration-contract.md) | If `waylock` is chosen, theming reaches four `0xRRGGBB` values and nothing else: no type, no clock, no layout. That is a deliberate trade of fidelity for attack surface on the one security-critical surface, and it belongs on this list rather than being quietly absorbed |

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Hand-maintained theme files, one per target** | No template engine, no build step; each file can use its format's full expressiveness; easiest to hand to a contributor who knows GTK CSS but not helm | Forty values times eleven files is the drift problem stated as a number. It also makes "change the accent" a multi-file patch, which is exactly the operation the handoff wants to be one line |
| **A runtime theming daemon that pushes colours over IPC** | No files on disk to get stale; instant application; one authoritative in-memory palette | Only works for clients that speak helm's protocol. GTK, Qt, yazi and btop all read files, so we would need the file pipeline anyway *and* a daemon. Strictly more machinery for the same result |
| **Adopt an existing scheme system** (base16, pywal, stylix) | Large ecosystem of ready-made templates; users already understand it; stylix in particular is idiomatic on our reference platform | base16's sixteen slots cannot express helm's six-step text ramp plus six accents plus per-border alphas without lossy mapping, and the handoff's palette is not negotiable at that fidelity. Stylix would also invert control: the palette would live in a NixOS module rather than in a file that works on Ubuntu and Fedora too. We can *export* a base16 mapping from `palette.toml` later; we cannot import from it without losing values |
| **Write templates against the raw palette, apply contrast at render time in each template** | One less derivation step | Every template would reimplement OKLab contrast, in whatever language the template engine offers. `Palette::derived()` does it once, correctly, in Rust |

## Consequences

### Good

- One line changes the desktop. `palette.toml` is the whole configuration
  surface for colour.
- Drift is structurally impossible for anything inside the pipeline.
- Contrast, hue separation and WCAG floors are checked before a single byte is
  written, by code that already exists and is tested.
- The pantheon map means a pane's accent is data, not a literal in the bar's
  drawing code.
- Per-file atomic replacement prevents torn files. It does not promise a
  cross-file transaction; the future helm-client notification must occur only
  after the requested replacements complete.

### Bad

- A template engine and a set of templates is real code to maintain, and every
  new themed target adds one.
- Adding a colour means editing the `Palette` struct, `palette.toml` and at
  least one template. That friction is intentional but it is friction.
- Users who want to hand-edit a generated file will have their edits overwritten
  on the next apply. We must say so in a header comment in every generated file.
- Some targets (Kvantum's SVG, Electron) are approximations. The reach table is
  honest about this but users will still find the seams.

### Neutral

- The palette carries typography, metrics and glyphs as well as colour. It is
  really "the design tokens file"; the name is historical and we are keeping it.

## Reversal

Low per-target, medium overall. Dropping a single target means deleting a
template. Abandoning the pipeline entirely means hand-writing eleven theme files
and accepting the drift, which is a day's work and a permanent tax.

The signal to reconsider is a target whose format genuinely cannot be generated
— something requiring a compiled binary theme, say — at which point that one
target gets an exception and the rule survives.

## Guard

- `palette::tests::shipped_palette_parses_and_round_trips`.
- `palette::tests::shipped_palette_passes_its_own_lint` — the shipped palette
  must satisfy the floors it enforces on users.
- `palette::tests::shipped_palette_survives_the_whole_contrast_range` — lint
  holds at every contrast setting in `[0.85, 1.40]`, not just the default 1.08,
  for all six accents.
- `palette::tests::lint_catches_muddy_text_and_duplicate_accents` — the lint
  itself is tested against a palette that should fail.
- `palette::tests::out_of_range_values_are_rejected_at_parse_time` — including
  `metrics.radius != 0`, which is how "helm has no rounded corners" is enforced.
- `palette::tests::derived_palette_is_idempotent`.
- *Planned (M1):* a CI grep asserting that no file outside `palette.toml`,
  `design/` and `docs/` contains a `#rrggbb` literal. This is the guard for the
  rule itself and is the one most likely to erode.
- *Planned (M1):* apply tests that distinguish per-file atomicity from a
  cross-file transaction. The helm-client notification is deferred until its
  IPC contract is accepted and has test-first implementation evidence.
