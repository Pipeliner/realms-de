# SPEC 0008 — Local agent-SDD pilot governance

- **Status:** Accepted (2026-08-29)
- **Milestone:** M0 (internal practice; it does not block product MVP work)
- **Decision:** [ADR 0014](../adr/0014-local-agent-sdd-pilot-governance.md)
- **Issue:** [#119](https://github.com/Pipeliner/realms-de/issues/119)

## Purpose and authority

This specification defines a narrow local pilot for resumable agent work. It
defines records a future `helm-sdd` may read/report on; it creates no product
feature or runtime behaviour.

| Domain | Authority | Pilot role |
|---|---|---|
| Product requirements, behaviour, architecture | Accepted `docs/specs/` and `docs/adr/` | May cite IDs; cannot define, amend or approve them |
| Public task state | GitHub issues/labels | Issue number keys records; status is never mirrored |
| Executable behaviour | Tests and CI at a revision | Records may cite a result; command/test is authoritative |
| Curated operational knowledge | `.claude/memory/` | Revisable working knowledge, never product truth |
| Resumable task metadata | `.agent/work/<github-issue>/` | Non-normative checkpoint/evidence only |

Live Git, files, tests and GitHub prevail over any record. A record never
permits skipping the repository's accepted-spec-before-code and TDD order.

## Scope and exclusions

Only `probe`, `spike` and `prototype` exist in the pilot.

| Transition | Required report contents | Result |
|---|---|---|
| `probe → spike` | question/hypothesis, success condition, one reproducible evidence item | report pass/fail only |
| `spike → prototype` | reproducible evidence, limitations, provenance and affected accepted specs if known | report pass/fail only |
| any transition to `candidate` or `production` | outside scope | always report unsupported; never mutate state |

No daemon, scheduler, hook, passive capture, cloud service, embeddings/vector
search, external database, OpenSpec, Sphinx-Needs, Serena, Beads integration,
Basic Memory integration or CI job is introduced. Basic Memory can be used
manually/local by a maintainer, but no record requires it or treats it as truth.

## Directory and TOML checkpoint schema

Each record directory is `.agent/work/<github-issue>/`; `<github-issue>` is a
positive decimal GitHub issue number with no leading zero. It may contain only
`state.md`, `checkpoint.toml` and `evidence.jsonl` in this pilot. `state.md` is
a readable projection; structured records control validation.

```toml
schema = "helm-agent-sdd/checkpoint/v1"
issue = 119
reason = "handoff"
created_at = "2026-08-29T12:00:00Z"
target_maturity = "spike"
git_head = "0123456789abcdef0123456789abcdef01234567"

[goal]
statement = "State one durable task goal."

[acceptance]
criteria = ["A concrete condition or accepted spec ID."]

[workspace]
branch = "codex/example"
base = "0123456789abcdef0123456789abcdef01234567"
dirty = false

[[completed]]
claim = "A completed, evidence-backed action."
evidence = ["ev-001"]

[[hypotheses]]
id = "H1"
statement = "An unresolved proposition."
confidence = "low"

[[rejected_approaches]]
approach = "A rejected approach."
reason = "Why it was rejected."
evidence = ["ev-001"]

[next_actions]
items = ["Inspect a named live file or rerun a named check."]
```

Required fields are `schema`, `issue`, `reason`, `created_at`,
`target_maturity`, `git_head`, `goal`, `acceptance`, `workspace` and
`next_actions`. The three array tables may be empty. `schema` is exactly
`helm-agent-sdd/checkpoint/v1`; maturity is `probe`, `spike` or `prototype`;
Git IDs are lowercase 40-character object IDs; timestamps are UTC RFC 3339;
and `next_actions.items` holds one to three non-empty actions. All evidence
references name sibling JSONL IDs.

## JSONL evidence schema and retention boundary

`evidence.jsonl` is UTF-8 JSON Lines: one object per line, no blank lines and
unique IDs. Absent optional fields are omitted, never `null`.

```json
{"id":"ev-001","ts":"2026-08-29T12:00:00Z","kind":"command","summary":"Ran the focused IPC test","command":"cargo test -p helm-core ipc::tests::frame_round_trip","exit_code":0,"git_head":"0123456789abcdef0123456789abcdef01234567","purpose":"reproduce framing behaviour"}
{"id":"ev-002","ts":"2026-08-29T12:03:00Z","kind":"file-observation","path":"crates/helm-core/src/ipc.rs","lines":"81-103","git_head":"0123456789abcdef0123456789abcdef01234567","claim":"Frames end in LF."}
{"id":"ev-003","ts":"2026-08-29T12:05:00Z","kind":"decision","claim":"Do not use sleep-based synchronisation.","reason":"It is nondeterministic under load.","derived_from":["ev-001"]}
```

Every non-decision requires `id`, `ts`, `kind` and `git_head`. A `decision`
requires `id`, `ts`, `kind`, `claim`, `reason` and one or more
`derived_from` IDs, from which it derives its revision. IDs are `ev-` plus a
positive decimal number; timestamps/Git IDs use the checkpoint forms; kind is
exactly `command`, `file-observation` or `decision`.

| Kind | Required fields | Optional fields |
|---|---|---|
| `command` | `summary`, `command`, `exit_code`, `purpose` | `path` |
| `file-observation` | `path`, `lines`, `claim` | `summary` |
| `decision` | `claim`, `reason`, `derived_from` | `summary` |

`command` is an invoked command line, never stdout, stderr, environment, shell
history, an absolute working directory or a substituted secret. No record may
contain credentials, tokens, passwords, private keys, cookies, bearer/API
values, customer data, arbitrary transcript text, complete source contents or
unbounded command output. Unknown fields and secret-detector matches fail; a
failure names record/field but never echoes prohibited content.

File observations are commit-scoped. Before using one, compare `git_head` with
current Git and reread the live path whenever they differ. Command success is
evidence at a revision, never a durable assertion that it still passes.

## Promotion and failures

Future #120 interfaces `helm-sdd gate --from <maturity> --to <maturity>` and
`helm-sdd promote <path> --to <maturity>` are read-only in this pilot: they
report satisfied/missing obligations and return non-zero for invalid records,
stale/missing evidence or unsupported transitions. They never edit records,
docs, memory, GitHub, labels, approvals or Git; nor infer approval or turn an
observation into a normative requirement.

Malformed TOML/JSONL, unknown fields, duplicate IDs, bad references, a
disallowed maturity, invalid Git IDs/timestamps and prohibited content fail.

## Acceptance criteria

#120 owns implementation/fixture tests. Until it lands these are requirements,
not existing-tool claims.

| # | Given / When / Then | Planned verification |
|---|---|---|
| A1 | Valid `probe` records validated | Success without tracked-file changes | #120 fixture + clean `git diff --exit-code` |
| A2 | Unknown field, duplicate ID, bad reference/timestamp/Git ID | Failure identifies only safe metadata | #120 fixture |
| A3 | Raw output, environment, credential-like value, source snapshot or unknown field | Hygiene/evidence validation fails without echoing it | #120 fixture |
| A4 | Evidence at a non-current revision offered as current fact | Report requires live revalidation | #120 fixture |
| A5 | Valid permitted gate requested | Report is complete and read-only | #120 fixture + clean diff |
| A6 | Candidate/production requested | Unsupported/read-only failure | #120 fixture + clean diff |
| A7 | Future pilot implementation reviewed | No daemon/hook/CI/cloud/embedding/Beads/Basic-Memory integration exists | #120 review/diff |

## Open questions

None. Expansion requires a new accepted ADR/specification after pilot evidence
is reviewed.

