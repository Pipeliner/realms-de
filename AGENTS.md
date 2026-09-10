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

- Native distro packages are built and verified in CI only. Do not install
  distro packaging toolchains merely to reproduce package builds locally, and
  do not treat their local absence as a blocker; inspect the matching CI job.
- Security checks and security-hardening review are post-MVP work. Do not make
  them MVP gates; track them for the post-MVP queue instead.
- **Never let process become the product.** Keep specifications, reviews,
  memory, orchestration, and verification proportional to the user-visible
  risk. If meta-work becomes the MVP critical path without directly improving
  launch usability, defer it and resume the highest user-visible blocker.
