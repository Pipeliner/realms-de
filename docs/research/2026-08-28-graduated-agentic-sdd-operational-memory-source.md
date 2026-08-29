# Designing and Validating a Graduated Agentic SDD and Operational-Memory System

> **Partial source-preservation record — 2026-08-28.** This is a
> non-normative preservation of the operative material recoverable from the
> owner-supplied report. It is **not a complete verbatim transcript**: the full
> conversation payload was not available as a repository artifact when this
> record was created. Conversation-local citations such as `turn21search0` are
> preserved where available; they are not resolvable repository citations and
> must not be used as verification. Replace this record with the complete
> verbatim source when the owner supplies it as an importable artifact.

## Executive summary

As of **August 28, 2026**, the strongest architecture for agentic software
development is not a single “memory product” or a single SDD framework. The
evidence from coding-agent research and current open-source tooling points
toward a **layered system in which repository truth, task state, operational
experience, procedural skills, executable tools, tests, and CI have different
persistence and authority levels**.

The recommended design is therefore:

> **Git repository = canonical truth; spec graph = normative intent; tests/CI
> = executable guarantees; Beads = work state; operational memory =
> evidence-backed experience; skills = reusable procedures; live tools =
> current-world observation.**

For the **graduated SDD lifecycle**, treat `probe → spike → prototype →
candidate → production` as an assurance axis, separate from the specification
abstraction axis `L0 → L6`. A high-level L1 requirement can already be
production-stable while an L4 implementation design refining it is still a
prototype. Conversely, a temporary spike may have an extremely detailed L4
design without creating a durable L1 product commitment.

| Level | Meaning | Primary reviewer |
|---|---|---|
| L0 | intent, outcomes, goals, non-goals | human |
| L1 | system/product requirements and guarantees | human |
| L2 | observable behavior, scenarios, contracts | human for semantic changes |
| L3 | architecture, components, APIs, data flows | agent + risk-based human review |
| L4 | detailed implementation specification | primarily agent |
| L5 | verification obligations and conformance evidence | machine + agent |
| L6 | implementation and runtime evidence | machine + agent |

The key engineering principle is **promotion rather than accumulation**:

```text
conversation / shell trace
        ↓
episode
        ↓
operational experience
        ↓
procedure
        ↓
SKILL.md
        ↓
script / typed tool
        ↓
test / linter / schema / CI invariant
```

Anything important should move toward a representation with less dependence on
LLM memory. A recurring fact becomes structured knowledge; a recurring
procedure becomes a skill; a recurring shell pipeline becomes a helper; an
invariant that must always hold becomes a test, type rule, schema, hook, or CI
check.

## Architecture recorded in the supplied report

The report proposes a minimal stack of **OpenSpec + Sphinx-Needs + Beads +
Superpowers**, optionally adding Basic Memory for curated durable knowledge and
Serena for live semantic navigation. It recommends deterministic identifiers,
paths, commits, error strings, commands, subsystem labels, and functional task
phase before considering embeddings.

It gives three distinct persistence/authority rules:

1. Memory preserves what an agent learned; live tools establish what is true
   now.
2. An observation, hypothesis, candidate requirement, approved behavior, and
   supported contract are distinct states; an observation must not silently
   become a normative requirement.
3. Promotion is a mechanical, evidence-backed state transition, with required
   human approval for semantic L0–L2 contracts.

It proposes checkpoint records containing a durable goal, acceptance criteria,
target maturity, workspace Git state, verified completed work, clearly labelled
hypotheses, rejected approaches, evidence references, and the next one to three
actions. It explicitly rejects transcript dumps and copied source snapshots.

It proposes that probes and spikes remain cheap, while candidate and production
work have complete traceability and verification obligations. Suggested pilot
metrics include re-derivation tax after handoff, traceability coverage,
false-promotion rate, stale-memory contradiction rate, skill routing
precision/recall, and checkpoint recovery latency. Its thresholds are stated to
be proposals, not literature-established values.

## Repository-bootstrap direction recorded in the supplied report

The report’s bootstrap handout directs an agent to inspect existing conventions
before acting, retain Git-tracked artifacts as canonical truth, keep `AGENTS.md`
short, checkpoint before compaction/handoff, and introduce a small repository
local validator before embedding search or passive memory capture. It explicitly
states that agent memory is never the sole source of truth for requirements,
APIs, current source contents, tests, security policy, or release state.

The proposed initial commands were `bootstrap`, `checkpoint`, `rehydrate`,
schema/evidence validation, graph lint/coverage, promotion gates, and hygiene
checks. The proposed rollout is incremental: measure first, add checkpoints,
trial traceability in one bounded area, add advisory promotion gates, then
evaluate operational-memory retrieval. Passive episode telemetry and semantic
search remain optional and evidence-gated.

## Archival caveat

The report supplied in conversation contains a much more extensive survey,
schemas, diagrams, examples, evaluation plan, and implementation handout. The
authoritative verbatim source is the owner-supplied conversation payload. This
tracked record preserves the report’s operative text and its citation caveat,
but deliberately does **not** convert its assertions, tool recommendations, or
thresholds into project commitments. The companion record supplies primary
sources for claims that were independently checked on 2026-08-28.
