# Contributing to helm

helm is a desktop environment, which means most of its bugs are other people's
bad afternoons: a file dialog that hangs, a tile with a hairline crack down it,
a theme that half-applies. The rules below exist so that those failures are
caught by a test rather than by a user, and so that a change made a year from
now can still be understood.

Read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §1–§3 and
[`docs/MVP.md`](docs/MVP.md) before your first change. If a change does not
serve the MVP cut line, it waits — however good it would look in a screenshot.

---

## Build and test

```console
$ cargo test
```

Before every commit, the trio. CI runs exactly this; do not make CI find it
first.

```console
$ cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
```

The MSRV is **1.85** (edition 2024), declared in [`Cargo.toml`](Cargo.toml) and
checked by its own CI job. It is not a matter of taste: Ubuntu 24.04's glibc
sets the floor for what we can link against, and the dependency set pins it
harder still. Raising it is a decision, not a convenience — say so in the PR.

Crates join the workspace only when they gain a real implementation, so a fresh
clone always builds. If a crate is not in `Cargo.toml`, it does not exist yet.

---

## Spec before code

Standing order S14: **specs and ADRs outrank code.** Nothing non-trivial is
implemented before its behaviour is written down. The full procedure lives in
[`docs/specs/README.md`](docs/specs/README.md); this is the working summary.

**What needs which artefact**

| You are… | Write |
|---|---|
| adding or changing a component's behaviour | a spec in `docs/specs/` |
| choosing between real alternatives — a protocol, a dependency, a model | an ADR in `docs/adr/`, and link it from the spec |
| fixing a bug, renaming a thing, tightening a doc comment | neither; just the fix, with a test |
| doing both — a new component *and* a decision | both, and cross-link them |

A spec answers *what must this do?* An ADR answers *why this and not that?* An
issue answers *who is doing which slice, and when?* A test answers *is it
actually true today?* If a spec starts arguing a decision, hand the argument to
an ADR and link it.

**The order of work**

```
spec ──▶ ADR (if a decision is involved) ──▶ failing happy-path tests
                                                      │
                                                      ▼
                              implementation ──▶ test names recorded in the spec
```

**A Draft spec may not be implemented against.** Draft means the open questions
are still open; code written against it will encode a guess. Move it to
Accepted first, or resolve the question and say so in the spec.

If implementing an Accepted spec proves the spec wrong — and it will happen —
correct the spec **in the same commit** as the code. A spec that has silently
diverged from its implementation is worse than no spec, because it is trusted.

**Writing happy-path tests from acceptance criteria**

1. Read the spec's *Acceptance criteria*. Each one is a single happy path.
2. Write one test per criterion, named as a sentence about behaviour:
   `undo_restores_the_exact_previous_ledger`, not `test_undo_2`.
3. Run them. **Watch them fail.** A test that has never failed has not been
   shown to test anything.
4. Implement until they pass, and no further.
5. Write the passing test names back into the spec's acceptance table, so the
   spec always says how to check itself.

Minimal means minimal: cover the intended path, not a matrix of edge cases.
Edge cases earn their tests when they turn up as bugs.

**The standing exception.** Layout geometry and colour maths get *invariant*
tests, not example tests — exact tiling across many sizes and window counts,
hue and ordering preserved across the whole contrast range. Example tests pass
happily while those algorithms are subtly wrong, and that is precisely the bug
class that reaches users as cracks between tiles and washed-out accents.

---

## House style

- **Comments explain why, never what.** Match the density of the surrounding
  file. `// bump the index` is noise; `// summon inserts after the focused
  window so a new terminal lands beside its parent, not at the end` is the
  reason someone will need in a year.
- **Every non-trivial function gets a test.** Layout and colour maths get
  invariant tests, as above.
- **No colour outside `palette.toml`.** Not in a template, not in a default, not
  "just for the placeholder". If a value you need is missing, add it to
  `palette.toml` and regenerate. See
  [ADR 0005](docs/adr/0005-palette-toml-single-source.md).
- **The ledger is the truth.** Window positions are never stored, only
  projected. A patch that caches a rectangle is a patch that breaks undo. See
  [ADR 0001](docs/adr/0001-ledger-as-single-source-of-truth.md).
- **Frame budgets are gates, not goals.** The numbers in
  [ARCHITECTURE §4](docs/ARCHITECTURE.md#4-what-robust-and-snappy-mean-here) —
  4 ms to new geometry, 8 ms to a bar redraw, 900 ms to a usable session — are
  pass/fail. A change that misses one is a regression, not a trade-off. Say in
  the PR which budget your change touches and how you know.
- **Prefer a proven component to a rewrite, behind a seam.** yazi, btop,
  starship, niri and fuzzel are used rather than reimplemented, each behind a
  trait or a generated config so it can be retired without touching its
  callers. A rewrite needs a written reason. See
  [ADR 0007](docs/adr/0007-reuse-yazi-btop-starship.md).
- **No animation.** v1 is motionless by design. State changes are instant.
- **British spelling** in prose: *colour*, *behaviour*, *licence* (noun).
  Identifiers keep whatever the ecosystem uses — `color::Oklab` stays.

## When you find a new way to break

Add a row to [`docs/PITFALLS.md`](docs/PITFALLS.md): what the user sees, what
helm does about it, and — the important column — the **guard**, the named test
that would fail loudly if the mitigation regressed. A bug we fixed but did not
write down is a bug we will ship again.

## Architectural changes need an ADR

Anything that changes a contract, a dependency with a real alternative, a wire
format, or the shape of a crate, gets an ADR in [`docs/adr/`](docs/adr/):
context, the decision, the alternatives actually considered, the consequences,
and the honest **reversal cost**. Number sequentially; never reuse a number.

A decision that turns out wrong gets a *new* ADR that supersedes the old one.
Do not quietly edit history — the reasoning is the point, including the
reasoning that did not survive.

## Things that need a person, not a patch

Hardware you do not have, a licence call, a security posture, a design
trade-off with no obviously right answer: label the issue **`needs-human`**,
state plainly *what decision is needed and what the options are*, and move on
to something else. Do not guess and proceed. Open `needs-human` questions are
listed on the front page of the [README](README.md#needs-a-human) so they are
never buried.

---

## Commits

One logical change per commit. The subject line says what, the body says why.

```
<scope>: <what changed, lower case, no full stop>

Why the change was needed, and anything a reader would otherwise have to
reconstruct from the diff. Wrap at 72 columns.

Closes #42
```

`<scope>` is the crate (`helm-core`, `helm-theme`), or `docs`, `ci`,
`packaging`, `configs`, `design`. Recent history is the reference:

```
helm-core: ledger, layout projection, palette and IPC contracts
docs: architecture, MVP cut line, failure register and operational memory
```

Close issues from commit messages (`Closes #N`), not by hand — standing order
S2 makes GitHub issues the project plan, and a commit that closes its issue
keeps the plan honest without a second step.

Never push a red build. If a change cannot be made green, revert it and reopen
the issue with what was learned.

## Pull requests

Use the [template](.github/PULL_REQUEST_TEMPLATE.md); it is the house rules as
a checklist. In particular, say which spec the change implements, or say
plainly that it is trivial enough not to need one.

## How work gets picked up

Issues, milestones and labels are the project plan — not a scratch file, not a
`TODO` comment. Every unit of work is an issue and every issue belongs to a
milestone.

Much of the routine work here is done by an autonomous agentic loop, whose
protocol is written down in
[`.claude/memory/40-loop.md`](.claude/memory/40-loop.md). Each iteration takes
the highest-priority *unblocked* issue in the earliest open milestone —
`blocked-others` › `bug` › `mvp` › everything else — restates its acceptance
criteria, implements the smallest change that satisfies them, runs the trio,
commits with `Closes #N`, and repeats. It skips anything labelled
`needs-human` and says so.

Two consequences worth knowing before you file an issue:

- **Acceptance criteria are the interface.** An issue whose criteria are
  ambiguous gets a question asked on it and is set aside, not guessed at. Write
  them as things that can be observed to be true.
- **The loop stops rather than improvising.** Anything needing hardware,
  credentials, a licence call or a design judgement is labelled `needs-human`
  with the options laid out, and left for a person.

Human contributors are welcome to take any issue that is not already assigned;
say so on the issue so the loop skips it.

## Licence

By contributing you agree that your contribution is dual-licensed under
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at the user's option, with
no additional terms. Everyone taking part is held to the
[Code of Conduct](CODE_OF_CONDUCT.md).
