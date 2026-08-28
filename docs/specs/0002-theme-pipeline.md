# SPEC 0002 — Theme pipeline

- **Status:** Accepted; implemented through A14 (2026-08-27), including A11's
  equivalent-spelling configuration-root correction, A13's matching palette
  containment rule, and #110's A12/A14 descriptor-relative writer
  protections. Multi-file all-or-nothing publication remains #22's boundary.
- **Milestone:** M1
- **Decisions:** [ADR 0005](../adr/0005-palette-toml-single-source.md),
  [ADR 0006](../adr/0006-oklab-contrast-not-filters.md)
- **Implements:** [INTERFACES.md §2](../INTERFACES.md)

> Written before the code, as S14 requires. The tests below were written from
> these criteria, watched to fail against an unimplemented `helm-theme`, and
> only then implemented against. All of them live in `crates/helm-theme`; run
> them with `cargo test -p helm-theme`.

## Purpose

One palette file must retint the entire desktop — GTK apps, Qt apps, the
terminal, yazi, btop, the shell prompt and helm's own clients — in a single
step, fast enough that a user tweaking a colour sees the result immediately.

This is the difference between a desktop environment and a collection of
programs that happen to be running at the same time. It is also the component
most likely to produce a visibly broken desktop if it gets it half right, which
is why atomicity is a hard requirement rather than a nicety.

## Scope

**In:** reading `palette.toml`; deriving the contrast variant; rendering every
template; writing outputs atomically; fanning out reloads exactly once; the
`helm ctl theme apply|lint|diff` surface.

**Out:** the palette *format* and its validation (SPEC 0001, `helm-core`);
deciding which colour goes where (that is `palette.toml` itself); anything that
draws (`helm-bar`).

## Behaviour

### Inputs

`~/.config/helm/palette.toml`, falling back to the shipped `palette.toml`.
Parsed and validated by `helm_core::Palette`. A palette with any **fatal** lint
finding is refused: nothing is written, the previous theme stays live, and the
findings are printed. Non-fatal findings are printed and applied.

### Derivation

Templates address the **derived** palette (`Palette::derived()`), so `contrast`
is folded in exactly once, at this boundary. No template may apply contrast
itself, and nothing downstream may use a display-side contrast filter — see
ADR 0006 for why that costs a fullscreen pass per frame and rotates hues.

### Rendering

Placeholder vocabulary is defined in [INTERFACES.md §2](../INTERFACES.md).
An unknown placeholder is a **hard error** that aborts the whole apply. A
silently blank colour is the exact bug this design exists to prevent.

Implementing it added two forms, both supersets of that table rather than
changes to it. An alpha argument may be a path as well as a literal —
`{{ border.seam.rgba(border.seam_alpha) }}` — so the alphas stay written down
once, beside the colours they belong to; and because `.over(...)` yields a
colour it may be followed by `.bare` or `.rgba(...)`, which is what lets an
alpha-less bare-hex format like fuzzel's express a translucent seam.

### Atomic application

The ordering is not negotiable, because a half-applied theme — a new terminal
palette against an old GTK stylesheet — is both ugly and hard to diagnose:

1. Render every template to memory.
2. For each output whose bytes differ from what is on disk, write a unique
   no-follow staging sibling named from `<target>.helm-tmp`, and `fsync` it.
3. `rename(2)` every temp file into place. (Same filesystem, so the rename is
   atomic per file.)
4. Only then, fan out reloads — once each, deduplicated.

If any step before 3 fails, nothing is renamed and the desktop is untouched.
Outputs that are byte-identical are neither rewritten nor reloaded, so a
no-op apply costs nothing and does not flash the desktop.

### Output containment

Template targets are normalized relative paths below the caller-supplied
configuration root. Empty, absolute, traversal, duplicate, and symlinked
targets are refused before rendering or writing. The configuration root itself
must not be a symlink under any equivalent terminal spelling, including one or
more trailing separators, terminal `.` components, or both. Root spelling is
normalized lexically without resolving components; `/` and `.` retain their
existing behavior. Staging files are created exclusively and without following
links, so an existing `<target>.helm-tmp` cannot redirect or clobber another
file. Descriptor-relative operations own the remaining replacement-race
protection for the final writer. The writer opens the configuration root once
with `NOFOLLOW`, opens or creates every normalized parent relative to that
descriptor with `NOFOLLOW`, and performs comparison, staging, rename, cleanup,
and directory `fsync` through the held descriptor. A later pathname replacement
may prevent the requested update from becoming visible, but must never redirect
a read, write, cleanup, or rename outside the originally opened directory.
Palette lookup and first-run initialization apply the same lexical root
handling: a symlinked configuration root must be refused under trailing-
separator and terminal-`.` spellings, and the root's `helm` directory and
`palette.toml` must be real children rather than symlinks before a copy or read
occurs. These per-file containment guarantees do not add multi-file
all-or-nothing publication, which remains #22's boundary.

### Reload fan-out

| Target | Mechanism |
|---|---|
| GTK 3/4, libadwaita | `gsettings set org.gnome.desktop.interface ...`, then GTK re-reads its stylesheet |
| Qt (qt6ct) | rewrite the colour scheme; running apps pick it up on next start |
| foot / terminals | `SIGUSR1` to the process |
| yazi, btop, starship | read on next start; no signal available |
| helm's own clients | session notification is planned; its IPC event/ordering contract is deferred until accepted and test-first implemented |

Where a target genuinely cannot hot-reload, `apply` says so in its report rather
than pretending. Users should never wonder whether it worked.

### First-run

If no user palette exists, `apply` copies the shipped one to
`~/.config/helm/palette.toml` first, so a user's first edit is to their own file.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the shipped palette, when `apply` runs against an empty config root, then every template's output file exists and contains no unexpanded `{{` | `theme::tests::apply_writes_every_template_with_no_unexpanded_placeholders` |
| A2 | Given a rendered theme, when `apply` runs again with the same palette, then every output is reported `unchanged` and nothing is reloaded | `theme::tests::a_second_apply_changes_nothing_and_reloads_nothing` |
| A3 | Given a palette with `accent.violet` changed, when `apply` runs, then only the outputs referencing violet are rewritten | `theme::tests::only_outputs_referencing_the_changed_colour_are_rewritten` |
| A4 | Given `contrast = 1.30`, when `apply` runs, then output colours match `Palette::derived()` and no template contains the literal source colour | `theme::tests::outputs_carry_derived_colours_not_source_literals`, `render::tests::every_template_renders_across_the_whole_contrast_range` |
| A5 | Given a template with an unknown placeholder, when `apply` runs, then it fails with the placeholder name and writes no file | `theme::tests::an_unknown_placeholder_aborts_the_apply_and_writes_nothing`, `render::tests::unknown_paths_and_transforms_are_errors_not_empty_strings` |
| A6 | Given a palette with a fatal lint finding, when `apply` runs, then it refuses, prints the findings, and leaves existing outputs untouched | `theme::tests::a_fatally_linted_palette_is_refused_and_leaves_the_theme_live` |
| A7 | Given a successful apply, when the reload fan-out runs, then each implemented reload mechanism fires exactly once regardless of how many templates share it | `theme::tests::each_reload_mechanism_fires_exactly_once` |
| A8 | Given no user palette, when `apply` runs, then the shipped palette is copied to the user config path first | `theme::tests::first_run_copies_the_shipped_palette_to_the_user_config` |
| A9 | Given the shipped palette, when `helm ctl theme lint` runs, then it exits 0 and prints the accent hue separations | `theme::tests::lint_report_is_clean_for_the_shipped_palette_and_lists_hue_separations` |
| A10 | Given a modified palette, when `helm ctl theme diff` runs, then it prints which outputs would change without writing any | `theme::tests::diff_reports_what_would_change_and_writes_nothing` |
| A11 | Given an empty, escaping, duplicate, or symlinked output path, or a symlinked configuration root spelled directly, with trailing separators, with terminal `.` components, or both, when `apply` runs, then it refuses before touching an output outside the configuration root | `theme::tests::unsafe_or_duplicate_targets_abort_before_any_output_is_written`, `theme::tests::a_symlinked_output_parent_is_refused_without_touching_its_destination`, `theme::tests::a_symlinked_configuration_root_is_refused`, `theme::tests::a_symlinked_configuration_root_with_a_trailing_separator_is_refused`, `theme::tests::a_symlinked_configuration_root_with_terminal_dot_is_refused` |
| A12 | Given an attacker-planted predictable staging symlink, when `apply` runs, then it does not modify that symlink's destination and stages only through a unique no-follow sibling | `theme::tests::a_symlinked_staging_file_is_not_touched` |
| A13 | Given a symlinked configuration root spelled directly, with trailing separators, with terminal `.` components, or both, or a symlinked `helm` palette directory or `palette.toml`, when palette loading or first-run initialization runs, then it refuses without reading or writing the link destination | `theme::tests::a_symlinked_palette_path_is_refused_without_touching_its_destination`, `theme::tests::a_symlinked_palette_root_with_a_trailing_separator_is_refused_without_initializing_its_destination`, `theme::tests::a_symlinked_palette_root_with_terminal_dot_is_refused_without_reading_its_destination` |
| A14 | Given an output parent is replaced with a symlink after its directory descriptor is acquired, when staging, cleanup, or commit proceeds, then no operation reaches the symlink destination | `theme::tests::a_replaced_output_parent_cannot_redirect_descriptor_relative_writes` |

A9 and A10 name `helm ctl` subcommands, but the CLI is a separate M1 slice and
does not exist yet. Both are tested at the library boundary the subcommands will
print — `helm_theme::lint` and `helm_theme::diff` — so "exits 0" is asserted as
"reports no fatal finding". The tests move to `ctl::tests::` when the CLI lands.

## Budgets

`apply` completes in **< 150 ms** for the full template set on a warm cache
([ARCHITECTURE.md §4](../ARCHITECTURE.md)). Rendering is serial: the whole
shipped set is a few tens of kilobytes of string scanning and lands in
single-digit milliseconds, which is less than the thread-spawn overhead that
parallelism would add. The rename phase is serial and cheap. The budget is a CI gate, not
a goal.

## Failure modes

From [PITFALLS.md](../PITFALLS.md), this component owns: "half-applied theme",
"colour written down twice", "contrast implemented as a filter", and
"unreadable palette after a tweak". It contributes to "Flatpak apps ignore the
theme", which is documented as a limit rather than fought.

## Generated-file ownership and activation

Helm owns **only** `$XDG_CONFIG_HOME/helm/generated/**`. Every file that
`theme apply` may replace, remove, or compare lives below that subtree. A
template for a program with a user-owned configuration file renders to a
tool-specific file below that subtree; it never writes into the program's
ordinary configuration directory.

For an activation file that the program reads by default (for example GTK's
`gtk.css`), `theme apply` may create the user-side file *only when it is
absent*. That first-run file contains only the documented Helm import and is
thereafter user-owned. If the activation file already exists, Helm must not
modify, replace, append to, or delete it. `helmctl doctor` reports the exact
import line and target file when the existing configuration does not activate
the generated theme.

This rule keeps the writer's atomicity boundary wholly inside an owned
subtree. It does not grant Helm ownership of a configuration merely because
that configuration currently imports Helm, and it does not change #22's
multi-file-publication boundary.

## Open questions

- **Kvantum SVG generation.** Generating a Kvantum theme means emitting SVG, not
  just a colour list. Worth it in M1, or is a qt6ct colour scheme enough until
  M6? *Recommendation: qt6ct only in M1.*
- **Reload for Qt.** No reliable hot-reload path for running Qt apps was found.
  Accepting "applies on next start" for M1 unless someone knows better.
