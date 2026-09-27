# ADR 0023 — Reuse-first bar and session-scoped themes

- **Status:** Accepted (2026-09-27)
- **Decider:** repository owner, approving both recommendations in the reuse assessment
- **Supersedes:** ADR 0008's mandatory custom renderer and categorical GTK
  exclusion; ADRs 0017/0018's per-launch theme ownership requirements for MVP.
  Partially supersedes ADRs 0003/0005/0009/0011 and incorporated activation
  clauses in other documents as explicitly scoped below.
- **Contract:** [SPEC 0030](../specs/0030-reuse-first-session-theme-mvp.md)
- **Evidence:** [reuse assessment](../research/2026-09-27-reuse-first-mvp.md)

## Decision

1. Compare configured Waybar plus a narrow Realm state adapter with the existing
   bar before further custom bar internals are developed. GTK is not disqualified
   by predicted overhead. This approves the experiment, not an untested switch.
   Keep working chord discovery and the existing bar until replacement evidence
   exists. Only one implementation is the shipped default.
2. Select the theme once at graphical login. Theme apply prepares the selection
   for the next login; it does not change current-session launches or running
   applications. No per-launch generation selection, transferable lease,
   launch-gate, descendant-drain or generation-aware live-upgrade machinery is
   required for MVP. Ordinary application activation is permitted.
3. Reuse ordinary systemd lifecycle management; evaluate UWSM only as a possible
   replacement for outer session plumbing, never as another supervisor layered
   over the same ownership. Its adoption is not an MVP gate.
4. Keep palette validation, coherent rendering, user-file preservation,
   truthful diagnostics, exact Realm window semantics, target distributions,
   portals and approved lock/idle behavior. Removing process-provenance promises
   does not permit partial config publication or destructive migration.
5. Native replacements for working upstream applications and a custom compositor
   are shelved, not inevitable milestones. Restart such work only for a concrete
   unmet product need. Retire the task of replacing dependencies for its own sake.

## Tradeoffs and limits

Users must log out and back in to see a newly applied theme. Processes already
running outside the new session, including single-instance applications, may
retain their previous appearance; Realm does not claim control over them.
The accepted bar functionality remains required, but renderer-specific damage
algorithms and exact internal timer counts do not constrain the comparison.
Existing performance targets are measured, not replaced with an assertion that
GTK is necessarily fast enough. Any missed target must be reported before the
replacement ships; there is no silent performance waiver.

Historical contracts and tests remain available as implementation evidence.
This decision does not assert the code has migrated, remove safeguards from the
old code path, or authorize deletion of active theme data. Existing PRs must be
reconciled against SPEC 0030 before merge.

## Guard and reversal

SPEC 0030 defines login/apply/relogin, failed-publication, ordinary activation,
bar functionality and comparative performance evidence. Those tests are planned,
not green by declaration. Reintroducing per-launch theme provenance or replacing
the compositor requires a new accepted decision, not resurrection of an old
issue. Reversal cost is medium: launcher/session and consumer configuration.
