# SPEC 0002 — Theme pipeline

- **Status:** Accepted; renderer, lint, palette initialization, and the retired
  mutable writer were implemented through A14 (2026-08-27). As reconciled by
  #159, [SPEC 0011](0011-theme-activation-generations.md) supersedes mutable
  target publication, equality/no-op reporting, reload fan-out, and mutable
  output diff for the supported apply/diff path.
- **Milestone:** M1
- **Decisions:** [ADR 0005](../adr/0005-palette-toml-single-source.md),
  [ADR 0006](../adr/0006-oklab-contrast-not-filters.md),
  [ADR 0017](../adr/0017-immutable-theme-activation-generations.md)
- **Implements:** [INTERFACES.md §2](../INTERFACES.md)

> Written before the code, as S14 requires. Existing `theme::tests` mappings
> prove the preserved palette/rendering/lint behavior or record the retired
> mutable implementation; generation publication and generation-aware diff are
> governed by SPEC 0011 and need their own acceptance implementation.

## Purpose

One palette file must render a coherent theme for GTK apps, Qt apps, the
terminal, yazi, btop, the shell prompt and Realm's own clients, then publish that
complete result for future launches as one immutable generation.

This is the difference between a desktop environment and a collection of
programs that happen to be running at the same time. It is also the component
most likely to produce a visibly broken desktop if it gets it half right, which
is why generation atomicity is a hard requirement rather than a nicety.

## Scope

**In:** reading `palette.toml`; deriving the contrast variant; rendering every
template; linting; publishing an immutable generation for future launches; and
the generation-aware `realm ctl theme apply|lint|diff` surface.

**Out:** the palette *format* and its validation (SPEC 0001, `realm-core`);
deciding which colour goes where (that is `palette.toml` itself); anything that
draws (`realm-bar`); live upgrade of an existing process; no-op apply
optimization; and compatibility with the retired mutable apply result or a
control-socket wire message.

## Behaviour

### Inputs

`~/.config/realm/palette.toml`, falling back to the shipped `palette.toml`.
Parsed and validated by `realm_core::Palette`. A palette with any **fatal** lint
finding is refused: no generation is published, `current` stays unchanged, and
the findings are printed. Non-fatal findings are printed and may be published.

### Derivation

Templates address the **derived** palette (`Palette::derived()`), so `contrast`
is folded in exactly once, at this boundary. No template may apply contrast
itself, and nothing downstream may use a display-side contrast filter — see
ADR 0006 for why that costs a fullscreen pass per frame and rotates hues.

### Rendering

Placeholder vocabulary is defined in [INTERFACES.md §2](../INTERFACES.md).
An unknown placeholder is a **hard error** that aborts the whole apply. A
silently blank colour is the exact bug this design exists to prevent.

The shipped Starship output SHALL render a dynamic `user@host ::` prefix and an
ASCII-safe `~% ` character by default, including the trailing space that
separates the prompt from the command. User, host, separator, sigil, percent
marker, Git state, and duration styles SHALL come only from derived palette
keys. Theme generation has no runtime font-probe result, so it SHALL use
`glyphs.prompt_sigil_fallback`; it SHALL NOT guess that the configured font can
draw `glyphs.prompt_sigil`. The exotic prompt form is post-MVP opt-in work.

Implementing it added two forms, both supersets of that table rather than
changes to it. An alpha argument may be a path as well as a literal —
`{{ border.seam.rgba(border.seam_alpha) }}` — so the alphas stay written down
once, beside the colours they belong to; and because `.over(...)` yields a
colour it may be followed by `.bare` or `.rgba(...)`, which is what lets an
alpha-less bare-hex format like fuzzel's express a translucent seam.

### Generation application

The supported path renders every template from one captured input snapshot and
publishes the complete output set through SPEC 0011. It never compares or
writes the retired mutable targets and never invokes `Reloader`,
`SystemReloader`, a process signal/command, or session notification. Its public
result is `GenerationPublicationOutcome`, not
`Applied { written, unchanged, reloaded }`.

A successful `current` commit activates the sealed generation only for future
launches. Existing processes remain pinned to the generation they selected and
are not reloaded by apply or rollback. Applying byte-identical inputs may still
publish a fresh generation; equality short-circuiting and a no-op outcome are
not part of the supported contract.

### Output containment

Template targets become normalized relative output paths inside one staged
generation. Empty, absolute, traversal, duplicate, symlinked, or prefix-
colliding outputs are refused before publication. All generated-tree access is
descriptor-relative and no-follow as specified by SPEC 0011.

Palette lookup and first-run initialization retain the established lexical root
handling: a symlinked configuration root must be refused under trailing-
separator and terminal-`.` spellings, and the root's `realm` directory and
`palette.toml` must be real children rather than symlinks before a copy or read
occurs. The former direct-target staging, rename, cleanup, comparison, and
reload rules are retained only as historical implementation provenance; they
are not an alternative supported apply path.

### Reload metadata and future live upgrade

Template reload metadata remains part of the canonical catalogue digest so a
future protocol can bind its decision to the exact generation inputs. It is
descriptive metadata only on the supported apply path. A pointer switch never
runs it.

Any live upgrade belongs to #22 and must be a future generation-aware
owned-process protocol which proves the process's selected generation. It
cannot restore direct reload on pointer switch, cannot target foreign/direct
launches, and is not a commitment to preserve the current wire protocol.

### First-run

If no user palette exists, `apply` copies the shipped one to
`~/.config/realm/palette.toml` first, so a user's first edit is to their own file.

## Acceptance criteria

Rows A1-A10 are the current theme-pipeline criteria. Their generation storage,
publication, and diff details are acceptance criteria G1-G11 in SPEC 0011.
Rows A11-A14 retain palette/input-containment guarantees; direct mutable-writer
tests with similar names are historical evidence, not a second apply contract.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the shipped palette, when `apply` runs against an empty config root, then one sealed generation contains every normalized template output with no unexpanded `{{`, and a successful publication selects it for future launches | renderer coverage retained; publication: SPEC 0011 |
| A2 | Given a current generation, when `apply` runs again with the same inputs, then it publishes according to SPEC 0011, reports a generation publication outcome, and sends no reload; it need not report `unchanged` or optimize the apply away | SPEC 0011 G11 |
| A3 | Given a palette with `accent.violet` changed, when the candidate is rendered, then only outputs referencing violet have different bytes; apply publishes the complete candidate generation rather than rewriting mutable targets selectively | `theme::tests::only_outputs_referencing_the_changed_colour_are_rewritten` (rendering relation only) |
| A4 | Given `contrast = 1.30`, when `apply` runs, then output colours match `Palette::derived()` and no template contains the literal source colour | `theme::tests::outputs_carry_derived_colours_not_source_literals`, `render::tests::every_template_renders_across_the_whole_contrast_range` |
| A5 | Given a template with an unknown placeholder, when `apply` runs, then it fails with the placeholder name and leaves `current` unchanged | `render::tests::unknown_paths_and_transforms_are_errors_not_empty_strings`; publication: SPEC 0011 |
| A6 | Given a palette with a fatal lint finding, when `apply` runs, then it refuses, prints the findings, and leaves `current` unchanged | lint behavior retained; publication: SPEC 0011 |
| A7 | Given a successful apply or rollback, when its pointer commit completes, then it invokes no reload mechanism and existing processes remain pinned to their selected generation | SPEC 0011 G11 |
| A8 | Given no user palette, when `apply` runs, then the shipped palette is copied to the user config path first | `theme::tests::first_run_copies_the_shipped_palette_to_the_user_config` |
| A9 | Given the shipped palette, when `realmctl theme lint` runs, then it exits 0 and prints the accent hue separations | `theme_cli::lint_shipped_palette_is_session_independent_and_prints_hue_separations` |
| A10 | Given a fully validated current generation and a modified candidate, when `realmctl theme diff` runs, then it reports only sorted `added`, `removed`, and `byte-different` normalized outputs and performs no control initialization, recovery, lease, publication, pointer change, output write, or reload | `theme_cli::diff_after_palette_edit_is_sorted_and_does_not_mutate_generation_tree`; `theme_cli::diff_refusal_for_missing_current_does_not_mutate_generation_tree` |
| A11 | Given an empty, escaping, duplicate, symlinked, or prefix-colliding output path, or an unsafe configuration/generated root, when `apply` runs, then it refuses before publishing a generation or touching anything outside Realm's owned subtree | SPEC 0011 |
| A12 | Given an attacker-controlled staging or generation entry, when `apply` runs, then descriptor-relative no-follow validation refuses it without modifying its destination | SPEC 0011 |
| A13 | Given a symlinked configuration root spelled directly, with trailing separators, with terminal `.` components, or both, or a symlinked `realm` palette directory or `palette.toml`, when palette loading or first-run initialization runs, then it refuses without reading or writing the link destination | `theme::tests::a_symlinked_palette_path_is_refused_without_touching_its_destination`, `theme::tests::a_symlinked_palette_root_with_a_trailing_separator_is_refused_without_initializing_its_destination`, `theme::tests::a_symlinked_palette_root_with_terminal_dot_is_refused_without_reading_its_destination` |
| A14 | Given a generation output parent is replaced after its directory descriptor is acquired, when staging, validation, cleanup, or commit proceeds, then no operation follows the replacement outside the held generation tree | SPEC 0011 |
| A15 | Given the shipped palette, when the Starship template renders, then it contains dynamic user and host modules, the literal separator, an ASCII `~% ` character with a trailing command separator, thoth teal for the normal prompt, palette accents for Git/error states, and no exotic prompt sigil | `render::tests::starship_prompt_is_complete_and_ascii_safe_by_default` |

A9 and A10 are exercised through the `realmctl` integration tests. A10 requires
the generation-aware read-only boundary from SPEC 0011; the legacy
mutable-output diff test does not satisfy it.

## Budgets

`apply` retains the **< 150 ms** budget for the full template set on a warm
cache ([ARCHITECTURE.md §4](../ARCHITECTURE.md)). Rendering is serial: the whole
shipped set is a few tens of kilobytes of string scanning and lands in
single-digit milliseconds. Generation validation, sealing, fsync, and pointer
publication are included; skipping byte-identical mutable targets is not an
available optimization.

## Failure modes

From [PITFALLS.md](../PITFALLS.md), this component owns: "partial or mismatched
theme generation becomes selectable",
"colour written down twice", "contrast implemented as a filter", and
"unreadable palette after a tweak". It contributes to "Flatpak apps ignore the
theme", which is documented as a limit rather than fought.

## Generated-file ownership and activation

Realm owns **only** `$XDG_CONFIG_HOME/realm/generated/**`. Every generated file
that `theme apply` publishes or `theme diff` compares is a manifest-listed
output below one generation in that subtree. Apply never writes a program's
ordinary configuration or a mutable compatibility target.

Any import, wrapper, or launch-profile integration which points a program at
its selected generation is provisioning outside theme apply. It must not grant
Realm ownership of an existing user configuration. `realmctl doctor` may report
missing integration, but apply does not repair it as a side effect.

## Open questions

- **Kvantum SVG generation.** Generating a Kvantum theme means emitting SVG, not
  just a colour list. Worth it in M1, or is a qt6ct colour scheme enough until
  M6? *Recommendation: qt6ct only in M1.*
- **Live upgrade, including Qt.** Deliberately out of scope here. #22 may
  specify a generation-aware owned-process protocol; direct reload on pointer
  switch remains forbidden.
