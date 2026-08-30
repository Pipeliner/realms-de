---
name: helm-agent-sdd-checkpoint
description: "Use after a fresh local agent-SDD pilot investigation needs a resumable, issue-numbered checkpoint, before an intentional handoff, or when a pilot gate must assess a report-only transition."
---

# Capture a local agent-SDD pilot checkpoint

This manual procedure is governed by SPEC 0008. It captures non-normative
metadata only; live repository state and GitHub remain authoritative.

## Do not use when

Do not use when the repository is dirty, the investigation has not been
revalidated at the current parent commit, or a task requires a candidate or
production promotion. The pilot has no write-capable promotion operation.

## Capture after fresh investigation

1. Re-run the focused command or re-read the relevant live file. Do not carry
   forward an old result as current evidence.
2. Start from a clean worktree after the non-record work is committed. The
   record is limited to exactly:

   ```text
   .agent/work/<issue>/checkpoint.toml
   .agent/work/<issue>/evidence.jsonl
   ```

3. Write only values permitted by the checkpoint and JSONL schemas in SPEC
   0008. Use the immediate parent commit as `git_head` for the checkpoint and
   every evidence entry.
4. Commit only those two files as a one-parent record-carrier commit. Do not
   combine a checkpoint with implementation, documentation or unrelated work.
5. With the repository clean, assess the requested supported edge:

   ```text
   helm-sdd gate --issue <issue> --from <maturity> --to <maturity>
   helm-sdd promote --dry-run --issue <issue> --from <maturity> --to <maturity>
   ```

The commands are read-only. A pass does not promote maturity, alter GitHub,
approve a specification or waive any missing obligation.

## Boundaries

Only `probe`, `spike` and `prototype` are stored in the pilot. Only
`probe` to `spike` and `spike` to `prototype` can pass.
Pilot exclusions: no hook, daemon, scheduler, CI job, service, external database, embedding search or third-party integration.
