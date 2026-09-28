# SPEC 0016 — README truthfulness snapshot

- **Status:** Accepted (2026-08-30; live-VM evidence amendment 2026-09-13; native-VM snapshot 2026-09-27; Fedora journey refresh 2026-09-28)
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

Those historical captures do not establish a completed M3 desktop or physical
hardware support. Separately, run 36327811527 at merge revision
`29e69684d1ff1d45629d7cdcc0b3d8b8ae6996dc` passed both native-package builds and
Ubuntu 24.04/Fedora graphical VM jobs. Link that run and distinguish successful
jobs from the overall failed run (Fedora source guard and Nix X11 checks).
Physical hardware and full MVP acceptance remain pending; do not claim later
browser, window-control or relogin probes passed from this earlier evidence.

Fedora-only refresh, verified 2026-09-28: run 36358017601 job 108735431595
passed the installed Fedora 44 native journey at tested merge
`11538ab5bd05a1018f48216f661990987c259ce2` (source `9f0aa56`). Replace only
the Fedora trial with package artifact 10944903392; retain Ubuntu artifact
10935181555 from the older run. Link runtime evidence artifact 10945836471.
The Fedora proof covers manual locking, real 300/600-second idle timing and
PAM unlock, controls/help, Foot/Zsh/private-tool activation and completion cache,
generation A-to-B relogin, WM crash recovery, portal Settings/cancel/capture,
and Firefox two delivered PNG frames with advancing presentedFrames and ended
track. Recovery screenshots were visually checked at 1280x800. It does not
prove native GTK/Qt CSS consumption, physical backlight/hardware or full MVP;
the overall run failed its Nix GTK check and Ubuntu partial idle-lock rendering.
The README must name both failures. Suspend remains post-MVP. Keep these
limits explicit rather than presenting the newer Fedora evidence as Ubuntu or
all-platform verification. Preserve all historical screenshot provenance.

For this pre-alpha snapshot, the `crates/realm-session` manifest, library and
backend contract, runtime owner, production `src/bin/realm-wm.rs` entrypoint,
real-socket runtime fixture, and installed NixOS QEMU capture are checked
evidence for the implemented daemon. README status must name the verified VM
boundary without broadening it to physical hardware. Separate native evidence
must identify its own run and revision. For
`realm-bar`, its manifest, binary entrypoint, render contract tests, and paired
which-key/grimoire captures are required artifacts. README status must call it
implemented and verified in that same bounded VM environment.
The `crates/realm-ctl` manifest and binary source are checked evidence for the
implemented theme commands and bounded `doctor`; the README must not imply
that the full M3 control surface is complete.
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
`2026-09-28`:

```text
repo:Pipeliner/realms-de is:issue is:open label:needs-human
```

It reports no open `needs-human` issues. Issue #25's GTK target-location
decision is resolved by Accepted SPEC 0030's named generation profiles; its
remaining toolkit runtime and fidelity checks stay open, not human-blocked.
The section must not retain an issue table or imply those checks passed.

Every blocker is a short consequence already stated by the linked issue. It
does not invent an option, recommendation, priority, owner or deadline. Closed
#34 is not an open question: its font-distribution decision is recorded in ADR
0012. The section states that the live label may change after the snapshot and
links to GitHub for current state.

When a `needs-human` issue is opened, closed, or gains/loses that label, refresh
the snapshot in the same documentation change; never silently leave it claiming
to be live.

Keep README current whenever shipped behavior, try-out instructions or verified
delivery evidence changes. Refresh the README, this dated evidence contract and
its fixtures together when their claims change; retain explicit limitations and
do not present a successful job as a successful whole run.
The current try-out instructions link the exact per-platform artifacts above,
disclose their 2026-12-26 expiry and possible earlier removal,
and warn readers to preserve their existing desktop/display manager. These are
CI trial artifacts, not releases or a completed MVP certification.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the README before its first divider, when a visitor reads it, then it contains the five-part identity and both inspected VM captures; their caption identifies NixOS QEMU and River 0.4.8, links the original provenance and explicit review correction, reports the PNG-IHDR-derived 1280x800 dimensions, and makes no hardware or final-theme claim; given the rule section, it contains the three exact headings with ADR 0001/0005/0009 links. | `docs/test-readme-truth-snapshot.sh` — `intro-and-rules`, `capture-evidence` |
| A2 | Given README status and map sections, when checked against tracked paths and `docs/ROADMAP.md`, then the M0-in-progress/M3-MVP wording, installed-NixOS-VM-verified `realm-wm` and `realm-bar`, separately linked native VM jobs and tested revision, failed overall-run boundary, remaining hardware/full-MVP boundary, present pre-alpha assets, and all named map paths are truthful. | `docs/test-readme-truth-snapshot.sh` — `artifact-truth` |
| A3 | Given the `2026-09-28` snapshot, when `Needs a human` is checked, then it states no open human-decision issues, links the live label, retains the unresolved runtime/fidelity boundary and has no obsolete issue rows. | `docs/test-readme-truth-snapshot.sh` — `needs-human-snapshot` |
| A4 | Given the documentation CI job, when it runs on a pull request or push, then uncommented fixture and production-check commands run in the `docs` job without a network call. | `docs/test-readme-truth-snapshot.sh` — `workflow-invocation` |

## Failure modes

Stale README claims violate standing order S15. The local check fails before
documentation CI if the identity/rules leave their required sections, an
obsolete human-decision row returns or the unresolved runtime boundary is lost,
a required artifact/map path is denied or missing, or a check command is not
load-bearing in the docs job.

## Open questions

None. This specification restates accepted repository facts and a dated issue
snapshot; it does not decide unresolved product semantics.
