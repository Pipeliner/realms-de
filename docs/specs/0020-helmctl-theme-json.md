# SPEC 0020 — helmctl theme JSON output

- **Status:** Accepted (2026-08-30)
- **Milestone:** M1
- **Issue:** [#166](https://github.com/Pipeliner/realms-de/issues/166)
- **Decisions:** [SPEC 0006](0006-helm-ctl.md), [SPEC 0011](0011-theme-activation-generations.md)
- **Supersedes / Superseded by:** —

## Purpose

Make `helmctl theme lint --json` and `helmctl theme diff --json` safe for
scripts without changing their read-only behaviour or exit meanings.

## Behaviour

1. Each successful or negative JSON invocation writes exactly one compact JSON
   object plus one trailing newline to stdout and writes nothing to stderr.
2. `theme lint --json` uses fields in this order: `status`, `separations`,
   `findings`. `status` is `clean` or `fatal`. Separations preserve existing
   order and use fields `a`, `b`, `degrees`; findings preserve existing order
   and use fields `path`, `message`, `fatal`.
   Clean exits 0 with empty findings; fatal exits 1 with all findings.
3. `theme diff --json` uses fields in this order: `status`, `changes`.
   `status` is `identical` for an empty diff or `different` otherwise. Changes
   are sorted by normalized path and use fields `kind`, `path`; kind is
   `added`, `removed`, or `byte-different`. Identical exits 0; different exits 1.
4. Palette input/parse failures for lint and generation or I/O refusals for
   diff write no JSON stdout. They retain existing human diagnostics on stderr
   and their existing exit code; a failed command never fabricates a result.
5. JSON serialization owns escaping. Neither command creates, repairs,
   publishes, selects, reloads, or otherwise mutates a generation.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the shipped clean palette, when `theme lint --json` runs, then stdout is one ordered clean object with separations and empty findings; stderr is empty and exit is 0. | `theme_cli::lint_json_clean_is_one_ordered_object` |
| A2 | Given a palette with fatal findings, when `theme lint --json` runs, then stdout is one ordered fatal object containing every finding; stderr is empty and exit is 1. | `theme_cli::lint_json_fatal_contains_every_finding` |
| A3 | Given malformed lint input, when `theme lint --json` runs, then stdout is empty and stderr remains diagnostic. | `theme_cli::lint_json_input_failure_has_no_result_object` |
| A4 | Given identical or changed generation outputs, when `theme diff --json` runs, then it emits an ordered identical or different object with normalized sorted changes and retains exits 0 or 1. | `theme_cli::diff_json_reports_identical_and_sorted_changes` |
| A5 | Given invalid current generation, when `theme diff --json` runs, then stdout is empty, stderr is diagnostic, exit is 6, and the tree is unchanged. | `theme_cli::diff_json_refusal_has_no_result_object` |

## Failure modes

The contract prevents scripts from parsing a diagnostic as success or mistaking
an I/O refusal for an empty diff. It creates neither a session wire format nor a
live-upgrade or retry protocol.

## Open questions

None. This refines accepted SPEC 0006 and preserves its output-stream and
exit-code rules.
