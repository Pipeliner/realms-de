# helm skills

Project-local Claude Code skills for building a Wayland desktop environment.
Loaded automatically when a task matches a skill's `description`; `CLAUDE.md`
points here for "write DE code specifically".

## Why these exist

Standing order S4 says to look for an applicable skill before starting a class
of work: the enabled skills, the org plugin catalogue, and the public skill
directories. We looked. Nothing covers compositors, Wayland session
integration, tiling geometry, palette-driven theming or DE frame budgets — the
public catalogues are dense with document conversion, web scaffolding and API
wrappers, and thin to empty on desktop internals.

S4 says that absence is a signal to *write* the skill. These seven are the
result. They encode the parts of helm that are expensive to rediscover: the
invariants that fail silently, the ordering contract that breaks portals, and
the design rules the mockups state once and everything downstream depends on.

## The ten

| Skill | Triggers when | What it prevents |
|---|---|---|
| [`helm-spec-first`](helm-spec-first/) | About to implement anything non-trivial; writing a spec or ADR; "should I write a test first" | Code landing ahead of a written contract; a spec that has silently diverged from its implementation; edge-case test matrices where a happy path was asked for |
| [`helm-layout`](helm-layout/) | Editing `layout.rs` or `ledger.rs`; adding a layout; touching `project()` or `partition()` | Hairline cracks between tiles, overlapping or escaped rectangles, windows that twitch when focus moves, a second source of truth for geometry |
| [`helm-theming`](helm-theming/) | Editing `palette.toml`; adding a template; theming an app; any question about a colour | A colour written down twice and drifting; a contrast *filter* returning and rotating accent hues; a half-applied, two-toned desktop |
| [`wayland-session-integration`](wayland-session-integration/) | Session startup, systemd user units, portals, D-Bus activation, cursor, XWayland; file dialogs that hang; screen share that silently fails | The classic minimal-Wayland failure: `WAYLAND_DISPLAY` and `XDG_CURRENT_DESKTOP` never reaching the systemd and D-Bus activation environments |
| [`helm-ui-fidelity`](helm-ui-fidelity/) | Building the bar, which-key strip, window headers, launcher or charon; reading the `.dc.html` prototypes | Drifting from the mockups; hardcoded sizes and colours; a rounded corner or an animation sneaking in; tofu glyphs on a machine without Nerd Fonts |
| [`helm-perf-budget`](helm-perf-budget/) | Adding a timer, poll loop, animation or redraw path; anything hot; "is this fast enough" | Idle CPU that never reaches zero; redundant redraws; banned effects arriving as "just a small one"; performance claimed rather than measured |
| [`helm-issue-flow`](helm-issue-flow/) | Picking up work, opening or closing an issue, running the agentic loop | Work that is not an issue; guessing at an ambiguous contract; a silent guess where a `needs-human` label belonged; a red build pushed |
| [`helm-agent-sdd-bootstrap`](helm-agent-sdd-bootstrap/) | Beginning or recovering a local agent-SDD pilot investigation for one GitHub issue | A stale checkpoint treated as current truth; pilot records created without the normal issue/spec-first flow |
| [`helm-agent-sdd-checkpoint`](helm-agent-sdd-checkpoint/) | Capturing a fresh issue-numbered pilot handoff or assessing a supported report-only edge | A record combined with ordinary work, stale Git provenance, or a dry-run assessment treated as promotion |
| [`helm-agent-sdd-evidence-capture`](helm-agent-sdd-evidence-capture/) | Recording one fresh local pilot command, file observation or decision | Tool output, secrets, source snapshots or stale observations entering durable records |

## Adding an eighth

1. **Check first that it does not exist** — here, in the enabled skills, in the
   plugin catalogue, and online (S4). Writing a skill is the fallback, not the
   default.
2. `mkdir .claude/skills/<kebab-case-name>/` and write `SKILL.md` with YAML
   frontmatter carrying exactly `name` and `description`.
3. **Write the description as trigger conditions, not a summary.** It is the
   only thing loaded until the skill fires, so it decides whether the skill
   fires at all. Start with when to use it, stay in the third person, and name
   the concrete nouns someone would actually type — file paths, type names,
   commands, and the symptoms they would describe in their own words. Compare:
   - poor: "Helps with helm's theming system."
   - good: "Use when anything about helm's colour or generated theme files is in
     play — editing `palette.toml`, adding a template, theming a new app…"
4. **Apply progressive disclosure — this is a hard requirement** (S5, the same
   rule that keeps `CLAUDE.md` at forty lines). `SKILL.md` holds the decision
   procedure, the rules and the checklist, and stays short enough to scan:
   roughly 120 lines. Everything long — reference tables, full API listings,
   worked examples, symptom catalogues — goes in a sibling `reference.md` or
   `examples.md` that `SKILL.md` links to **and says when to open**. A 400-line
   `SKILL.md` has failed the assignment: it is loaded in full every time it
   triggers, whether or not the reader needs the tables.
5. **Verify every citation.** Grep for every file path, test name, function and
   type before you commit them. A skill citing a test that does not exist is
   worse than no skill, because it will be trusted.
6. **Say plainly when something is not built yet**, and name the milestone
   rather than writing instructions against imaginary code. Several of these
   skills describe M1–M3 work that does not exist; each says so.
7. House style applies here as everywhere: prose explains *why*, not *what*;
   British-inflected plain English; no emoji.
8. Add the row to the table above.
