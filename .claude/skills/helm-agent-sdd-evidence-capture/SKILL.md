---
name: helm-agent-sdd-evidence-capture
description: "Use when manually recording one fresh command result, file observation, or decision as minimal evidence for a local agent-SDD pilot checkpoint under SPEC 0008."
---

# Capture minimal local agent-SDD pilot evidence

This procedure is governed by SPEC 0008. Evidence is an evidence-backed
historical observation, not a source snapshot, product requirement or current
repository fact.

## Do not use when

Do not use when preserving command output, a transcript, environment values,
source text, secrets, customer material or a stale result. Those do not belong
in this pilot record.

## Capture one fresh fact

1. Rerun the command or reread the file at the current parent commit.
2. Add the smallest valid JSONL object to
   `.agent/work/<issue>/evidence.jsonl`, using the allowed `command`,
   `file-observation` or `decision` shape from SPEC 0008.
3. Keep the record to its allowed ASCII metadata fields and cite its evidence
   ID from `.agent/work/<issue>/checkpoint.toml` where required.
4. For a decision, cite earlier non-decision evidence from the same snapshot.
5. Make the separate record-carrier commit, then use the existing read-only
   assessment:

   ```text
   helm-sdd gate --issue <issue> --from <maturity> --to <maturity>
   helm-sdd promote --dry-run --issue <issue> --from <maturity> --to <maturity>
   ```

## Never record

Do not record command output, stderr, environment values, shell history,
secrets, credentials, tokens, absolute paths, customer material, source snapshots
or copied source fragments. Do not trust a prior result without a
fresh live check. A passing report does not promote maturity or create a
product contract.

## Boundaries

`helm-sdd` currently exposes only `gate` and `promote --dry-run` for the
pilot.
Pilot exclusions: no hook, daemon, scheduler, CI job, service, external database, embedding search or third-party integration.
