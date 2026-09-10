# Task 4 report: `realmctl theme apply`

## Delivery

- `theme apply --config-root PATH` now uses the established config-root
  precedence and calls `realm_theme::apply` in-process.
- `Committed` prints `generation <id> selected for future launches` and exits
  0. `CommittedWithCleanupPending` prints the same success line, warns on
  stderr with a JSON-escaped cause, and exits 0.
- `OutcomeAmbiguous` emits no stdout, reports the candidate as unconfirmed
  with a JSON-escaped cause, and exits 6. It does not retry, recover, use IPC,
  or notify/reload a session.
- The CLI result mapping is a small internal report seam: `run_apply` is still
  the only public command path and performs the actual stdout/stderr writes.

## TDD evidence

1. Added `apply_reports_selected_future_generation_without_reload_or_session`.
2. `cargo test -p realm-ctl apply_` initially failed because `Apply` returned
   success without running the generation publication or reporting its result.
3. Added only the direct `realm_theme::apply` outcome mapping and reran the
   focused test successfully.
4. Added `public_apply_fatal_candidate_preserves_current_generation`; it passed
   immediately against the existing public implementation, proving the
   requested pointer-preservation behavior was already present.
5. Added cleanup-pending and ambiguous result tests first. They initially did
   not compile because the result-reporting seam did not exist; after extracting
   the local report value, both assert exit code, stdout discipline, and
   JSON-escaped stderr causes directly.

## Final verification

- `cargo fmt --all -- --check`
- `cargo clippy -p realm-ctl --all-targets -- -D warnings`
- `cargo test -p realm-ctl` — 12 test instances passed, 0 failed (the two
  internal reporting tests are also discovered through the integration test's
  existing source-module harness).
- `cargo test -p realm-theme public_apply_fatal_candidate_preserves_current_generation`
  — 1 passed, 0 failed.
- `git diff --check`
