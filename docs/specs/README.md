# Specs

helm is built spec-first (standing order S14). Nothing non-trivial is
implemented before its behaviour is written down here, and the happy-path tests
come from this directory before the code does.

## What belongs where

| Artefact | Answers | Lives in |
|---|---|---|
| **Spec** | *What must this component do?* | `docs/specs/NNNN-name.md` |
| **ADR** | *Why did we choose this over the alternatives?* | `docs/adr/NNNN-name.md` |
| **Issue** | *Who is doing which slice of it, and when?* | GitHub |
| **Test** | *Is it actually true today?* | the crate |

A spec that argues a decision should hand that argument to an ADR and link it. An
ADR that specifies behaviour should hand that behaviour to a spec and link it.

## Lifecycle

```
  Draft ──▶ Accepted ──▶ Implemented ──▶ (Superseded)
    │           │             │
    │           │             └─ acceptance criteria now name real test functions
    │           └─ happy-path tests written and failing
    └─ open questions still unresolved; may not be implemented against
```

A spec at **Draft** must not be implemented against. If implementing an
**Accepted** spec proves it wrong, the spec is corrected **in the same commit**
as the code. A spec that has silently diverged from its implementation is worse
than no spec at all, because it is trusted.

## Minimal TDD, concretely

1. Read the spec's **Acceptance criteria**. Each is one happy path.
2. Write one test per criterion. Run them. **Watch them fail.** A test that has
   never failed has not been shown to test anything.
3. Implement until they pass. Nothing more.
4. Write the passing test names back into the spec's acceptance table, so the
   spec always says how to check itself.

Minimal means minimal: cover the intended path, not an edge-case matrix. Edge
cases earn their tests when they show up as bugs.

**The standing exception:** layout geometry and colour maths get *invariant*
tests rather than example tests — exact tiling at many sizes, hue preserved
across the whole contrast range. Example tests pass happily while those
algorithms are subtly wrong, which is precisely the bug class that shows up as
hairline cracks between tiles and washed-out accents.

## Numbering

Sequential, never reused. A superseded spec keeps its number and gains a
`Superseded by` header.

| Spec | Title | Status |
|---|---|---|
| [0001](0001-helm-core-contracts.md) | helm-core contracts | Implemented |
| [0002](0002-theme-pipeline.md) | Theme pipeline | Implemented |
| [0007](0007-control-socket-security.md) | Control-socket security amendment | Accepted |
| [0008](0008-agent-sdd-pilot-governance.md) | Local agent-SDD pilot governance | Accepted |
| [0009](0009-fedora-44-pre-alpha-baseline.md) | Fedora 44 pre-alpha packaging and CI baseline | Accepted |
| [0010](0010-packaged-helm-sdd-git-runtime.md) | Packaged `helm-sdd` Git runtime | Accepted |
| [0011](0011-theme-activation-generations.md) | Immutable theme activation generations | Accepted |
| [0012](0012-activation-launch-lifecycle.md) | Activation launch lifecycle | Accepted |
| [0013](0013-truthful-fresh-desktop-exec.md) | Truthful fresh desktop Exec launch | Accepted |
| [0016](0016-readme-truth-snapshot.md) | README truthfulness snapshot | Accepted |
| [0017](0017-contribution-templates.md) | Contribution templates | Accepted |
| [0018](0018-agent-sdd-pilot-procedures.md) | Local agent-SDD pilot procedures | Accepted |
| [0019](0019-ubuntu-versioned-rust-toolchain.md) | Ubuntu versioned Rust toolchain guard | Accepted |
| [0020](0020-helmctl-theme-json.md) | helmctl theme JSON output | Accepted |
| [0021](0021-github-publication-body-safety.md) | GitHub publication body safety | Accepted |
