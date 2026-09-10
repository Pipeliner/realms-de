# SPEC 0025 — Realm project namespace

- **Status:** Accepted (2026-09-10)
- **Milestone:** MVP
- **Issue:** [#209](https://github.com/Pipeliner/realms-de/issues/209)
- **Decision owner:** repository owner, directly approved in the active development session

## Purpose

Realm has one project identity before further MVP code lands. This is a direct
pre-launch cutover: there is no compatibility namespace, migration layer, or
historical-name documentation because no released users depend on one.

## Contract

The product and display name SHALL be `Realm`. First-party lowercase identifiers
SHALL use `realm`; uppercase constants and environment variables SHALL use
`REALM`; Rust libraries SHALL use `realm_*`; and executable names SHALL use the
`realm` prefix, including `realmctl`, `realm-sdd`, `realm-session`, `realm-wm`,
and `realm-bar` where those programs exist. The control CLI has exactly one
spelling: the installed `realmctl` binary. A spaced command or forwarding alias
is not part of the product.

All tracked first-party paths and file bytes SHALL be free of the retired
four-byte ASCII namespace token, case-insensitively. The repository guard SHALL
construct that token from numeric byte values `(104, 101, 108, 109)` so its own
source does not preserve the spelling. The check includes binary tracked files.

The cutover SHALL cover Cargo packages/imports/binaries, commands and
diagnostics, project-defined environment variables, config/state/runtime paths,
session and systemd assets, desktop metadata, packaging, CI, tests, docs, skills,
memory, and retained first-party source bundles. An external proper name that
cannot satisfy the tracked-byte guard SHALL be described generically instead of
preserved as repository text.

GitHub's default branch SHALL be `main`. Active issue and pull-request metadata
SHALL use Realm terminology. Historical Git commits are immutable evidence and
are outside this working-tree identity contract.

## Acceptance criteria

| ID | Criterion | Evidence |
|---|---|---|
| A1 | Every tracked path and tracked file, including binary files, passes the case-insensitive retired-token guard. | `scripts/check-project-namespace` |
| A2 | The complete Rust workspace builds and tests using only Realm package, crate, binary, and environment-variable names. | `cargo test --workspace --all-targets` |
| A3 | Packaging and session contract tests refer only to Realm paths, units, commands, and payloads. | repository shell tests and remote native-package CI |
| A4 | The retained first-party workspace source archive expands under a Realm root and itself passes A1. | `packaging/tool-sources/test-bundle-linkage.sh` plus archive inspection |

## Out of scope

- compatibility aliases for an unreleased name;
- rewriting Git history;
- a generalized namespace-migration framework;
- post-MVP security hardening unrelated to correctness of this cutover.
