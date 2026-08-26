# SPEC NNNN — <Component or feature>

- **Status:** Draft | Accepted | Implemented | Superseded
- **Milestone:** M<n>
- **Decisions:** ADR NNNN, ADR NNNN
- **Supersedes / Superseded by:** —

## Purpose

One paragraph. What this exists to do, and what the desktop cannot do without
it. If you cannot say why a user would notice its absence, it may not be needed.

## Scope

**In:** the behaviours this spec is responsible for.

**Out:** the neighbouring things it is explicitly not responsible for, with a
pointer to whatever is.

## Behaviour

The contract, stated so someone could implement it without guessing. Types and
signatures where they clarify; prose where they do not. Say what happens on the
unhappy paths too, even though they are not tested first.

## Acceptance criteria

Each row is one happy path and becomes one test. Fill `Test` in once the test
exists — that is what moves this spec to Implemented.

| # | Given / When / Then | Test |
|---|---|---|
| A1 | | |
| A2 | | |

## Budgets

Any timing, allocation or frame budget this component must hold, and how it is
measured. Reference `docs/ARCHITECTURE.md` §4 rather than inventing new numbers.

## Failure modes

Which rows of `docs/PITFALLS.md` this component is responsible for not causing,
and what guards each.

## Open questions

Anything genuinely undecided. A question here with the `needs-human` label on
its issue is the honest way to stay blocked; guessing is not.
