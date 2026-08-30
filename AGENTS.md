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
- Commit and push completed repository work unless the user explicitly says
  otherwise.
- **Never pass Markdown or other GitHub body content through a shell argument.**
  In particular, never use `gh ... --body`, command substitution, a heredoc, or
  shell interpolation for an issue or PR body. Write the exact literal content
  to a tracked or temporary file with `apply_patch`, then call
  `scripts/gh-body-file` so `gh --body-file` reads it without shell evaluation.
  This rule exists because Markdown backticks are shell syntax in an inline
  command and must be mechanically unable to execute.
