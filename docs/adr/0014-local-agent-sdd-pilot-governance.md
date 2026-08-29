# ADR 0014 — Local agent-SDD pilot records are tracked but non-authoritative

- **Status:** Accepted (2026-08-29)
- **Deciders:** helm maintainers, repo owner
- **Supersedes / Superseded by:** —

## Context

helm needs concise, inspectable handoffs between human and agent sessions. The
repository already separates authority: accepted `docs/` and ADRs state product
truth, GitHub issues organise public work, tests/CI establish executable
behaviour, and `.claude/memory/` carries curated working knowledge. A handoff
record must not become another specification, retain secrets/tool output, or
add a background service to the MVP.

## Decision

1. Adopt a **local, Git-tracked, metadata-only** pilot under
   `.agent/work/<github-issue>/`. It covers only `probe → spike → prototype`;
   it makes no candidate or production claim. #120 supplies its validator and
   fixtures.
2. Records use the TOML checkpoint and refreshable current-snapshot JSONL
   evidence schemas in [SPEC 0008](../specs/0008-agent-sdd-pilot-governance.md).
   The current snapshot is replaced only with deliberately recaptured evidence
   at its new carrier parent; prior snapshots remain in ordinary Git history.
   Records describe task state/evidence, never normative requirements or
   current source files.
3. Promotion is **report-only**. The local validator may report met or missing
   obligations, but never edits maturity, docs, GitHub, memory, or approval
   metadata.
4. Do not add a daemon, scheduler, lifecycle hook, passive transcript/tool
   capture, cloud service, embeddings/vector retrieval, external database,
   OpenSpec, Sphinx-Needs, Serena, or CI change. Beads stays untouched. Basic
   Memory, if used, remains manual, local, curated and non-authoritative.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Automatically retain transcripts and tool output | Maximum recall without curation | It can retain secrets, private paths and stale source snapshots; that is too much surface and authority for an MVP pilot |
| Install the full spec/memory stack now | OpenSpec, Sphinx-Needs and semantic tools may help later | They introduce overlapping authorities before the repository measures a handoff problem |
| Use only GitHub comments | Public and already available | Free-form comments cannot reliably enforce freshness, structured validation or concise resumption |

## Consequences

### Good

- The implemented and verified contract lets a fresh agent recover task
  context without trusting stale source snapshots or model summaries as
  current truth.
- The records are reviewable in ordinary Git history with deliberately
  constrained metadata fields. Git history and existing clones retain prior
  content: deleting a record stops future use but does not reliably erase it.
- #120 has a deterministic, network-free and service-free validator contract.

### Bad

- Evidence selection will remain deliberate work; no automatic recall/search is
  promised.
- Records can be stale, so observations and commands are commit-scoped and
  live Git/files/tests always win.
- The pilot does not retroactively add L0–L6 traceability to helm.

### Neutral

- GitHub remains the Symphony work graph. Records are issue-keyed but never
  duplicate labels, owners, dependencies or status.

## Reversal

Low. If implemented, remove pilot records, the validator and its routing
documentation; no production component or external account depends on them.
Historical Git objects and clones may still retain removed record content, so
sensitive material must never be recorded. Reconsider only after two real
iterations show no handoff value or a deterministic retrieval gap that paths,
commands, IDs and Git revisions cannot cover.

## Guard

*Implemented (#120):* read-only `helm-sdd gate` and
`helm-sdd promote --dry-run` fixtures reject malformed records,
raw-output-like fields, stale evidence and every transition beyond report-only
pilot scope.

## Needs a human

None. Candidate/production promotion, L0–L6 graphs, automatic capture,
semantic retrieval, external memory, hooks and CI enforcement need new ADRs
and accepted specifications.
