# Task 1 — Generated theme ownership regressions and activation model

## Scope delivered

- Added ownership regressions requiring every shipped output to live beneath
  `helm/generated/`.
- Added GTK activation regressions: missing `gtk.css` must contain only the
  declared import, while an existing `gtk.css` remains byte-identical.
- Added public, data-only `Activation` and `ActivationDiagnostic` models and
  shipped GTK metadata for the user activation paths and exact import lines.
- Deliberately left output routing and user-file creation unchanged for Task 2.

## Verification

- `cargo fmt --check` — passes.
- `cargo test -p helm-theme --no-run` — compiles.
- `cargo test -p helm-theme` — 27 passed; the three new Task 2 regressions are
  intentionally red:
  - `every_shipped_template_target_is_helm_owned`
  - `missing_gtk_activation_files_get_exactly_one_helm_import`
  - `gtk_activation_diagnostics_expose_the_exact_import_and_generated_target`
- `existing_gtk_activation_files_remain_byte_identical` passes, confirming
  Task 1 makes no user-file writes.

## Next task

Task 2 must redirect every target into the Helm-owned subtree and create only
absent GTK activation files after a successful owned-output apply. That work
will make the three intentional regressions green.
