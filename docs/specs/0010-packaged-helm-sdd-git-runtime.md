# SPEC 0010 — Packaged `helm-sdd` Git runtime

- **Status:** Accepted (2026-08-29)
- **Milestone:** M0 (internal packaging correctness; it does not block desktop MVP behaviour)
- **Decision:** [ADR 0016](../adr/0016-packaged-helm-sdd-carries-git.md)
- **Issue:** [#146](https://github.com/Pipeliner/realms-de/issues/146)

## Scope

This specification covers only the Nix-installed `helm-sdd` executable. The
validator is a read-only local tool for the agent-SDD pilot; it uses Git to
resolve and inspect the caller's existing worktree. It does not cover
`helm-session`, other Helm binaries, non-Nix source builds, Git installation on
the host, network access, or any automatic memory/record capture.

## Runtime contract

1. The Nix `helm` output SHALL install `bin/helm-sdd` and wrap that executable
   so its runtime search path contains the pinned `pkgs.git` binary directory.
2. `helm-sdd gate` and `helm-sdd promote --dry-run` SHALL retain the semantics,
   output, exit codes, and read-only behaviour specified by SPEC 0008. The
   wrapper supplies Git; it does not bypass Git validation.
3. A caller's inherited `PATH` SHALL NOT be required for Git discovery by the
   Nix-installed `helm-sdd`.
4. The `helm-session` wrapper and its shared desktop runtime dependency list
   SHALL NOT gain Git solely for `helm-sdd`.
5. A valid result still requires an existing local Git repository containing
   the relevant commits and record-carrier state. The wrapper SHALL NOT create,
   mutate, fetch, clone, or otherwise provision that repository.

## Acceptance criteria

| # | Given / When / Then | Verification |
|---|---|---|
| A1 | Given the Nix `helm` output and a valid, clean local record-carrier fixture, when the installed `helm-sdd gate --issue 120 --from probe --to spike` is invoked with an otherwise unusable caller `PATH`, then it exits 0 and emits the canonical SPEC 0008 pass JSON | Planned #146 Nix package regression |
| A2 | Given the installed `helm-sdd` wrapper, when it runs A1, then Git is resolved from the package closure rather than the caller environment | Planned #146 Nix package regression's empty-PATH invocation |
| A3 | Given the Nix output, when its generated `helm-session` wrapper is inspected, then it does not include the pinned Git store bin directory | Planned #146 Nix package regression |
| A4 | Given a normal source-workspace validation, when the existing gate fixtures run, then its established Git-backed and read-only semantics remain unchanged | `crates/helm-agent-sdd/tests/gate.rs` (47 fixtures at acceptance) |

## Open questions

None. A future non-Nix host-prerequisite interface requires a new ADR/spec and
a safe, explicit dependency diagnostic before it can be claimed as supported.
