---
name: helm-issue-flow
description: Use when picking up or closing out work in this repo - asking "what should I work on next", starting or running the agentic loop, opening or closing a GitHub issue, deciding whether something needs the needs-human label, writing a commit message, or wondering whether it is safe to stop. Also use when an issue's acceptance criteria look ambiguous, when a milestone appears to be finished, or when a change would contradict an ADR or a spec.
---

# How work is tracked and closed

The protocol is `.claude/memory/40-loop.md`. **Read it — this skill is the
index, not a replacement.** What follows is the shape of the loop and the four
judgement calls it does not make for you.

## The loop, in one line each

Eight steps, safe to interrupt at any point because every iteration ends with a
green build, a pushed commit and an updated issue.

```
1 ORIENT ─ 2 PICK ─ 3 SPEC ─ 4 TEST ─ 5 BUILD ─ 6 VERIFY ─ 7 LAND ─ 8 REFLECT ─┐
└──────────────────────────────────────────────────────────────────────────────┘
```

1. **Orient** — `CLAUDE.md`, then `00-standing-orders.md`, the open milestone,
   `git log --oneline -10`. Do not re-derive what memory already says.
2. **Pick** — see the priority order below.
3. **Spec** — write or update `docs/specs/`, plus an ADR if a real decision is
   involved, *before* touching code (S14).
4. **Test** — turn the spec's acceptance criteria into happy-path tests and run
   them to watch them fail.
5. **Build** — the smallest change that turns them green.
6. **Verify** — `cargo fmt --all && cargo clippy --all-targets -- -D warnings &&
   cargo test`, plus the issue's own acceptance check.
7. **Land** — commit saying *why*, `Closes #N`, push to the working branch,
   update the issue.
8. **Reflect** — test names back into the spec, then anything durable into
   `.claude/memory/` or `docs/PITFALLS.md`.

Steps 3 and 4 are the spec-first gate. The `helm-spec-first` skill covers them
in detail; the short version is that an issue with no accepted spec is not ready
to be coded, and **writing that spec is the first slice of the work**, not a
reason to skip the issue.

## Picking

Highest-priority *unblocked* issue in the **earliest open milestone**:

```
blocked-others  ›  bug  ›  mvp  ›  everything else
```

Skip anything labelled `needs-human`, and say that you skipped it. The loop only
works inside M0–M3 unless a human moves the marker: shipping the MVP
(`docs/MVP.md`) outranks starting M4.

If an issue's acceptance criteria are ambiguous, the ambiguity belongs in the
spec's **Open questions**. Comment on the issue with the specific question, pick
the next issue, and do not guess at a contract.

## `needs-human`, and what a good one says

Standing order S3. Apply it when the repo cannot supply the judgement: physical
hardware, credentials, a licence call, a security decision, or a design
trade-off with no obviously right answer. Never silently guess and move on.

A good `needs-human` issue has three parts, in this order:

1. **The decision** — one sentence, phrased as a question with a subject.
   "Where do generated GTK files live?", not "GTK theming is complicated".
2. **The options** — each with its cost. Two or three; if you have five you have
   not thought about it yet.
3. **Your recommendation** — and why. A recommendation is not a decision, and
   offering one is not overstepping; it is the difference between a question a
   human can answer in a minute and one that needs an afternoon.

Surface these in the README so they are not buried. The Open questions section
of `docs/specs/0002-theme-pipeline.md` is the worked example.

## Committing and closing

- The message says **why**, not what — the diff already says what. House style,
  same as code comments.
- Reference the issue so it closes from the commit: `Closes #N`. Issues are not
  closed by hand and not tracked in a scratch file (S2).
- Never push red, never force-push a shared branch. If it cannot be made green,
  revert and reopen the issue with what was learned.
- Never leave an issue in progress without a comment saying where it stands.
- Never widen scope mid-iteration — file a follow-up issue.

## Stop conditions

Hand back to a human when any of these is true:

- The milestone has no unblocked, non-`needs-human` issues left.
- The same test has failed on two consecutive iterations for *different* reasons
  — that is a sign the model of the problem is wrong, not the code.
- A change would alter an ADR's decision or contradict a committed spec. Write
  the superseding ADR or the spec revision and **ask first**.
- Something needs hardware, credentials, a licence call or a design judgement:
  label it `needs-human`, state the options and the recommendation, move on.

Full text, including the invariants and how the scheduled workflow runs:
`.claude/memory/40-loop.md`.
