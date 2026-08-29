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

1. Run a **local, Git-tracked, metadata-only** pilot under
   `.agent/work/<github-issue>/`. It covers only `probe → spike → prototype`;
   it makes no candidate or production claim.
2. Records use the TOML checkpoint and append-only JSONL evidence schemas in
   [SPEC 0008](../specs/0008-agent-sdd-pilot-governance.md). They describe
   task state/evidence, never normative requirements or current source files.
3. Promotion is **report-only**. A future local validator may report met or
   missing obligations, but never edits maturity, docs, GitHub, memory, or
   approval metadata.
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

- A fresh agent can recover verified task context without trusting stale source
  snapshots or model summaries as current truth.
- Records are reviewable in ordinary Git history with a deliberately small
  privacy and retention surface.
- #120 can implement a deterministic validator without network or a service.

### Bad

- Evidence selection remains deliberate work; no automatic recall/search is
  promised.
- Records can be stale, so observations and commands are commit-scoped and
  live Git/files/tests always win.
- The pilot does not retroactively add L0–L6 traceability to helm.

### Neutral

- GitHub remains the Symphony work graph. Records are issue-keyed but never
  duplicate labels, owners, dependencies or status.

## Reversal

Low. Remove pilot records, the future validator and its routing documentation;
no production component or external account depends on them. Reconsider only
after two real iterations show no handoff value or a deterministic retrieval
gap that paths, commands, IDs and Git revisions cannot cover.

## Guard

*Planned (#120):* `helm-sdd schema-check`, `evidence-check`, `gate`, and
`hygiene-check` fixtures reject malformed records, raw-output-like fields,
stale evidence and every transition beyond report-only pilot scope. Until then,
review of this spec is the guard; no tool claims to enforce it yet.

## Needs a human

None. Candidate/production promotion, L0–L6 graphs, automatic capture,
semantic retrieval, external memory, hooks and CI enforcement need new ADRs
and accepted specifications.

