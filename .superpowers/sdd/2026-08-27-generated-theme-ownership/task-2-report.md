# Task 2 — Owned outputs and absent-only GTK activation

## Scope delivered

- Redirected every shipped template target below
  `$XDG_CONFIG_HOME/helm/generated/<tool>/` without changing its source or
  reload identity.
- Added post-commit activation creation through held directory descriptors.
  `CREATE|EXCL|NOFOLLOW` creates a missing GTK `gtk.css` with exactly its
  declared import while every existing path remains untouched.
- Kept activation files outside `Applied::written`; an activation failure is
  returned after generated commits and before reload, without claiming a user
  file was modified or widening #22's publication boundary.
- Rejected unsafe activation paths before rendering or writing.
- Added regressions for activation-create failure ordering and unsafe
  activation metadata. Task 1's ownership tests were not weakened.

## Review

Independent review found and rechecked one pathname-cleanup race. The final
implementation never unlinks an activation pathname after exclusive creation,
because that entry is user-owned and could have been replaced concurrently.
The re-review reported no remaining Critical or Important issues and marked
Task 2 ready.

## Verification

- `cargo fmt --check` — passes.
- `cargo test -p helm-theme` — 32 passed.
- `cargo clippy -p helm-theme --all-targets --all-features -- -D warnings` —
  passes.
- `git diff --check` — passes.
- The crate retains `#![forbid(unsafe_code)]`; the workspace manifest retains
  `rust-version = "1.85"`.

## Next task

Task 3 should add the ownership/activation acceptance rows and stable doctor
diagnostic contract to SPEC 0002, run the full workspace verification, push,
and report the explicit #23 dependency on issue #33.
