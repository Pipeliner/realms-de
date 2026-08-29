# Worked examples: acceptance criteria into tests

Two examples. The first shows how SPEC 0002's preserved rendering and lint
criteria combine with SPEC 0011's governing generation contract. The second is
SPEC 0001, retro-fitted, which shows what a finished table looks like.

The rules live in `SKILL.md`.

## SPEC 0002 + SPEC 0011 — theme criteria after supersession

SPEC 0002 remains Accepted for palette initialization, derivation, rendering,
placeholder validation, and linting. SPEC 0011 and ADR 0017 explicitly
supersede its historical mutable-target publication, equality/no-op reporting,
mutable diff, and reload fan-out. New apply/diff work starts from SPEC 0011's
generation acceptance criteria, not from the retired mechanics below.

| # | Current criterion | Governing acceptance row | What "watch it fail" must expose |
|---|---|---|---|
| A1 | Apply renders every normalized output with no unexpanded `{{`, publishes one sealed generation, and selects it for future launches | SPEC 0002 A1; SPEC 0011 G1–G5, G9 | a partial, unsealed, or mixed generation becomes selectable |
| A2 | Applying identical inputs still returns a generation publication outcome and sends no reload; no `unchanged` no-op is promised | SPEC 0002 A2; SPEC 0011 G11–G12 | apply exposes the retired mutable result or notifies a running process |
| A3 | Changing `accent.violet` changes only candidate output bytes that depend on violet, while apply publishes the complete generation | SPEC 0002 A3; SPEC 0011 G4 | publication selectively mutates current targets or mixes input snapshots |
| A4 | At `contrast = 1.30`, outputs match `Palette::derived()` and contain no source literal | SPEC 0002 A4 | contrast is applied late, omitted, or changes hue unexpectedly |
| A5 | An unknown placeholder fails with its name and leaves `current` unchanged | SPEC 0002 A5; SPEC 0011 G2–G5 | a blank placeholder or partial generation is selected |
| A6 | Fatal lint findings refuse publication and leave `current` unchanged | SPEC 0002 A6; SPEC 0011 G2–G5 | invalid palette output reaches a selectable generation |
| A7 | Pointer commit sends no signal, command, notification, or reload; existing processes remain on their selected generation | SPEC 0002 A7; SPEC 0011 G7, G11 | a pointer switch changes or notifies an already-running process |
| A8 | With no user palette, initialization copies the shipped palette safely before capture | SPEC 0002 A8 | initialization reads in place or follows an unsafe path |
| A9 | `theme lint` accepts the shipped palette and prints hue separations | SPEC 0002 A9 | the shipped palette fails its own lint contract |
| A10 | `theme diff` compares candidate outputs with a fully validated `current` generation, reports sorted `added`/`removed`/`byte-different` paths, and mutates nothing | SPEC 0002 A10; SPEC 0011 G10 | diff treats missing/invalid current as empty, writes state, or reports control files |

Notes on the shape of these:

- **Atomicity is testable without a compositor.** A5 and A6 assert that a
  failure leaves `current` naming the previous fully validated generation. Use
  a temporary generated root and compare the pointer plus sealed inventory.
- **A7 forbids a reload seam in supported apply.** `Reload` remains canonical
  catalogue metadata, but pointer commit does not execute it. Live upgrade and
  wire compatibility are out of scope; a future protocol must be
  generation-aware and cannot restore reload as a pointer-switch side effect.
- **None of these needs a live desktop.** That is deliberate: the agent
  container has no Wayland display and no D-Bus session, so anything requiring
  one is `needs-human` and belongs in a different slice.
- **What is *not* on this list** is as important. It does not promise a no-op
  apply, compatibility with the retired mutable result or wire messages, or a
  live upgrade path.

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
