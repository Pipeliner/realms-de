# Graduated agentic SDD and operational memory: verification companion

**Verified:** 2026-08-28<br>
**Status:** non-normative research record<br>
**Scope:** tool capabilities and implementation constraints only; no tool is
installed, initialized, configured, or enabled by this record.

## Verified claims

| Claim | Primary-source verification | Consequence for this repository |
|---|---|---|
| OpenSpec separates current specs from change artifacts, and archive updates current specs. | The [OpenSpec CLI](https://openspec.dev/docs/cli) documents `specs`, `changes`, validation, and archive; its [quickstart](https://openspec.dev/docs/quickstart) says archive updates main specs and moves the completed change to history. | Do not initialize it until its ownership boundary with accepted `docs/specs/` is specified. Archive must never be an unreviewed canonicalization mechanism. |
| Sphinx-Needs can model typed, linked engineering objects and validate fields/relations using schemas. | [Sphinx-Needs schema validation](https://sphinx-needs.readthedocs.io/en/stable/schema/) documents typed fields, link validation, local and network validation, and severities. | It is a possible future graph substrate, not a reason to add Sphinx or CI now. |
| Beads setup changes repository integration files. | [Beads setup documentation](https://github.com/gastownhall/beads/blob/main/docs/getting-started/ide-setup.md) says `bd setup codex` creates a skill, `AGENTS.md` section, and Codex hooks; the [project README](https://github.com/gastownhall/beads) says `bd init` can update `AGENTS.md` and integrations. | Do not run `bd init` or `bd setup`; GitHub issues remain Symphony task authority during the pilot. |
| Basic Memory needs explicit licensing and integration review. | [Basic Memory’s repository](https://github.com/basicmachines-co/basic-memory) publishes its license and describes local Markdown-backed knowledge plus MCP integrations. | Do not install, connect, or ingest into it. Any later adoption needs an explicit license/privacy decision. |

## Corrections and limits

1. Conversation tokens such as `turn21search0` are not stable public
   citations. They are preserved in the source record only; the table above is
   the independently source-linked material.
2. The source report’s tool maturity assessments, research-result summaries,
   and proposed numerical thresholds are not accepted repository facts. Their
   original papers/releases must be individually checked before relying on
   them for a decision.
3. A spec graph is not automatically a source of truth. This project already
   has accepted specifications and ADRs; any new graph must reference or
   replace a declared owner, never duplicate it.
4. Checkpoints and operational experiences may be useful task records, but
   they cannot establish product semantics or overrule live Git/filesystem/test
   evidence.

## Conservative research conclusion

The source report’s safe, testable hypothesis is worth piloting: a small,
tracked checkpoint/evidence format may reduce handoff re-derivation without
creating new infrastructure. The hypothesis remains unproven here. Any pilot
must be local, metadata-only, opt-in per issue, and measured against a baseline;
it must not start a daemon, add hooks, add cloud services, install tools, alter
CI, or change product behaviour.

Before a candidate/production gate is introduced, the project needs an accepted
specification for its artifact authority, privacy allowlist, retention,
freshness, and failure behaviour. That work belongs in the Symphony loop and
must follow the repository’s existing specification-first delivery order.
