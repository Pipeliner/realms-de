---
title: Standing Rule — Scope Directions and Verify Evidence
tags: [process, evidence, scope, agent-operations]
type: note
permalink: experiences/standing-rule-scope-and-evidence
---

# Standing Rule — Scope Directions and Verify Evidence

This is a durable operating rule for work in realms-de. Acknowledging a
direction is not the same as carrying it out. Each instruction must be judged
for its actual scope and recorded in the authority that can preserve it: the
conversation for a transient choice, task state for resumable work, `AGENTS.md`
for repository-wide process, an accepted specification or ADR for behavior and
architecture, and a tracked issue or external system for coordination. The
resulting state must then be checked with authoritative evidence.

## Observations

- [decision] Treat “Understood” as an acknowledgement only, never as proof of completion.
- [requirement] Classify each direction as conversational, task-local, repository policy, specification, or external side effect before recording it.
- [procedure] Record the direction at the narrowest durable authority that governs its scope.
- [requirement] Verify files, tests, CI, issue state, or runtime behavior before claiming the direction was handled.
- [lesson] An unverified status restatement is not progress; evidence must change authoritative state or establish the next action.

## Relations

- implements [[Codex working agreement]]
- relates_to [[Verification Before Completion]]
- relates_to [[Graduated Agentic SDD and Operational Memory]]
