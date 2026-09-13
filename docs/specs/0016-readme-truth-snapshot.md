# SPEC 0016 — README truthfulness snapshot

- **Status:** Accepted (2026-08-30; live-VM evidence amendment 2026-09-13)
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

Before the first README divider, realm is identified as a keyboard-first,
gapless-tiling, Rust-first Wayland desktop environment with zero animations and
one palette file. Its hero is a visually inspected compositor framebuffer from
the installed Realm package running under River 0.4.8 in the NixOS QEMU
reference VM, and the README also shows the paired grimoire capture. The caption
identifies that environment as a VM and does not imply physical-hardware,
native-package-installation or full-MVP verification. It links both the original
capture provenance and the review correction kept beside the images.

Reported capture dimensions come from each PNG's IHDR, not the resolution
requested from the VM configuration. The original provenance remains preserved
verbatim even when it contains incorrect requested-resolution metadata; a
linked correction records the observed dimensions and explains the discrepancy
without rewriting the original evidence.

The `What makes it different` section uses these exact rule headings and links
their governing ADRs:

1. `The ledger is the truth.` — ADR 0001.
2. `No colour outside palette.toml.` — ADR 0005.
3. `Snappy is a number.` — ADR 0009.

The river explanation remains explicit that realm is river’s window manager, not
a client of a compositor.

### 2. Status, delivery and map truth

The README states that the roadmap marks M0 **in progress** and that M3 is the
MVP. It distinguishes the installed NixOS reference-VM proof from a completed
or generally usable desktop:
`realm-core` and `realm-theme` have source and tests; tracked session entry,
wrapper, systemd-unit, portal-configuration, Nix-module and native-package
assets exist; `realmctl theme apply`, `theme lint`, and `theme diff` are
implemented; the `realm-session` crate contains the accepted `WmBackend`
contract, real River adapter, daemon binary, and dispatch loop. The NixOS QEMU
reference VM verifies the installed session entry reaches a live River
compositor, the daemon and bar run from the installed package, three ordinary
Wayland application windows are managed, and the which-key and grimoire
surfaces render. The captures show foot with the current unthemed/default-font
presentation, including visible ASCII glyph fallback; the README must not
present that as the final themed desktop.

That proof does not establish a completed M3 desktop, physical-hardware support,
native Debian or Fedora installation, or functional portal integration.

For this pre-alpha snapshot, the `crates/realm-session` manifest, library and
backend contract, runtime owner, production `src/bin/realm-wm.rs` entrypoint,
real-socket runtime fixture, and installed NixOS QEMU capture are checked
evidence for the implemented daemon. README status must name the verified VM
boundary without broadening it to physical hardware or native packages. For
`realm-bar`, its manifest, binary entrypoint, render contract tests, and paired
which-key/grimoire captures are required artifacts. README status must call it
implemented and verified in that same bounded VM environment.
The `crates/realm-ctl` manifest and binary source are checked evidence for the
implemented theme commands; the README must not imply that the full M3 control
surface or `doctor` exists.
`realm-theme` source/test evidence consists of its manifest, `src/lib.rs` and
`src/theme.rs` containing Rust test evidence. The Nix module is
`packaging/nix/nixos-module.nix`; native package-definition evidence is
`packaging/debian/control` and `packaging/fedora/realm.spec`.

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
| A1 | Given the README before its first divider, when a visitor reads it, then it contains the five-part identity and both inspected VM captures; their caption identifies NixOS QEMU and River 0.4.8, links the original provenance and explicit review correction, reports the PNG-IHDR-derived 1280x800 dimensions, and makes no hardware or final-theme claim; given the rule section, it contains the three exact headings with ADR 0001/0005/0009 links. | `docs/test-readme-truth-snapshot.sh` — `intro-and-rules`, `capture-evidence` |
| A2 | Given README status and map sections, when checked against tracked paths and `docs/ROADMAP.md`, then the M0-in-progress/M3-MVP wording, installed-NixOS-VM-verified `realm-wm` and `realm-bar`, remaining hardware/native-package/portal boundary, present pre-alpha assets, and all named map paths are truthful. | `docs/test-readme-truth-snapshot.sh` — `artifact-truth` |
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
