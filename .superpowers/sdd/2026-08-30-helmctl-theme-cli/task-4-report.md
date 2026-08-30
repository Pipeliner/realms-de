# Task 4 report: `helmctl theme apply`

## Delivery

- `theme apply --config-root PATH` now uses the established config-root
  precedence and calls `helm_theme::apply` in-process.
- `Committed` prints `generation <id> selected for future launches` and exits
  0. `CommittedWithCleanupPending` prints the same success line, warns on
  stderr with a JSON-escaped cause, and exits 0.
- `OutcomeAmbiguous` emits no stdout, reports the candidate as unconfirmed
  with a JSON-escaped cause, and exits 6. It does not retry, recover, use IPC,
  or notify/reload a session.

## TDD evidence

1. Added `apply_reports_selected_future_generation_without_reload_or_session`.
2. `cargo test -p helm-ctl apply_` initially failed because `Apply` returned
   success without running the generation publication or reporting its result.
3. Added only the direct `helm_theme::apply` outcome mapping and reran the
   focused test successfully.
4. The proposed public fatal-candidate regression was not retained: inspection
   showed the public path validates the candidate before publication, so it did
   not demonstrate a current-pointer preservation gap requiring a change.

## Final verification

- `cargo fmt --all -- --check`
- `cargo clippy -p helm-ctl --all-targets -- -D warnings`
- `cargo test -p helm-ctl` — 8 passed, 0 failed.
- `cargo test -p helm-theme public_apply_fatal_candidate_preserves_current_generation`
  — filter matched no existing regression; the command exited 0.
- `git diff --check`
