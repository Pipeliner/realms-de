# helm — working agreement

helm is a keyboard-first, gapless-tiling Wayland desktop environment. Rust-first,
zero animations, one palette file.

**Read first, always:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §1–§3 and
[`docs/MVP.md`](docs/MVP.md). If a change does not serve the MVP cut line, it
waits.

## Operational memory — load on demand

This file stays short on purpose. Load the memory file that matches what you are
about to do, and **update it when you learn something durable**.

| Before you… | Load |
|---|---|
| do anything at all | [`.claude/memory/00-standing-orders.md`](.claude/memory/00-standing-orders.md) — how the user wants this repo run |
| make or revisit a design decision | [`.claude/memory/10-decisions.md`](.claude/memory/10-decisions.md) + [`docs/adr/`](docs/adr/) |
| build, test, or touch the toolchain | [`.claude/memory/20-environment.md`](.claude/memory/20-environment.md) |
| debug something that smells familiar | [`.claude/memory/30-gotchas.md`](.claude/memory/30-gotchas.md) + [`docs/PITFALLS.md`](docs/PITFALLS.md) |
| pick up work / close it out | [`.claude/memory/40-loop.md`](.claude/memory/40-loop.md) — the agentic loop |
| write DE code specifically | the skills in [`.claude/skills/`](.claude/skills/) |

## The four rules that override taste

1. **Spec before code — hard stop.** Do not write, edit, or commit implementation
   code until its governing spec is **Accepted** and names the intended
   behaviour; then write the test, watch it fail, and only then implement.
   A bug fix must first amend its accepted spec with the regression criterion.
   An unresolved design question requires an ADR or `needs-human` decision,
   never a code guess. See standing order S14.
2. **The ledger is the truth.** Window positions are never stored, only
   projected. See ADR 0001.
3. **No colour outside `palette.toml`.** Everything themed is generated.
   See ADR 0005.
4. **Snappy is a number.** Frame budgets in `docs/ARCHITECTURE.md` §4 are gates,
   not aspirations.

## House style

- Comments explain *why*, never *what*. Match the density of the surrounding file.
- Happy paths are tested first, from the spec's acceptance criteria, and are
  watched to fail before anything is implemented. Keep it minimal — edge cases
  earn their tests when they turn up. Layout and colour maths are the standing
  exception: they get invariant tests, because example tests pass while the
  algorithm is subtly wrong.
- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` before
  every commit. CI runs the same thing; do not make CI find it first.
- Prefer a proven component over a rewrite, and put it behind a seam so it can
  be replaced. See ADR 0007.
