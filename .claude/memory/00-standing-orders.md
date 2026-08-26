# Standing orders

Durable instructions from the repo owner. These outlive any single session and
apply to every task in this repo unless the owner says otherwise. Newest
qualifications win; nothing here is deleted, only struck through and dated.

---

## S1 — Build the DE described in the handoff

`design/HANDOFF.md` plus the four `design/*.dc.html` prototypes are the brief.
`Desktop v3.dc.html` is canonical; v1/v2 are provenance only. The prototypes are
**design references, not production code** — recreate them in the real stack.
Fidelity is high: colours, spacing, type and copy are final unless an ADR
overrides them with a reason.

## S2 — Track progress in GitHub

Issues, milestones and labels are the project plan. Not a scratch file, not a
TODO comment. Every unit of work is an issue; every issue belongs to a
milestone. Close issues from commits.

## S3 — Mark what needs a human

Anything requiring judgement the repo cannot supply — physical hardware, a
licence call, a design trade-off with no obviously right answer, a security
decision — gets the **`needs-human`** label and a clear statement of *what
decision is needed and what the options are*. Never silently guess and move on.
Surface these in the README so they are not buried.

## S4 — Search for applicable skills before acting

Before starting a class of work, check for a skill that covers it: the local
`.claude/skills/`, the user's enabled skills, the org plugin catalog, **and
online skill directories**. If nothing fits, that is a signal to *write* the
skill — see S9.

## S5 — Build operational memory, with progressive exposure

Keep durable knowledge in `.claude/memory/`, indexed from a short `CLAUDE.md`.
Small always-on context; detail one hop away, loaded only when the task calls
for it. Update memory at the moment of learning.

## S6 — Target Nix/NixOS, Ubuntu and Fedora

All three are first-class and tested in CI. The Nix flake is the reference
build; deb and rpm are generated from the same metadata rather than maintained
in parallel. Anything else is best-effort.

## S7 — Robust and snappy, and avoid the common DE problems

Both words are tests, never adjectives: frame budgets in
`docs/ARCHITECTURE.md` §4 are gates. Known DE failure modes are catalogued in
`docs/PITFALLS.md`; every mitigation names the guard that would fail if it
regressed. Add a row whenever a new failure mode is found.

## S8 — Reuse proven components where they suit

yazi, btop, zsh+starship, niri, fuzzel and friends are used rather than
rewritten — but always behind a seam (a trait, a config, a generated file) so a
stopgap can be retired without touching its callers. A rewrite needs a written
reason.

## S9 — Set up and run an agentic loop

Work proceeds in autonomous iterations: pick the highest-priority open issue,
implement, test, commit, close, repeat. The loop's protocol lives in
`.claude/memory/40-loop.md`. It must be safe to interrupt at any point and it
must never push a red build.

## S10 — Make the repo beautiful

The repo is part of the product. README, docs, diagrams, commit messages and
code comments are held to the same standard as the DE's visual design. A
newcomer should be able to tell what helm is, and why it is built this way,
within a minute of landing on the front page.

## S11 — Use subagents heavily wherever it makes sense

Fan out parallel, independent work to subagents: research sweeps, per-crate
implementation, docs, packaging, review. Partition by directory so parallel
agents never contend on the same files. Keep the conclusions, not the file
dumps. Serialise anything that shares a contract until the contract is committed.

## S12 — Architecture before implementation; MVP before everything

Establish the architecture first, and treat it as provisional: written down,
argued with, revisable — never set in stone, never implicit. Then prioritise
strictly towards the MVP cut line in `docs/MVP.md`, and only then write code.
Superseded work is fine; unplanned work is not.

## S14 — Spec-driven development, with minimal TDD

**Specs and ADRs outrank code.** Nothing non-trivial is implemented before its
behaviour is written down: a spec in `docs/specs/` for a component or feature, an
ADR in `docs/adr/` for a decision with alternatives. If implementing reveals the
spec was wrong, fix the spec in the same commit — code that has silently
diverged from its spec is worse than no spec.

**Minimal TDD.** Once a spec exists, write the **happy-path tests first**, from
the spec's acceptance criteria, and watch them fail. Then implement until they
pass. Minimal means exactly that: cover the intended path, not a matrix of
edge cases. Edge cases earn their tests when they turn up as bugs or as
invariants worth defending (layout tiling and colour maths are the standing
exceptions — those get invariant tests because example tests pass while the
algorithm is subtly wrong).

The order is: **spec → ADR if a decision is involved → failing happy-path tests
→ implementation → the test names recorded back in the spec.**

## S15 — Truthful presentation

The repository must never imply capability it does not have.

- **Imagery is labelled for what it is.** Anything not captured from running
  software is marked *concept rendering* or *diagram*, in the caption, at the
  point of use — not in a footnote. Concept art is replaced with real captures
  as soon as there is something to capture, and never earlier.
- **AI authorship is disclosed** on the front page: what an agent wrote, what a
  human wrote, and who decided the decisions. A reader should not have to infer
  it from commit trailers.
- **Every claim is checkable.** If the README says a thing is tested, it names
  the test. If a component does not exist, it says so in those words. Status
  tables list what is *not* done as prominently as what is.
- **No aspirational tense.** "helm does X" means it does X today. Anything else
  is written as planned, with its milestone.

This is not modesty. An unverifiable claim on a front page devalues the
verifiable ones next to it.

## S13 — Preserve instructions in reusable form

An instruction given once is written here, generalised past its immediate
occasion, so it applies to future work without being repeated. This file is the
place; `CLAUDE.md` indexes it.
