# SPEC 0016 — README truthfulness snapshot

- **Status:** Accepted (2026-08-30)
- **Milestone:** M0
- **Issue:** [#8](https://github.com/Pipeliner/realms-de/issues/8)
- **Decisions:** Standing orders S3, S10 and S15
- **Supersedes / Superseded by:** —

## Purpose

Keep the front page useful to a new contributor without claiming unfinished
work is absent or that GitHub state is permanently frozen. The README must make
the project’s evidence and decisions visible without turning a stale summary
into a false product claim.

## Scope

**In:** the first-screen description, the three repository rules, the M0 status
snapshot, the pre-alpha delivery boundary, the `needs-human` index, and the
repository map in `README.md`.

**Out:** changing milestones, issue labels, product decisions, package policy,
or querying GitHub from CI. The issue tracker remains the live work graph; this
is a reviewable repository snapshot of it. Nix ShellCheck of the two check
scripts is defense in depth, not a second documentation gate.

## Behaviour

### 1. First screen and rules

Before the first README divider, helm is identified as a keyboard-first,
gapless-tiling, Rust-first Wayland desktop environment with zero animations and
one palette file. The `What makes it different` section uses these exact rule
headings and links their governing ADRs:

1. `The ledger is the truth.` — ADR 0001.
2. `No colour outside palette.toml.` — ADR 0005.
3. `Snappy is a number.` — ADR 0009.

The river explanation remains explicit that helm is river’s window manager, not
a client of a compositor.

### 2. Status, delivery and map truth

The README states that the roadmap marks M0 **in progress** and that M3 is the
MVP. It distinguishes present pre-alpha artifacts from a usable desktop:
`helm-core` and `helm-theme` have source and tests; tracked session entry,
wrapper, systemd-unit, portal-configuration, Nix-module and native-package
assets exist; none provides a working log-in session because `helm-wm` is not
implemented. The missing bar also prevents a usable desktop, but it is not the
session wrapper’s abort condition.

For this pre-alpha snapshot, absence of the `crates/helm-session` and
`crates/helm-bar` implementation manifests (and of an implicit
`src/bin/helm-wm.rs`) is the checked repository evidence for that `helm-wm`
claim. `helm-theme` source/test evidence consists of its manifest, `src/lib.rs`
and `src/theme.rs` containing Rust test evidence. The Nix module is
`packaging/nix/nixos-module.nix`; native package-definition evidence is
`packaging/debian/control` and `packaging/fedora/helm.spec`.

The README must not describe those tracked assets as “Planned. Not started” or
say that no session entry exists. Its tree representation resolves to each of
these checked-in paths:
`crates/`, `configs/`, `configs/templates/`, `configs/portal/`, `packaging/`,
`packaging/nix/`, `packaging/debian/`, `packaging/fedora/`, root `flake.nix`,
`docs/`, `design/`, `.claude/`, and `palette.toml`.

### 3. `needs-human` snapshot

The `Needs a human` section is a dated snapshot of this GitHub query at
`2026-08-30T06:18:36Z`:

```text
repo:Pipeliner/realms-de is:issue is:open label:needs-human
```

It contains exactly one Markdown table row per snapshot issue. Each row has
the structure `| [#number — exact GitHub title](exact issue URL) | factual,
nonempty blocker |`, for exactly these issues: `#16`, `#17`, `#23`, `#24`,
`#25`, `#30`, `#35`, `#132`, `#133`, `#134`, `#135`, `#166`, and `#168`.

Every blocker is a short consequence already stated by the linked issue. It
does not invent an option, recommendation, priority, owner or deadline. Closed
#34 is not an open question: its font-distribution decision is recorded in ADR
0012. The section states that the live label may change after the snapshot and
links to GitHub for current state.

When a `needs-human` issue is opened, closed, or gains/loses that label, refresh
the snapshot in the same documentation change; never silently leave it claiming
to be live.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the README before its first divider, when a visitor reads it, then it contains the five-part identity; given the rule section, it contains the three exact headings with ADR 0001/0005/0009 links. | `docs/test-readme-truth-snapshot.sh` — `intro-and-rules` |
| A2 | Given README status and map sections, when checked against tracked paths and `docs/ROADMAP.md`, then the M0-in-progress/M3-MVP wording, present pre-alpha assets, absent `helm-wm`, and all named map paths are truthful. | `docs/test-readme-truth-snapshot.sh` — `artifact-truth` |
| A3 | Given the `2026-08-30T06:18:36Z` snapshot, when each `Needs a human` table row is checked, then it binds one exact issue number, URL and title to a nonempty factual blocker; exactly the 13 accepted rows exist and #34 does not. | `docs/test-readme-truth-snapshot.sh` — `needs-human-snapshot` |
| A4 | Given the documentation CI job, when it runs on a pull request or push, then uncommented fixture and production-check commands run in the `docs` job without a network call. | `docs/test-readme-truth-snapshot.sh` — `workflow-invocation` |

## Failure modes

Stale README claims violate standing order S15. The local check fails before
documentation CI if the identity/rules leave their required sections, an
accepted snapshot row is omitted or malformed, #34 is presented as unresolved,
a required artifact/map path is denied or missing, or a check command is not
load-bearing in the docs job.

## Open questions

None. This specification restates accepted repository facts and a dated issue
snapshot; it does not decide unresolved product semantics.
