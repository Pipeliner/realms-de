---
name: helm-agent-sdd-bootstrap
description: "Use when beginning a local agent-SDD pilot investigation for one GitHub issue, recovering a pilot task after a handoff, or deciding whether the current issue can safely use the report-only pilot records."
---

# Bootstrap the local agent-SDD pilot

This is a local, report-only procedure governed by SPEC 0008. GitHub issues,
accepted repository specs, live Git state and tests remain authoritative. It
does not create a record, change issue status, or infer product behaviour.

## Do not use when

Do not use when implementing product behaviour, making a semantic product
decision, or selecting normal Symphony work without a local pilot record. Use
`helm-spec-first` and `helm-issue-flow` for those tasks.

## Orient one issue

1. Read `AGENTS.md`, `CLAUDE.md`, `.claude/memory/00-standing-orders.md` and
   `.claude/memory/40-loop.md`.
2. Follow `helm-issue-flow` to select and track the GitHub issue. Follow
   `helm-spec-first` before any non-trivial implementation work.
3. Re-query the live branch, working tree, relevant specs and focused tests.
   A prior checkpoint is historical metadata, not current source truth.
4. If `.agent/work/<issue>/` exists, inspect only its `checkpoint.toml` and
   `evidence.jsonl`, then rerun or reread the cited live evidence before using
   it.
5. Use the existing read-only assessment only after the record is fresh and the
   repository is clean:

   ```text
   helm-sdd gate --issue <issue> --from <maturity> --to <maturity>
   helm-sdd promote --dry-run --issue <issue> --from <maturity> --to <maturity>
   ```

`helm-sdd` currently exposes only `gate` and `promote --dry-run` for this
pilot. A pass reports whether a supported edge is met; it does not promote a
task, amend a spec or update GitHub.

## Boundaries

Pilot exclusions: no hook, daemon, scheduler, CI job, service, external database, embedding search or third-party integration.
If the issue needs a product or
process decision not supplied by accepted repository authority, follow S3:
mark it `needs-human` with options and a recommendation rather than guessing.
