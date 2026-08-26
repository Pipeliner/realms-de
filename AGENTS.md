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
