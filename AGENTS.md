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
