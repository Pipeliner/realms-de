# SPEC 0017 — Contribution templates

- **Status:** Accepted (2026-08-30)
- **Milestone:** M0
- **Issue:** [#9](https://github.com/Pipeliner/realms-de/issues/9)
- **Decisions:** Standing orders S3, S7 and S14
- **Supersedes / Superseded by:** —

## Purpose

Make the repository's spec-first and human-escalation discipline available at
the point a contributor opens an issue or pull request.

## Behaviour

1. The issue chooser offers a `Work item` form. Its pre-filled body names Why
   this matters, a Given/When/Then acceptance checklist, `Blocked by:`, and a
   `Source:` link to a repository document section.
2. The chooser offers a dedicated `Needs a human` form with exactly four
   contributor-answer sections: decision, options and honest trade-offs,
   recommendation, and what remains blocked.
3. The pull-request template asks for its governing spec, `Closes #`, and a
   local confirmation of `cargo fmt --all && cargo clippy --all-targets -- -D
   warnings && cargo test`.
4. The bug-report form asks which `docs/PITFALLS.md` guard should have caught
   the regression.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the issue forms, when the work-item form is inspected, then it exposes the four required work-state prompts. | `docs/test-contribution-templates.sh` — `work-item-contract` |
| A2 | Given the issue forms, when the needs-human form is inspected, then it exposes exactly the four S3 decision prompts. | `docs/test-contribution-templates.sh` — `needs-human-contract` |
| A3 | Given the pull-request template, when inspected, then it names spec, closing issue, and the local trio command. | `docs/test-contribution-templates.sh` — `pull-request-contract` |
| A4 | Given the bug report form, when inspected, then it asks for the PITFALLS guard that should have caught the defect. | `docs/test-contribution-templates.sh` — `bug-guard-contract` |

## Open questions

None. This is a direct implementation of existing standing orders, not a new
product or process decision.
