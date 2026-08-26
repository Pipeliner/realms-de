# Operational memory

Durable knowledge about *how this repo is worked on*, kept separate from
`docs/`, which is knowledge about *what is being built*.

## Progressive exposure

`CLAUDE.md` is loaded on every session and is deliberately ~40 lines. It carries
an index, not content. Each file here is loaded only when the task calls for it.
That keeps the always-on context small while leaving the detail one hop away.

| File | Load when | Written by |
|---|---|---|
| `00-standing-orders.md` | Always — first thing | The user; transcribed verbatim in intent |
| `10-decisions.md` | Making or revisiting a design call | Whoever makes the call |
| `20-environment.md` | Building, testing, packaging | Whoever hits a toolchain fact |
| `30-gotchas.md` | Debugging | Whoever loses time to something avoidable |
| `40-loop.md` | Starting or ending a work session | The loop itself |

## Rules for writing memory

1. **Durable only.** "cargo 1.94 is installed" is durable. "The build is
   currently red" is not — that belongs in an issue.
2. **One fact, one line, with its consequence.** Not "we use OKLab" but
   "contrast is derived in OKLab because a `contrast()` filter rotates hues —
   don't reintroduce a filter."
3. **Append, don't rewrite.** When a fact goes stale, strike it and date the
   replacement, so the reasoning trail survives.
4. **If it belongs to the product, it goes in `docs/`.** Memory is about
   working, not about helm.
5. **Update at the moment of learning,** not at the end of the session.
