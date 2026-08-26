# Worked examples: acceptance criteria into tests

Two examples. The first is SPEC 0002, whose `Test` column is still empty — this
is the work waiting to be done. The second is SPEC 0001, retro-fitted, which
shows what a finished table looks like.

The rules live in `SKILL.md`.

## SPEC 0002 — the theme pipeline, criteria into test names

`docs/specs/0002-theme-pipeline.md` is Accepted for M1 with an empty `Test`
column. Each row below is one happy path, one test, written and watched to fail
before `helm-theme` exists in any working form.

| # | The criterion | The test it becomes | What "watch it fail" looks like |
|---|---|---|---|
| A1 | Apply against an empty config root writes every output, with no unexpanded `{{` | `theme::tests::apply_writes_every_template_with_no_unexpanded_placeholders` | fails to compile — `apply` does not exist. That is a legitimate red |
| A2 | A second apply with the same palette reports everything `unchanged` and reloads nothing | `theme::tests::a_second_apply_changes_nothing_and_reloads_nothing` | fails because `Applied::unchanged` is empty and reloads fire regardless |
| A3 | Changing `accent.violet` rewrites only the outputs that reference violet | `theme::tests::only_outputs_referencing_the_changed_colour_are_rewritten` | fails because every file is rewritten |
| A4 | At `contrast = 1.30` outputs match `Palette::derived()` and contain no source literal | `theme::tests::outputs_carry_derived_colours_not_source_literals` | fails while contrast is applied late, or not at all |
| A5 | An unknown placeholder fails with its name and writes nothing | `theme::tests::an_unknown_placeholder_aborts_the_apply_and_writes_nothing` | fails while unknown placeholders render as empty strings — the exact bug the design exists to prevent |
| A6 | A fatally linted palette is refused and existing outputs are untouched | `theme::tests::a_fatally_linted_palette_is_refused_and_leaves_the_theme_live` | fails while `apply` writes first and lints later |
| A7 | Each reload mechanism fires exactly once regardless of how many templates share it | `theme::tests::each_reload_mechanism_fires_exactly_once` | fails while reloads fan out per template |
| A8 | With no user palette, the shipped one is copied to the user config path first | `theme::tests::first_run_copies_the_shipped_palette_to_the_user_config` | fails while `apply` reads the shipped palette in place |
| A9 | `helm ctl theme lint` exits 0 on the shipped palette and prints hue separations | `ctl::tests::theme_lint_exits_zero_on_the_shipped_palette` | fails until the subcommand exists |
| A10 | `helm ctl theme diff` prints what would change and writes nothing | `ctl::tests::theme_diff_writes_nothing` | fails until the subcommand exists |

Notes on the shape of these:

- **Atomicity is testable without a compositor.** A5 and A6 are the atomicity
  criteria in disguise: both assert that a failure leaves the filesystem exactly
  as it was. Point them at a temporary config root and compare the whole tree
  before and after.
- **A7 needs a seam.** Counting reloads means `Reload` execution has to be
  injectable — a `FnMut(&Reload)` or a small trait. The test is what forces that
  design, which is the point of writing it first.
- **None of these needs a live desktop.** That is deliberate: the agent
  container has no Wayland display and no D-Bus session, so anything requiring
  one is `needs-human` and belongs in a different slice.
- **What is *not* on this list** is as important. There is no test for a
  read-only target directory, a full disk, or a palette with a byte-order mark.
  Those are edge cases; they earn tests when they turn up as bugs.

## SPEC 0001 — a finished table

`docs/specs/0001-helm-core-contracts.md` is Implemented, and every row names a
test that exists and passes today. It was written after its code — recorded
honestly as such in the spec's own header — and from SPEC 0002 onward the order
is spec first.

Read it for two things:

1. **The naming convention.** `ledger::tests::summon_inserts_after_focus_not_at_the_end`
   states the behaviour, not the function under test. In a spec table and in a
   CI failure, that name is the whole message.
2. **Where the invariant exception applies.** Criteria A6 (every layout tiles
   exactly at every plausible size) and A9 (contrast raises separation without
   desaturating or rotating accents) are sweeps, not examples — the two standing
   exceptions to minimal TDD. Everything else on that table is a single happy
   path.

A row may name more than one test where the behaviour genuinely has two halves —
A3 names both `undo_restores_the_exact_previous_ledger` and
`undo_history_is_bounded`. That is not a licence to grow the matrix; it is the
honest count for "restores exactly, and stays bounded".
