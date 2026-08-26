# The agentic loop

How autonomous work proceeds in this repo. The loop is designed to be **safe to
interrupt at any point**: every iteration ends with a green build, a pushed
commit and an updated issue, so stopping mid-run never loses or corrupts work.

---

## One iteration

```
  ┌─▶ 1. ORIENT ──▶ 2. PICK ──▶ 3. PLAN ──▶ 4. BUILD ──▶ 5. VERIFY ──▶ 6. LAND ─┐
  │                                                                              │
  └──────────────────────── 7. REFLECT ◀─────────────────────────────────────────┘
```

**1. Orient.** Read `CLAUDE.md`, then `00-standing-orders.md`. Check the open
milestone and `git log --oneline -10`. Do not re-derive what memory already says.

**2. Pick.** The highest-priority *unblocked* issue in the earliest open
milestone. Priority order:
`blocked-others` › `bug` › `mvp` › everything else.
Skip anything labelled `needs-human` — leave it and say so.

**3. Plan.** Restate the issue's acceptance criteria in your own words. If they
are ambiguous, comment on the issue asking the specific question and pick the
next one; do not guess at a contract.

**4. Build.** Smallest change that satisfies the criteria. Fan work out to
subagents when the pieces are independent and touch disjoint paths (see S11).
Serialise anything sharing a not-yet-committed contract.

**5. Verify.** `cargo fmt --all && cargo clippy --all-targets -- -D warnings &&
cargo test`. Plus the issue's own acceptance check. **A red build is never
pushed** — if it cannot be made green, revert the change and reopen the issue
with what was learned.

**6. Land.** Commit with a message that says *why*. Reference the issue
(`Closes #N`). Push to the working branch. Update the issue with what landed and
what remains.

**7. Reflect.** Append anything durable to memory: a decision to
`10-decisions.md`, a toolchain fact to `20-environment.md`, a time-sink to
`30-gotchas.md`, a new DE failure mode to `docs/PITFALLS.md`. Then loop.

---

## Stop conditions

Stop and hand back to a human when any of these is true:

- The milestone has no unblocked, non-`needs-human` issues left.
- The same test has failed on two consecutive iterations for different reasons.
- A change would alter an ADR's decision — write the superseding ADR and ask
  first.
- Something needs hardware, credentials, a licence call or a design judgement:
  label it `needs-human`, state the options and the recommendation, move on.

## Invariants

1. Never push red. Never force-push a shared branch.
2. Never leave an issue in progress without a comment saying where it stands.
3. Never widen scope mid-iteration; file a follow-up issue instead.
4. Never let the loop run against a milestone whose ADRs are unreviewed.
5. Every iteration leaves the repo installable from a clean clone.

## Running it

- **Manually:** `/loop` with the iteration prompt, or ask for "next issue".
- **On a schedule:** the `agentic-loop` workflow in `.github/workflows/` opens a
  session on a cadence; it is opt-in and defaults to dry-run.
- **Scope guard:** the loop only ever works inside milestones M0–M3 unless a
  human moves the marker. Shipping the MVP outranks starting M4.
