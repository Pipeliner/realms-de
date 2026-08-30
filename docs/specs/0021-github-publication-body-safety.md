# SPEC 0021 — GitHub publication body safety

- **Status:** Accepted (2026-08-30)
- **Milestone:** M0 (repository-maintenance safety)
- **Issue:** [#190](https://github.com/Pipeliner/realms-de/issues/190)
- **Decisions:** [AGENTS.md](../../AGENTS.md)
- **Supersedes / Superseded by:** —

## Purpose

Prevent Markdown intended for a GitHub issue comment, issue creation, or pull
request creation from being interpreted by the local shell while a
repository-controlled publication command is assembled.

## Scope and boundary

This specification covers tracked repository automation and the project helper.
It does not claim to control an arbitrary interactive command typed outside the
repository. Enforcing that wider boundary needs an execution-environment policy
and is not introduced here.

## Behaviour

1. `scripts/gh-body-file` is the sole tracked repository helper for its
   supported GitHub Markdown-body operations. It accepts body content only as
   a final `BODY_FILE` argument, requires it to be a readable regular file, and
   forwards that pathname through GitHub CLI's file-body facility. Body bytes
   are never a shell argument, shell program, or helper-generated string.
2. The helper supports `issue-create`, `issue-comment`, and `pr-create` with
   fixed argument positions. Issue numbers are positive decimal identifiers;
   branch references are nonempty and do not begin with `-`; titles remain one
   separately quoted metadata argument. Invalid arity or body files fail before
   any GitHub CLI invocation.
3. `docs/check-github-body-safety.sh` deterministically examines every tracked
   shell command source, Make/just source, workflow, and composite action,
   excluding only its static test fixtures and the checker itself. It rejects a
   GitHub body-bearing command except for the exact approved helper invocations.
   It rejects direct or simple indirect GitHub CLI/API publication, known
   authoring actions, and direct dynamic shell evaluation in those command
   surfaces. Documentation prose and static test fixtures remain outside that
   command-source check.
4. The normal CI workflow runs the helper's black-box test and the guard's
   pass/fail fixtures before documentation success is reported.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a body fixture containing backticks, substitution syntax, quotes, glob characters and newlines, when every supported helper operation runs against a fake GitHub CLI, then the fake receives the exact fixed argv and only the fixture pathname; no fixture content is evaluated. | `docs/test-gh-body-file.sh` — `literal-file-forwarding` |
| A2 | Given invalid identifiers, arity, or non-regular/unreadable body inputs, when the helper is run, then it fails before invoking the fake GitHub CLI. | `docs/test-gh-body-file.sh` — `reject-before-gh` |
| A3 | Given a tracked automation fixture containing any forbidden direct body publication form, when the repository guard runs, then it fails; given the approved helper and read-only queries, it passes. | `docs/test-github-body-safety.sh` — `tracked-command-allowlist` |
| A4 | Given the normal pull-request CI workflow, when documentation checks run, then the helper and guard fixtures execute and the repository guard passes. | `.github/workflows/ci.yml` — `docs` |

## Failure modes

An unsafe tracked command is a repository-policy failure, not a recoverable
publication path. The guard is deliberately conservative: extending GitHub
publication capability requires extending this accepted specification, the
helper, and its black-box tests in the same change.

## Open questions

None. Interactive execution-boundary enforcement is explicitly out of scope.
