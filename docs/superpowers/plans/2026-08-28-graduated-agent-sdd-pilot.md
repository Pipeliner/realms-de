# Graduated Agent SDD Pilot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** Add a local, metadata-only checkpoint and evidence pilot without creating a second product-truth, task-truth, or hosted-memory system.

**Architecture:** Existing specs/ADRs, GitHub issues, tests, and CI retain their present authority. A small internal Rust CLI reads and reports on tracked task records under `.agent/work/<github-issue>/`; the first pilot supports `probe`, `spike`, and `prototype` only. Beads stays untouched and Basic Memory remains manual/local.

**Tech Stack:** Rust 2021/MSRV 1.85, existing workspace serde/serde_json/toml/clap dependencies, Markdown.

**Spec:** A new accepted governance spec and ADR, created after #3's correction/reratification gate.

## Global Constraints

- Resolve and independently re-review #3 before framework behavior is implemented.
- No daemon, cloud service, hook, passive capture, embeddings, OpenSpec, Sphinx-Needs, Serena, or new CI workflow in this pilot.
- GitHub issues remain the only Symphony work graph; do not mutate Beads or mirror issues.
- Every tracked evidence record is metadata only: no raw output, environment values, credentials, arbitrary transcript, or customer data.
- `helm-sdd` validates and reports; it never promotes product contracts or alters canonical status.

---

### Task 1: Correct the ADR ratification set

**Files:**
- Modify: affected `docs/adr/*.md`, affected specs and portal policy only where the #3 audit requires truth alignment.

- [ ] Apply only the verified #3 correction set.
- [ ] Run documentation/static consistency checks and commit the correction.
- [ ] Obtain independent adversarial re-review; do not mark #3 ratified without a clean verdict.

### Task 2: Preserve and verify the supplied research

**Files:**
- Create: `docs/research/2026-08-28-graduated-agentic-sdd-source.md`
- Create: `docs/research/2026-08-28-graduated-agentic-sdd-verified.md`

- [ ] Preserve the supplied source verbatim with a non-normative/archive notice.
- [ ] Produce a concise verified companion using primary-source links and record corrections.
- [ ] Commit the research records without changing product behavior.

### Task 3: Define pilot governance

**Files:**
- Create: a new ADR and accepted specification under `docs/adr/` and `docs/specs/`.
- Modify: `docs/specs/README.md`, `CLAUDE.md`, and `.claude/memory/40-loop.md` only for concise routing.

- [ ] Define authority, scope, maturity terms, metadata-only data classification, and promotion-report semantics.
- [ ] Define tracked checkpoint and evidence schemas plus acceptance criteria.
- [ ] Keep candidate/production graph enforcement, CI, hooks, and external tooling explicitly out of scope.
- [ ] Commit the accepted governance contract.

### Task 4: Add the deterministic local validator

**Files:**
- Modify: root `Cargo.toml`.
- Create: `crates/helm-agent-sdd/` with library, `helm-sdd` binary, and tests.
- Create: crate-local record templates materialized into temporary clean Git repositories by tests.

- [ ] Write failing tests for duplicate evidence IDs, invalid lifecycle transition, stale commit evidence, prohibited evidence fields, and exact report output.
- [ ] Implement only read-only `gate` and `promote --dry-run`; their validation subsumes schema, evidence and hygiene checks.
- [ ] Run focused tests, then the workspace trio, and record test names in the governance specification.
- [ ] Commit the crate and fixtures.

### Task 5: Pilot procedures and measurement

**Files:**
- Create: three focused skills beside existing `.claude/skills/` procedures.
- Modify: the skills index and loop memory with concise invocation/measurement guidance.

- [ ] Add bootstrap, checkpoint, and evidence-capture procedures with positive and negative triggers.
- [ ] Use the records during two real Symphony iterations after #3 closes.
- [ ] Record recovery observations; add neither CI nor external tooling until review of those measurements.
- [ ] Commit procedure documentation.
