# ADR 0019 — Nix CI caching is optional and must not block verification

- **Status:** Accepted (2026-08-30)
- **Deciders:** helm maintainers
- **Related:** [ADR 0010](0010-nix-flake-as-reference-build.md)

## Context

The GitHub Nix workflow used a retired cache action which now requires a
FlakeHub account.  Without those credentials it fails before either the
KVM-capable VM check or the documented no-KVM fallback runs.

## Decision

The Nix reference build and its VM evidence are required; a remote cache is
not.  CI therefore uses no cache action that needs external credentials.  The
workflow must run the same KVM-detected full-check/no-KVM fallback policy from
ADR 0010 regardless of cache availability.

## Consequences

Builds may be slower, but a missing optional service cannot turn verification
into a false red result.  A future cache may be added only when it is
credential-free for this public repository or when its credentials and failure
mode are explicitly reviewed.

## Guard

The `nix flake check` GitHub job must reach `Run the reference build`; CI
failures in cache setup are a regression.
