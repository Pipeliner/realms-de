# SPEC 0023 — M1 Yazi and Starship source intake

- **Status:** Accepted (2026-08-31)
- **Milestone:** M1
- **Issue:** [#134](https://github.com/Pipeliner/realms-de/issues/134)
- **Decisions:** [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md),
  [ADR 0010](../adr/0010-packaged-helm-sdd-git-runtime.md)
- **Evidence:** [M1 provenance research](../research/2026-08-30-m1-yazi-starship-provenance.md)

## Purpose

Helm supports Yazi and Starship on all three M1 targets without a live upstream
download during package construction and without Helm-operated public package,
artifact, or signing infrastructure.

## Contract

1. NixOS continues to consume the repository's locked Nix inputs. Debian and
   Fedora package construction consumes only reviewed fixed inputs already
   retained with the release source or the reviewed repository revision; no
   required build target contacts an upstream service.
   For a Rust source build, the retained input is the tagged source archive
   **and its Cargo.lock-resolved dependency closure** (for example a reviewed
   `cargo vendor` tree plus the configuration selecting it). A top-level source
   archive alone is evidence for intake, not an offline-buildable package route.
2. Each retained Yazi or Starship input has a machine-readable record containing
   tool name, upstream version/tag, canonical source filename, SHA-256, source
   URL, license identifier, notice path, intake date, and the reviewer-approved
   replacement/rollback relationship. A digest mismatch, missing notice,
   duplicate tool/version, untracked retained file, or non-regular input is a
   refusal.
3. The retained inputs preserve the upstream MIT (Yazi) and ISC (Starship)
   notices. Helm does not claim that a checksum alone authenticates an upstream
   publisher; the review record names the identity/provenance evidence used at
   intake.
4. Updating an input is a reviewed source-intake change: add the new complete
   record and notice, verify its digest, run target availability fixtures, then
   retain the prior approved input until the replacement is proven. Rollback
   selects a retained prior input; it never fetches a URL.
5. This contract does not publish a package repository, key, binary, container,
   mirror, or network service. It does not claim Fedora COPR or an upstream APT
   repository as a supported Helm source. The native packages declare or build
   from the retained inputs; an active immutable Helm generation remains
   unchanged by a later package upgrade or rollback.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given an intake manifest and retained inputs, when validation runs, then every supported Yazi/Starship record has the required identity, digest, license notice and rollback relation and its bytes match the digest. | `packaging/tool-sources/test-intake.sh` |
| A2 | Given a Debian or Fedora package build definition, when its source policy check runs, then it has no live upstream acquisition path and reads only declared retained inputs, including the locked Cargo dependency closure required for any source-built Yazi or Starship binary. | Not yet implemented; `test-no-live-fetch.sh` is a partial negative guard, not A2 evidence. |
| A3 | Given NixOS, Debian and Fedora target declarations, when availability fixtures run, then each identifies a supported Yazi and Starship route and the configured version meets the recorded floor. | Not yet implemented; planned mappings are not availability evidence. |
| A4 | Given an installed immutable Helm generation, when package input update or rollback is selected, then its selected generation bytes are unchanged; only later launches may use the changed package/runtime. | Not yet implemented; requires generation-lifecycle evidence and a rollback fixture. |

## Non-goals

No public infrastructure, upstream fetching during package build, automatic
intake, binary-only trust shortcut, or unreviewed third-party repository is
introduced by this specification.

## Current implementation boundary

The first implementation increment retains and validates the two tagged source
archives and notices only. The Debian/Fedora entries in `targets.toml` are
planned mappings, not claims that an installable package route exists: source
closure retention, native integration, availability, and rollback fixtures
remain required by A2-A4 before either target is supported through this policy.
