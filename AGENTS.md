# Codex working agreement

## Non-negotiable delivery order

**Solve at the specification level first.** Before editing implementation code:

1. Find or write the governing specification and ensure it is **Accepted**.
2. Amend that specification for any newly discovered bug or security property.
3. Write the corresponding test and observe it fail.
4. Implement only the behaviour the accepted specification requires.

Do not use code to decide an unresolved design question. Write an ADR or mark
the issue `needs-human` instead. Keep the spec, tests, and implementation in
the same change whenever a contract changes.

## Repository operations

- Keep `README.md` synchronized with current verified project state. When
  delivery, install commands, downloadable artifacts, known limitations or
  human decisions change, update the README in the same change/iteration.
  Verify artifact availability and exact CI evidence before recommending a
  build; record its revision, verification date and expiration. Distinguish
  passing jobs from overall workflow success and trial builds from the MVP.
  Refresh or withdraw expired/broken trial links; never leave historical
  snapshots presented as current. Keep the README truth fixtures in sync.

- Track time spent on each specific task from 2026-09-27 onward in
  `docs/task-time.csv`. Record UTC start/end, issue or task ID, actor, activity
  category and outcome at task switches and handoff. Separate active work from
  CI/external waiting; do not count overlapping waits as active work or sum
  parallel agent time as elapsed delivery time. Do not fabricate historical
  durations. Follow `docs/task-time.md`; keep bookkeeping lightweight.

- For the MVP launch, follow `docs/specs/0031-mvp-launch-delivery.md`: three
  bounded delivery lanes, one writer for shared launch plumbing, draft PRs for
  development and full CI before integration. Batch docs/queue updates; defer
  routine dependency PRs but inspect them. Reuse #78's acceptance evidence and
  do not turn optional Waybar/UWSM evaluation into an open-ended launch blocker.

- When an issue recurs, investigate and fix the shared cause at the specification
  and implementation levels. Repeating a workaround is temporary recovery,
  not resolution; verify that the original recurrence scenario is prevented.
- Run `gh` outside the sandbox.
- When sandboxing is suspected to explain a diagnostic failure, hang, or inaccessible dependency, rerun the same read-only/diagnostic command in the unsandboxed execution context before treating it as a repository defect. This does not authorize local package builds or installation; packaging remains CI-only.
- Use `scripts/ci-monitor --once` for each Symphony CI inspection and `scripts/ci-monitor --watch` for bounded background monitoring; keep it read-only, unsandboxed, and running for the current MVP verification and future iterations.
- At every Symphony iteration, inspect and handle the complete open pull-request
  queue, including automated dependency PRs, before selecting the next issue.
- Commit and push completed repository work unless the user explicitly says
  otherwise.
- **Never pass Markdown or other GitHub body content through a shell argument.**
  In particular, never use `gh ... --body`, command substitution, a heredoc, or
  shell interpolation for an issue or PR body. Write the exact literal content
  to a tracked or temporary file with `apply_patch`, then call
  `scripts/gh-body-file` so `gh --body-file` reads it without shell evaluation.
  This rule exists because Markdown backticks are shell syntax in an inline
  command and must be mechanically unable to execute.

- **“Understood” is never evidence.** Classify every direction by scope
  (conversation, task, repository policy, specification, or external side
  effect), record it at the appropriate durable authority, and verify the
  resulting state before claiming it was handled.

- **All packaging is performed in CI only.** This includes native/Nix package
  builds, source archives, vendor bundles, provenance/source-bundle rebinding,
  and package installation/verification. Do not run these locally or install
  packaging toolchains. Local source edits and lightweight tests that produce
  no packages or bundles are allowed. Inspect CI for packaging evidence; if a
  packaging step lacks a CI path, add that path rather than executing it locally.
- Regularly clean stale agent-generated build caches using explicit ownership,
  active-build exclusion, and bounded retention. Never sweep source worktrees,
  user files, retained bundles, or verification evidence. Keep local cleanup
  infrastructure private and record its operation at the local authority.
- All permitted local Cargo checks across agents and worktrees use one shared
  `CARGO_TARGET_DIR` through the private cache wrapper. Do not create per-task
  target directories or bypass its active-build lease. Apply periodic size and
  age cleanup only when the shared cache is inactive; packaging remains CI-only.
- Security checks and security-hardening review are post-MVP work. Do not make
  them MVP gates; track them for the post-MVP queue instead.
- **Never let process become the product.** Keep specifications, reviews,
  memory, orchestration, and verification proportional to the user-visible
  risk. If meta-work becomes the MVP critical path without directly improving
  launch usability, defer it and resume the highest user-visible blocker.
