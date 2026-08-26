# ADR 0007 — Reuse yazi, btop and zsh+starship rather than rewrite them

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

The handoff names five tools by their pantheon names: charon (files), horus
(monitor), thoth (shell), hecate (launcher), odin (agent harness). Four of the
five have a mature Rust or near-Rust equivalent that already does most of what
the design asks for. One does not.

Read the charon specification in the handoff carefully and it describes yazi:
miller columns with a 240px parent / flex current / 460px preview split,
toggleable text-only preview, marks, flags, git status suffixes, a `:` shell
line, sort by mtime, hidden-file toggle. Every one of those is a yazi feature
today. What is missing is the colour scheme and the keymap.

The same is true of horus and btop, and of thoth and zsh+starship. The prompt
the handoff specifies, `nav@caldera :: 𓂃%`, is a starship format string.

odin is the exception. The handoff says so: "custom `ratatui` TUI wrapping your
agent runner. Nothing off-the-shelf matches."

Standing order S8 states the general rule. This ADR applies it and records the
specific assignments.

## Decision

| helm name | Implementation | What we actually build | Milestone |
|---|---|---|---|
| charon (files) | **yazi** | `configs/yazi/`: generated theme, helm keymap, preview and thumbnail settings | M1 |
| horus (monitor) | **btop** | `configs/btop/`: generated theme | M1 |
| thoth (shell) | **zsh + starship** | `configs/zsh/`, `configs/starship/`: generated prompt and colours | M1 |
| hecate (launcher) | **fuzzel**, as a stopgap | `configs/fuzzel/`: generated theme. Retired by `helm-hecate` | M1, retired M4 |
| odin (agent harness) | **`helm-odin`, written from scratch** | A `ratatui` TUI. Nothing existing matches | M4 |
| charon portal | **`xdg-desktop-portal-termfilechooser`** pointed at yazi | `configs/portal/`. Stopgap; native dialog at M4 | M3, retired M4 |

The rule that makes this safe: **always behind a seam.** Every reused tool is
reached through a generated config file (ADR 0005) or a documented invocation in
the keymap, never through a hardcoded call site. Retiring a stopgap deletes a
template and changes one line in the keymap.

fuzzel is a stopgap and is labelled as one in `docs/MVP.md`. yazi, btop and
starship are not stopgaps. We currently expect them to be permanent.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Write `ratatui` equivalents now** | Total control over every pixel, so the design is matched exactly rather than approximated; one language, one build, one theme mechanism; no upstream to track | A file manager that is genuinely trustworthy with a user's files is a multi-year project, and the first two years of it are worse than yazi. The same is true of btop's sensor coverage, which spans a decade of hardware quirks we have no way to test against. This would consume the entire M0–M3 critical path and produce a worse desktop |
| **Fork them and vendor the forks** | Exact control while keeping the existing feature set; no upstream to negotiate with; we could add a helm-native theming hook | Permanent maintenance. Every upstream fix has to be merged by us forever, and the divergence only grows. We would be taking on the cost of a rewrite without getting the benefit of owning a clean design |
| **Reuse, but wrap each tool in a helm process that mediates it** | A uniform helm-shaped interface over heterogeneous tools; could unify theming and keybinding | A layer of indirection that buys nothing. The tools already read config files; generating those files *is* the integration. A wrapper would add latency and a second thing to debug |
| **Reuse different tools** (nnn or lf for files, htop or bottom for the monitor) | Some are lighter; `bottom` is Rust and themable | yazi's miller-column model is what the design specifies, and lf/nnn would need the columns building. `bottom` is a reasonable alternative to btop and worth revisiting, but btop's braille sparklines and meter style match the handoff's `▰▱` and `⡀⣤⣶` vocabulary more closely |

## Consequences

### Good

- charon and horus are roughly 90% theme and keymap rather than 90% new code,
  which is what makes M1 a milestone rather than a year.
- Users get features we would never have got round to: yazi's archive preview,
  btop's per-core and GPU coverage, starship's language detection.
- Each tool is independently useful and independently debuggable. A user can run
  `yazi` outside helm and it still works.
- Bug reports about file operations go upstream, where people who understand
  file operations read them.
- The reuse is visible in the repo layout (`configs/yazi/`, `configs/btop/`),
  so nobody is misled about what helm wrote.

### Bad

- Fidelity to the design is bounded by what each tool's theme system exposes.
  The handoff acknowledges this ("TUI/eww approximations are expected to
  quantize to the cell grid"), but the charon header, the footer format and the
  exact column widths will not match `Files & Theming.dc.html` pixel for pixel.
- Each tool has its own config format, its own theme schema and its own release
  cadence. An upstream schema change breaks a template.
- Runtime dependencies that packaging must declare on three distros, at versions
  new enough to have the theme keys we use.
- Keybinding consistency is work. yazi and btop have their own defaults and
  their own ideas about modifiers; making them feel like one desktop is a
  per-tool effort rather than a shared mechanism.
- Users see tool names in process lists and error messages that do not match the
  pantheon names in the UI, which is mildly confusing.

### Neutral

- Two of the six entries are explicit stopgaps with a scheduled retirement. The
  seam makes that a config deletion rather than a refactor.
- yazi, btop and starship are all actively maintained with large user bases. If
  that changes for any of them, the seam is what makes the response cheap.

## Reversal

Low per tool. Replacing a reused tool means writing the replacement — which is
the expensive part and is unchanged by this decision — and then deleting a
template and editing a keymap entry, which is an afternoon.

We have already scheduled two reversals: fuzzel to `helm-hecate` at M4, and the
termfilechooser portal to a native charon dialog at M4. Doing them will prove
the seam works, or reveal that it does not, at a point where the cost of fixing
it is still low.

The signal to reconsider a *permanent* reuse would be an upstream that stops
being maintained, or a theming limitation severe enough that helm looks like a
pile of programs rather than a desktop — which is the exact phrase `docs/MVP.md`
uses for what the theme pipeline is meant to prevent.

## Guard

- *Planned (M1):* a template render test per tool that renders the config from
  `palette.toml` and parses it with the tool's own validator, so an upstream
  schema change fails CI rather than a user's session.
- *Planned (M1):* a version-floor test asserting that the installed yazi, btop
  and starship are at least the versions whose theme keys our templates use.
- *Planned (M3):* `helm ctl doctor` reports each reused tool's presence and
  version, so a missing dependency is a diagnosis rather than a mystery.
- *Planned (M2):* the seam guard — a CI grep asserting that the strings `yazi`,
  `btop`, `fuzzel` and `starship` appear only in `configs/`, `packaging/`,
  `docs/` and the keymap's default table, never in drawing or logic code.
