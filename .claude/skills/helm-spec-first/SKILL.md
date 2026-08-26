---
name: helm-spec-first
description: Use before implementing anything non-trivial in this repo - adding a component, a feature, a command, a template, a client or a new behaviour to an existing crate - and whenever someone asks "should I write a test first", proposes a new crate or module, or opens a file under crates/ to add behaviour rather than fix a typo. Also use when writing or revising a spec in docs/specs/ or an ADR in docs/adr/, turning acceptance criteria into tests, or deciding whether something needs a spec, an ADR, both or neither.
---

# Spec first, then a failing test, then code

Standing order S14: specs and ADRs outrank code. Nothing non-trivial is
implemented before its behaviour is written down, because a contract discovered
while implementing is a contract only its author understands — and this repo is
worked on by several agents in parallel, where an unwritten contract means two
components inventing the same type twice.

The order, from `.claude/memory/40-loop.md` steps 3–5 and 8:

```
spec  →  ADR (if a decision)  →  failing happy-path tests  →  code  →  test names back into the spec
```

## Spec, ADR, both, or neither

`docs/specs/README.md` draws the line; in practice:

| You are about to… | Write |
|---|---|
| Define what a component or feature must do | a **spec** in `docs/specs/NNNN-name.md` |
| Choose between real alternatives with a cost either way | an **ADR** in `docs/adr/NNNN-name.md`, linked from the spec |
| Both: a new component whose shape was a genuine choice | both, cross-linked — the spec says *what*, the ADR says *why not the other thing* |
| Fix a bug, rename a thing, tidy a comment, add a test for existing behaviour | neither. Just do it |
| Add an edge case to something already specified | neither, usually — but if the edge case changes the contract, it is a spec revision |

A spec that starts arguing a decision should hand that argument to an ADR. An
ADR that starts specifying behaviour should hand it to a spec. Both link back.

Copy `docs/specs/TEMPLATE.md`; it already requires the sections that get
forgotten — Scope **Out**, Budgets referencing `docs/ARCHITECTURE.md` §4,
Failure modes referencing `docs/PITFALLS.md`, and Open questions.

## Status is a gate, not a label

`Draft → Accepted → Implemented → (Superseded)`.

- **Draft** may not be implemented against. Open questions are still open; that
  is what Draft means.
- **Accepted** means the happy-path tests may now be written.
- **Implemented** means every acceptance row names a real, passing test.

An open question with no obviously right answer is not a reason to stall — it is
a `needs-human` issue stating the decision, the options and your recommendation
(standing order S3). SPEC 0002's "where generated files live" question is the
worked example of that.

## Acceptance criteria → tests

Each acceptance row is exactly one happy path and becomes exactly one test.
Write them in Given / When / Then form, because that dictates the test's
arrange / act / assert without further thought.

Then, in order:

1. Write the test. Name it after the behaviour, not the function — these names
   are read in the spec table and in CI output, so
   `summon_inserts_after_focus_not_at_the_end` earns its length and
   `test_summon_2` does not.
2. **Run it and watch it fail.** A test that has never failed has not been shown
   to test anything; it may be asserting something already true, or nothing at
   all. This is the step people skip, and it is the one that gives the practice
   its value.
3. Implement the smallest thing that turns it green. Nothing more — extra
   behaviour has no spec row and no test.
4. Write the passing test name back into the spec's `Test` column. That is what
   moves the spec to Implemented and what keeps it self-checking.

Minimal means minimal: the intended path, not a matrix of edge cases. Edge cases
earn tests when they turn up as bugs — and then they go in `docs/PITFALLS.md`
too, with the guard named.

## The standing exception: invariant tests

**Layout geometry and colour maths get invariant tests instead of example
tests.** An example test on `partition()` passes happily while the algorithm is
off by one pixel at a resolution nobody tried, and an example test on
`apply_contrast()` passes while an accent rotates 18° toward blue at the top of
the range. Both bug classes are invisible in review and glaring on screen.

So for those two areas, assert the property over a sweep: exact tiling at every
plausible size and window count; hue preserved across the whole contrast range.
`layout::tests::every_layout_tiles_exactly_for_every_plausible_size` and
`palette::tests::shipped_palette_survives_the_whole_contrast_range` are the
models to copy. The `helm-layout` and `helm-theming` skills have the details.

Everything else gets the happy path and stops.

## When implementing proves the spec wrong

This is normal and expected — it is why specs are provisional. What is not
allowed is leaving the two disagreeing.

**Correct the spec in the same commit as the code.** A spec that has silently
diverged is worse than no spec, because it is trusted. If the correction changes
a decision rather than a detail, stop: write the superseding ADR and ask, per the
loop's stop conditions.

## Examples

`examples.md` in this directory works SPEC 0002's acceptance criteria into the
test functions they should become, and shows the retro-fitted SPEC 0001 table as
a model for the `Test` column. Open it when you have criteria in front of you
and want to see the shape of the tests they turn into.
