# The agentic loop

How autonomous work proceeds in this repo. The loop is designed to be **safe to
interrupt at any point**: every iteration ends with a green build, a pushed
commit and an updated issue, so stopping mid-run never loses or corrupts work.

---

## One iteration

```
  ┌─▶ 1. ORIENT ─▶ 2. PICK ─▶ 3. SPEC ─▶ 4. TEST ─▶ 5. BUILD ─▶ 6. VERIFY ─▶ 7. LAND ─┐
  │                                                                                   │
  └───────────────────────────── 8. REFLECT ◀─────────────────────────────────────────┘
```

**1. Orient.** Read `CLAUDE.md`, then `00-standing-orders.md`. Check the open
milestone and `git log --oneline -10`. Do not re-derive what memory already says.

**2. Pick.** The highest-priority *unblocked* issue in the earliest open
milestone. Priority order:
`blocked-others` › `bug` › `mvp` › everything else.
Skip anything labelled `needs-human` — leave it and say so.

**3. Spec.** Write or update the spec in `docs/specs/` before touching code
(S14). If the change embeds a decision with real alternatives, write the ADR
too. If the issue's acceptance criteria are ambiguous, that ambiguity belongs in
the spec's "Open questions" — comment on the issue, pick the next one, and do
not guess at a contract.

**4. Test.** Turn the spec's acceptance criteria into happy-path tests and
**run them to watch them fail**. A test that has never failed has not been shown
to test anything. Keep it minimal: the intended path, not an edge-case matrix.

**5. Build.** Smallest change that turns those tests green. Fan work out to
subagents when the pieces are independent and touch disjoint paths (S11).
Serialise anything sharing a not-yet-committed contract.

**6. Verify.** `cargo fmt --all && cargo clippy --all-targets -- -D warnings &&
cargo test`. Plus the issue's own acceptance check. **A red build is never
pushed** — if it cannot be made green, revert the change and reopen the issue
with what was learned.

**7. Land.** Commit with a message that says *why*. Reference the issue
(`Closes #N`). Push to the working branch. Update the issue with what landed and
what remains.

**8. Reflect.** Record the passing test names back into the spec's acceptance
criteria, then append anything durable to memory: a decision to
`10-decisions.md`, a toolchain fact to `20-environment.md`, a time-sink to
`30-gotchas.md`, a new DE failure mode to `docs/PITFALLS.md`. Then loop.

---

## Stop conditions

Stop and hand back to a human when any of these is true:

- The milestone has no unblocked, non-`needs-human` issues left.
- The same test has failed on two consecutive iterations for different reasons.
- A change would alter an ADR's decision, or contradict a committed spec — write
  the superseding ADR or the spec revision, and ask first.
- Something needs hardware, credentials, a licence call or a design judgement:
  label it `needs-human`, state the options and the recommendation, move on.

## Invariants

1. Never push red. Never force-push a shared branch.
1b. Never implement ahead of a spec, and never leave a spec contradicted by the
   code that implements it.
2. Never leave an issue in progress without a comment saying where it stands.
3. Never widen scope mid-iteration; file a follow-up issue instead.
4. Never let the loop run against a milestone whose ADRs are unreviewed.
5. Every iteration leaves the repo installable from a clean clone.

## Spec-number allocation

Before creating a new `docs/specs/NNNN-*.md`, inspect open pull requests as
well as `main`. Draft specs reserve their number: as of 2026-08-30, PRs #162
through #165 reserve 0012 through 0015 even though `main` ends at 0011. Use the
next unreserved number and reconcile the index in the same change; never reuse
or silently collide with an in-flight specification.

## Remote ShellCheck authority

The Nix `shellcheck` check is a merge gate even when this host lacks a local
ShellCheck binary. On 2026-08-30 it rejected SC2016 (a single-quoted Markdown
literal containing backticks) and SC1112 (a typographic apostrophe). Treat
remote Nix/ShellCheck as the authoritative verification for changed shell
scripts; keep shell literals ASCII where practical and write quoted Markdown so
ShellCheck can see literal intent without a suppression.

## Running it

- **Manually:** `/loop` with the iteration prompt, or ask for "next issue".
- **On a schedule:** the `agentic-loop` workflow in `.github/workflows/` opens a
  session on a cadence; it is opt-in and defaults to dry-run.
- **Scope guard:** the loop only ever works inside milestones M0–M3 unless a
  human moves the marker. Shipping the MVP outranks starting M4.
