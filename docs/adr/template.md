# ADR NNNN — <Title>

- **Status:** Proposed (YYYY-MM-DD)
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

## Context

What forces are in play, stated concretely and specific to helm. Reference the
design handoff, the frame budgets in `docs/ARCHITECTURE.md` §4, the target
distros, or the failure register in `docs/PITFALLS.md` where they bear on the
decision. Generic architecture prose does not belong here; if a sentence would
be equally true of any desktop environment, cut it.

## Decision

What we are doing, stated so that someone can act on it without reading the rest
of the document. One paragraph, or a short numbered list of commitments.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| … | … | … |
| … | … | … |

At least two real alternatives, argued fairly. A strawman here is worse than
nothing: the point of the table is that a future reader can tell whether the
losing option has since become the better one.

## Consequences

### Good

- …

### Bad

- …

### Neutral

- …

## Reversal

Which files change, roughly how much work it is, and what observation should
make us reconsider. Be honest: "structural" is a legitimate answer, but say so
rather than pretending everything is cheap.

## Guard

The test, lint or CI job that fails if this decision silently stops being true.
Name a real test where one exists, for example
`layout::tests::every_layout_tiles_exactly_for_every_plausible_size`. If the
guard does not exist yet, write it as *planned:* with the milestone it lands in,
and open an issue.

## Needs a human

Only when the decision genuinely cannot be made from inside the repo. Name the
options, state a recommendation, and say what evidence would settle it. Tag the
corresponding GitHub issue `needs-human` (standing order S3).
