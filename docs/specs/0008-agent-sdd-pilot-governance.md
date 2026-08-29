# SPEC 0008 — Local agent-SDD pilot governance

- **Status:** Accepted (2026-08-29)
- **Milestone:** M0 (internal practice; it does not block product MVP work)
- **Decision:** [ADR 0014](../adr/0014-local-agent-sdd-pilot-governance.md)
- **Issue:** [#119](https://github.com/Pipeliner/realms-de/issues/119)

## Purpose and authority

This specification defines a narrow, planned local pilot for resumable agent
work. It defines records that future `helm-sdd` may read and report on; it
creates no product feature or runtime behaviour.

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

Only `probe`, `spike` and `prototype` exist in the pilot. The pilot accepts
only these report-only transitions:

| Transition | Required checkpoint fields | Required evidence | Result |
|---|---|---|---|
| `probe → spike` | `question`, `success_condition` | one command or file observation | pass/fail report |
| `spike → prototype` | `limitations`, `affected_specs` (possibly empty) | one command or file observation | pass/fail report |
| any transition to `candidate` or `production` | n/a | n/a | unsupported report |

No daemon, scheduler, hook, passive capture, cloud service, embeddings/vector
search, external database, OpenSpec, Sphinx-Needs, Serena, Beads integration,
Basic Memory integration or CI job is introduced. Basic Memory can be used
manually/local by a maintainer, but no record requires it or treats it as truth.

## Directory, text-safety and TOML checkpoint schema

Each record directory is `.agent/work/<github-issue>/`; `<github-issue>` is a
positive decimal GitHub issue number with no leading zero. It contains exactly
`checkpoint.toml` and `evidence.jsonl`; `state.md` is deliberately excluded so
that the pilot has no unvalidated free-text projection. Unknown files fail
hygiene validation.

Every string is Unicode NFKC-normalised before validation and must contain
only printable ASCII characters (`U+0020`–`U+007E`), with no newline, tab or
other control character. JSON object keys are fixed by the schema below;
unknown keys fail. The permitted string classes are:

| Class | Exact rule |
|---|---|
| prose | 1–240 ASCII printable characters; it may not contain `` ` ``, `$`, `{`, `}`, `;`, `\\`, `"`, `'`, `<`, `>`, `|`, `&`, `:`, `=`, or a run of three or more `-` characters |
| command | 1–512 characters matching `^[A-Za-z0-9_./:@%+, -]+$`; it may not start with `-`, contain `..` or contain `/`, and no whitespace-delimited token may equal `fn`, `struct`, `enum`, `const`, `let`, `impl`, `use`, `pub` or `mod` |
| repository path | 1–240 characters matching `^[A-Za-z0-9][A-Za-z0-9._/-]*$`, with no empty, `.` or `..` segment and no leading `/` |
| branch | 1–128 characters matching `^[A-Za-z0-9][A-Za-z0-9._/-]*$`, with no `..` or trailing `.` or `/` |
| evidence ID | `^ev-(?:0[0-9]{2,}|[1-9][0-9]*)$` |
| document reference | `^(ADR|SPEC)-[0-9]{4}$` |
| commit ID | `^[0-9a-f]{40}$` and resolves to a commit |
| timestamp | UTC RFC 3339 in `YYYY-MM-DDTHH:MM:SSZ` form |
| lines | `^[1-9][0-9]*(?:-[1-9][0-9]*)?$` with an end not less than its start |

NFKC is an inspection copy only: the parsed value is retained unchanged and
then rejected unless it is printable ASCII, so normalisation cannot silently
alter a record. The detector applies an unanchored ASCII substring search to
every string value, including `command`, each array element and every
nested-table string. It uses Rust `regex` 1.x syntax; `(?i)` means ASCII-only
case-insensitive matching, and `^`/`$` (where present) refer to the complete
single-line string. It rejects these exact patterns:

```text
-----BEGIN (?:[A-Z0-9 ]+ )?PRIVATE KEY-----
(?:^|[^A-Za-z0-9_-])[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}(?:$|[^A-Za-z0-9_-])
(?i)(authorization|bearer|api[_-]?key|access[_-]?token|secret|password|passwd|private[_-]?key|session|cookie)[ ]*(=|:)[ ]*[^ ]+
```

For this pilot, prohibited **source-code material** is a multi-line excerpt or
an individual prose field containing a source delimiter (backtick, quote,
brace, semicolon, backslash, shell substitution/control operator, angle
bracket, pipe, ampersand, colon or equals sign), or a command containing a
prohibited source token. Every prose delimiter is excluded and every field is
single-line; this applies independently to every command and array element. A
bare identifier or a plain English sentence that happens to resemble a comment
is not source-code material and is governed by the metadata limits instead.
The pilot deliberately does not claim content-similarity detection across
fields or a general source snapshot classifier. The command grammar excludes
shell syntax capable of embedding a source fragment. The validator reports
only the prescribed safe obligation code/status on stdout, never a rejected
value, path, line or field name. It is a deterministic best-effort guard,
**not proof that a record is secret-free**:
sensitive or customer material must never be recorded.

```toml
schema = "helm-agent-sdd/checkpoint/v1"
issue = 119
reason = "handoff"
created_at = "2026-08-29T12:00:00Z"
current_maturity = "probe"
requested_maturity = "spike"
git_head = "0123456789abcdef0123456789abcdef01234567"
question = "Can the focused IPC test reproduce the framing behaviour?"
success_condition = "One reproducible command or file observation exists."
limitations = ["No product contract is inferred from this probe."]
affected_specs = ["SPEC-0001"]

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
`current_maturity`, `requested_maturity`, `git_head`, `question`,
`success_condition`, `limitations`, `affected_specs`, `goal`, `acceptance`,
`workspace` and `next_actions`. The three array tables may be empty.
No TOML table or key may repeat. All listed fields are required unless marked
optional; no other field or table is permitted.

| Location | Type and exact constraint |
|---|---|
| root `schema` | string exactly `helm-agent-sdd/checkpoint/v1` |
| root `issue` | integer `1..=2_147_483_647`, equal to the directory number |
| root `reason`, `question`, `success_condition` | prose |
| root `created_at` | timestamp |
| root `current_maturity`, `requested_maturity` | enum: `probe`, `spike`, `prototype` |
| root `git_head`; `workspace.base` | commit ID |
| root `limitations`, `affected_specs` | arrays of 0–16 prose/document-reference strings respectively |
| `goal.statement` | prose |
| `acceptance.criteria` | array of 1–16 prose/document-reference strings |
| `workspace.branch` | branch; `workspace.dirty` is boolean |
| each `completed` | table exactly `{claim: prose, evidence: array[1..16] of evidence ID}` |
| each `hypotheses` | table exactly `{id: prose, statement: prose, confidence: low|medium|high}`; IDs unique within the array |
| each `rejected_approaches` | table exactly `{approach: prose, reason: prose, evidence: array[1..16] of evidence ID}` |
| `next_actions.items` | array of 1–3 prose strings |

`affected_specs` values resolve at assessment `HEAD`, not the historic record
commit. `ADR-0014` maps to exactly one `docs/adr/0014-*.md` whose first line
matches Rust regex `^# ADR 0014 — .+$`. The file must contain exactly one line matching Rust regex
`^- \*\*Status:\*\* Accepted(?: \([0-9]{4}-[0-9]{2}-[0-9]{2}\))?$` at HEAD.
`SPEC-0008` maps to exactly one `docs/specs/0008-*.md` whose first line matches
Rust regex `^# SPEC 0008 — .+$`; it uses the same exact-one status-line rule.
ADR and SPEC references are both allowed. An empty
`affected_specs` explicitly means no accepted document is known to be affected.
All evidence references name sibling JSONL IDs.

## JSONL evidence schema, provenance and retention boundary

`evidence.jsonl` is UTF-8 JSON Lines: one object per line, no blank lines and
unique IDs. Absent optional fields are omitted, never `null`. Every evidence
object, including a decision, requires `id`, `ts`, `kind` and `git_head`; its
Git ID must resolve to a commit. IDs are `ev-` plus a positive decimal number;
timestamps/Git IDs use the checkpoint forms; kind is exactly `command`,
`file-observation` or `decision`.

```json
{"id":"ev-001","ts":"2026-08-29T12:00:00Z","kind":"command","summary":"Ran the focused IPC test","command":"cargo test -p helm-core ipc::tests::frame_round_trip","exit_code":0,"git_head":"0123456789abcdef0123456789abcdef01234567","purpose":"reproduce framing behaviour"}
{"id":"ev-002","ts":"2026-08-29T12:03:00Z","kind":"file-observation","path":"crates/helm-core/src/ipc.rs","lines":"81-103","git_head":"0123456789abcdef0123456789abcdef01234567","claim":"Frames end in LF."}
{"id":"ev-003","ts":"2026-08-29T12:05:00Z","kind":"decision","git_head":"0123456789abcdef0123456789abcdef01234567","claim":"Do not use sleep-based synchronisation.","reason":"It is nondeterministic under load.","derived_from":["ev-001"]}
```

| Kind | Required fields beyond common fields | Optional fields |
|---|---|---|
| `command` | `summary`, `command`, `exit_code`, `purpose` | `path` |
| `file-observation` | `path`, `lines`, `claim` | `summary` |
| `decision` | `claim`, `reason`, `derived_from` | `summary` |

`derived_from` contains one or more earlier (strictly smaller JSONL line
number), non-decision evidence IDs at the
same `git_head`; it may not refer to itself. This makes provenance acyclic and
single-revision. `command` is an invoked command line, never stdout, stderr,
environment, shell history, an absolute working directory or a substituted
secret. Every JSON object has exactly its common fields plus its required and
optional kind fields; `id` uses evidence-ID; `summary`, `purpose`, `claim` and
`reason` use prose; `command`, `path` and `lines` use their named string
classes; `ts` is a timestamp;
`exit_code` is an integer `0..=255`; and `derived_from` is an array of 1–16
evidence IDs. JSON numbers must be integral and arrays may contain only the
declared element type. A validation failure is represented only by the
prescribed safe obligation code/status report.

Tracked records are retained in ordinary Git history. Deleting a record stops
future use but does not reliably erase historical commits or clones; therefore
sensitive material is not recoverably retractable and must never be recorded.

File observations and command results are commit-scoped. A passing assessment
requires a clean repository: `git status --porcelain=v1 --untracked-files=all`
is empty and its observed dirty state equals `workspace.dirty` (therefore both
are `false`). Assessment `HEAD` must be a **record-carrier commit** with exactly
one parent, called `C`. Compare `C^{tree}` and `HEAD^{tree}` recursively as raw
Git tree entries, without rename or copy inference. The set of paths with any
addition, deletion, blob-object, mode or type difference must be exactly
`.agent/work/<issue>/checkpoint.toml` and `.agent/work/<issue>/evidence.jsonl`.
Both paths at `HEAD` must be regular non-executable blobs (`100644`); at `C`
each may be absent or a regular non-executable blob. No other tree entry may
differ. `checkpoint.git_head` and every `evidence.git_head` must equal `C`, and
`workspace.base` must be an ancestor of `C`. Otherwise it reports
`fresh_checkpoint` or `fresh_evidence` as unmet, as applicable. This permits the
tracked metadata commit to describe the immediately preceding, unchanged
repository revision without requiring an impossible Git self-reference. It
never treats old success as a current fact: a new record-carrier commit is
required after any non-record change, and live files/tests must be reread or
rerun before that record supplies evidence.

## Report-only gate contract

Future #120 commands select one record by `--issue <positive-decimal>`:

```text
helm-sdd gate --issue <n> --from <maturity> --to <maturity>
helm-sdd promote --dry-run --issue <n> --from <maturity> --to <maturity>
```

`promote` is only a named dry-run assessment alias; neither command writes.
The valid transition matrix is exactly `probe → spike` and `spike → prototype`.
No-op, reverse, skipped and other pilot edges are invalid. A requested `to` of
`candidate` or `production` is unsupported even though those values cannot be
stored in a checkpoint.

Assessment order is: (1) parse CLI and load the selected record; (2) schema,
hygiene, Git-object, directory-issue, decision-provenance and accepted-ref
validation; (3) unsupported `to`; (4) `from/current_maturity` and
`to/requested_maturity` equality plus transition-matrix validation; (5) dirty
workspace, stale evidence and transition obligations. A failure in steps 1–2
is `invalid`; step 3 is `unsupported`; step 4 is `invalid`; and step 5 is
`unmet`. This precedence is mandatory.

Validation short-circuits at the first failure. It always emits one compact
JSON object. A CLI parse failure uses `issue`, `from` and `to` as `null`,
`outcome` `invalid`, and exactly `[{"code":"cli","status":"invalid"}]`.
A missing record directory after valid flags uses the parsed values, `outcome`
`invalid`, and exactly `[{"code":"record","status":"invalid"}]`. At a
step-2 failure it uses the
parsed values, `outcome` `invalid`, and exactly one object for the first failed
code in this fixed order: `schema`, `hygiene`, `git_objects`,
`issue_directory`, `decision_provenance`, `accepted_refs`. Only after these
steps pass does the normal complete obligation set below apply.

For either valid edge, `transition_fields` is met only when all required table
fields for that edge are present and non-empty where their type permits; for
`spike → prototype`, `limitations` must contain at least one prose item.
`transition_evidence` is met only when at least one command with `exit_code`
zero or one file observation exists at `C`. All evidence must be fresh for
`fresh_evidence` to be met; a failing command can be recorded but does not
satisfy `transition_evidence`. If assessment `HEAD` is a root or merge commit,
there is no unique `C`: `fresh_checkpoint`, `fresh_evidence` and
`transition_evidence` are all `unmet`. If `HEAD` has one parent but otherwise
fails the record-carrier tree rule, `transition_evidence` is still assessed at
that parent; the two freshness obligations remain `unmet`.

Output is canonical compact JSON with top-level properties in this exact order:
`issue`, `from`, `to`, `outcome`, `obligations`. `obligations` contains exactly
one object for each applicable code below, sorted bytewise ascending by `code`;
each object has properties in order `code`, `status`. Status is one of `met`,
`unmet`, `invalid`, `unsupported`; no code repeats.

| Code | Applicable when | Status rule |
|---|---|---|
| `accepted_refs` | record loads | `invalid` if any reference fails HEAD resolution, else `met` |
| `cli` | CLI parse failure | `invalid` |
| `clean_workspace` | step 5 | `met` only under the clean-workspace rule, else `unmet` |
| `decision_provenance` | record loads | `invalid` unless every decision meets its direct-parent rule, else `met` |
| `fresh_evidence` | step 5 | `met` only when assessment `HEAD` is a record-carrier commit and all evidence is at its parent `C`, else `unmet` |
| `fresh_checkpoint` | step 5 | `met` only when assessment `HEAD` is a record-carrier commit, checkpoint head is its parent `C`, and workspace base is an ancestor of `C`, else `unmet` |
| `git_objects` | record loads | `invalid` unless every commit ID resolves, else `met` |
| `hygiene` | record loads | `invalid` unless all field/file rules pass, else `met` |
| `issue_directory` | record loads | `invalid` unless checkpoint issue equals directory, else `met` |
| `record` | selected directory is absent or cannot be opened | `invalid` |
| `schema` | record loads | `invalid` unless all TOML/JSON schema rules pass, else `met` |
| `transition` | step 4 | `invalid` unless flags and matrix match, else `met` |
| `transition_evidence` | step 5 | `met` only under the evidence rule, else `unmet` |
| `transition_fields` | step 5 | `met` only under the field rule, else `unmet` |

The `outcome` is `invalid` if any applicable code is invalid, `unsupported` at
step 3, `unmet` if any step-5 code is unmet, otherwise `pass`. Exit codes are
0 `pass`, 2 `unmet`, 3 `invalid`, and 4 `unsupported`. The commands never
mutate records, docs, memory, GitHub, labels, approvals or Git, nor infer
approval or turn an observation into a normative requirement.

## Acceptance criteria

#120 owns implementation and fixture tests. Until it lands these are
requirements, not existing-tool claims.

| # | Given / When / Then | Planned verification |
|---|---|---|
| A1 | A valid `probe → spike` record is assessed from a clean record-carrier commit with initial regular-blob record files | Exit 0 and exact pass JSON without tracked-file changes | #120 temporary-Git fixture + clean `git diff --exit-code` |
| A2 | Every scalar/table/array boundary, unknown field, duplicate ID, bad reference/timestamp/Git object, mismatched issue directory, cross-revision decision or decision cycle is supplied | Exit 3 identifies only safe metadata | #120 fixtures |
| A3 | A forbidden raw-output-like field, environment-like assignment, credential-like value, source delimiter/snapshot, disallowed command syntax or unknown file/field is supplied | Hygiene/evidence validation fails without echoing it | positive and negative #120 fixtures covering every permitted field class and detector pattern |
| A4 | Evidence or checkpoint not at the record-carrier parent, a root/merge/non-regular carrier, a carrier diff that touches another path or changes an outside-path mode, a non-ancestor workspace base, or dirty/index/untracked workspace is assessed | Exit 2 with the exact `fresh_evidence`, `fresh_checkpoint`, `transition_evidence` and/or `clean_workspace` obligation vector | #120 temporary-Git fixture |
| A5 | A permitted transition is assessed with a selected issue and matching maturity fields | Stable complete JSON and no writes | #120 fixture + clean diff |
| A6 | Candidate/production is requested | Exit 4 unsupported and no writes | #120 fixture + clean diff |
| A7 | An accepted reference is missing, non-Accepted, malformed, or differs between record commit and HEAD | Exit 3 resolves only current HEAD accepted documents | #120 fixtures |
| A8 | Future pilot implementation is reviewed | No daemon/hook/CI/cloud/embedding/Beads/Basic-Memory integration exists | #120 review/diff |
| A9 | Malformed CLI, absent record and each first step-2 failure is assessed | Exact specified compact invalid JSON, exit 3 and no writes | #120 fixtures |

## Open questions

None. Expansion requires a new accepted ADR/specification after pilot evidence
is reviewed.
