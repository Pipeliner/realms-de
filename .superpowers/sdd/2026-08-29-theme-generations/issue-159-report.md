# Issue 159 specification reconciliation report

## Proposed scope

- Make SPEC 0011 the governing accepted contract for supported theme apply and
  diff: apply publishes a fresh immutable generation for future launches only,
  pointer switches never reload existing processes, and diff compares candidate
  normalized outputs with a fully validated `current` generation without
  initializing, recovering, leasing, publishing, or otherwise mutating control
  state.
- Explicitly supersede SPEC 0002's mutable target writes, byte-identical no-op
  optimization, `Applied { written, unchanged, reloaded }`, and reload fan-out
  for the supported path while retaining its palette, derivation, rendering,
  lint, placeholder, and normalized-output guarantees.
- Align SPEC 0006 and INTERFACES.md with generation publication outcomes,
  session-independent apply/diff, and added/removed/byte-different diff results.
- Correct ADR 0017's #22 guard: any later live upgrade must be a
  generation-aware owned-process protocol and cannot restore reload on pointer
  switch. Keep live upgrade, no-op optimization, and wire compatibility out of
  scope.
- Update only the leading crate-level rustdoc in
  `crates/helm-theme/src/lib.rs` as a narrow public-documentation exception;
  do not change executable Rust behavior, public APIs, or code tests.

## Completion

Completed the scoped contract reconciliation:

- SPEC 0011 now normatively defines generation-aware read-only diff against a
  fully validated `current` generation, including sorted added/removed/
  byte-different results and an explicit ban on control initialization,
  recovery, leases, GC, publication, writes, pointer changes, and reloads. G10
  and G11 cover diff and future-launch-only pointer commits.
- SPEC 0002 explicitly retires mutable target publication, equality/no-op
  reporting, `Applied` lists, and reload fan-out for the supported path while
  retaining palette initialization, derivation, lint, rendering, placeholder,
  normalization, and containment guarantees.
- SPEC 0006 and INTERFACES.md now expose session-independent generation apply,
  generation-aware diff, publication outcomes, and no post-commit notification.
- ADR 0017 limits #22 to a future generation-aware owned-process upgrade
  protocol and makes no-op and wire compatibility explicitly non-contractual.
- Only the leading crate-level rustdoc in `crates/helm-theme/src/lib.rs` changed;
  executable Rust behavior and APIs were not edited.

Verification run from the assigned worktree:

- `cargo fmt --all --check` — passed.
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` — passed.
- `cargo test -p helm-theme --doc` — passed (0 doctests, 0 failures).
- `git diff --check` — passed.

No code tests were added or changed, as required. The executable legacy mutable
APIs remain for the later implementation task; every authoritative document
updated here identifies them as retired rather than a supported alternative.
