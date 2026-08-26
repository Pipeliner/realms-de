## What this changes

<!-- One paragraph. What is different after this lands, and why it was needed.
     The diff already says what changed; say why. -->

Closes #

## Spec and decisions

<!-- Standing order S14: behaviour is written down before it is written in Rust.
     Link the spec this implements, or say plainly that the change is trivial
     enough not to need one — a bug fix, a rename, a doc correction. -->

- Spec:
- ADR (if this change involves a decision with real alternatives):

## Checklist

**The trio** — CI runs exactly this, so it should not be the one to find out.

- [ ] `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test` runs clean.

**Spec-first**

- [ ] A spec exists and is linked above, **or** this change is trivial enough not to need one.
- [ ] The spec was **Accepted**, not Draft, before I implemented against it.
- [ ] Happy-path tests were written from the spec's acceptance criteria **first**, and I watched them fail before implementing.
- [ ] The spec's acceptance criteria now name the passing test functions.
- [ ] If implementing proved the spec wrong, the spec is corrected **in this same commit**.

**Tests**

- [ ] A test was added or changed, named as a sentence about behaviour rather than `test_thing_2`.
- [ ] Layout geometry or colour maths touched? Then it has **invariant** tests, not example tests — exact tiling across sizes and counts, hue preserved across the contrast range.

**The rules that override taste**

- [ ] The ledger is still the truth: no window position is stored, only projected. (ADR 0001)
- [ ] `palette.toml` is untouched, **or** it changed deliberately and the relevant ADR and templates were updated with it. No colour literal was added anywhere else. (ADR 0005)
- [ ] No animation, transition, blur, shadow or rounded corner was introduced. (ADR 0009)

**Budgets**

- [ ] I considered which frame budget in [ARCHITECTURE §4](../docs/ARCHITECTURE.md) this change touches, and it still holds. Which one, and how I know:

  <!-- e.g. "key press → new geometry (<4 ms): the added work is O(1) per window
       and runs off the projection path" — or "none: docs only" -->

**Docs**

- [ ] If this uncovered a new way for helm to break, [docs/PITFALLS.md](../docs/PITFALLS.md) has a new row: what the user sees, what helm does about it, and the **guard** that would fail if the mitigation regressed.
- [ ] Anything a newcomer would now find stale — README, ARCHITECTURE, INTERFACES, ROADMAP — has been updated.

**Judgement**

- [ ] Nothing in here is a quiet guess at something that needed a person. Anything that did is labelled `needs-human`, with the options stated, and is listed on the [README front page](../README.md#needs-a-human).

## Anything a reviewer should look at twice

<!-- The part you are least sure about. Naming it saves everyone an afternoon. -->
