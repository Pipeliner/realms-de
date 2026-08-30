# SPEC 0018 — Local agent-SDD pilot procedures

- **Status:** Accepted (2026-08-30)
- **Milestone:** M0
- **Issue:** [#121](https://github.com/Pipeliner/realms-de/issues/121)
- **Decisions:** [SPEC 0008](0008-agent-sdd-pilot-governance.md), [ADR 0014](../adr/0014-local-agent-sdd-pilot-governance.md)
- **Supersedes / Superseded by:** —

## Purpose

Make the accepted, local-only agent-SDD pilot repeatable without creating a
second task tracker, authoritative memory store, write-capable helper, or
automation service.

## Behaviour

1. `helm-agent-sdd-bootstrap` orients an agent to one GitHub issue, the live
   repository state, SPEC 0008 and the existing issue/spec-first procedures. It
   never creates a record or invents a status. The project skill index and loop
   memory make the three pilot procedures discoverable and state how to measure
   their two real iterations.
2. `helm-agent-sdd-checkpoint` describes how a maintainer manually captures a
   fresh, clean, issue-numbered record in exactly the two files permitted by
   SPEC 0008, commits it as a separate record-carrier commit, and runs only the
   existing read-only gate or dry-run promotion assessment.
3. `helm-agent-sdd-evidence-capture` describes manual, minimal evidence
   capture from a just-rerun command or just-read file. It excludes command
   output, secrets, environment values, absolute paths, shell history and
   source snapshots.
4. The procedures state that `helm-sdd` currently exposes only `gate` and
   `promote --dry-run`; a successful report is not a maturity promotion.
5. The procedures introduce no CI job, hook, daemon, scheduler, external
   service, embedding search, database or third-party integration.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given the project skill directory, when the pilot procedures are inspected, then bootstrap, checkpoint and evidence-capture skills have valid front matter, point to the accepted pilot governance, and are discoverable from the skill index and loop measurement guidance. | `docs/test-agent-sdd-pilot-procedures.sh` — `procedure-contract` |
| A2 | Given the checkpoint and evidence procedures, when their safety boundaries are inspected, then they describe exactly the permitted record files, fresh read-only assessment and excluded sensitive material. | `docs/test-agent-sdd-pilot-procedures.sh` — `record-safety-contract` |
| A3 | Given any pilot procedure, when its command surface and scope are inspected, then it does not document a write-capable pilot command or introduce excluded automation. | `docs/test-agent-sdd-pilot-procedures.sh` — `report-only-contract` |
| A4 | Given two subsequent real Symphony iterations that use the procedures, when their local results are reviewed, then revisable operational memory records the recovery detail available, added record work and limitations without treating either observation as product truth. | Manual evidence from two subsequent iterations |

## Failure modes

The procedures prevent stale or sensitive operational records from being
mistaken for current repository truth. The authoritative validation and
failure reports remain those in SPEC 0008 and `helm-sdd`.

## Open questions

None. This specifies focused guidance over accepted pilot behaviour; it does
not make a new process or product decision.
