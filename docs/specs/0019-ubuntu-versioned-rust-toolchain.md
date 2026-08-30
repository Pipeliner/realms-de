# SPEC 0019 — Ubuntu versioned Rust toolchain guard

- **Status:** Accepted (2026-08-30)
- **Milestone:** M3
- **Issue:** [#104](https://github.com/Pipeliner/realms-de/issues/104)
- **Decisions:** [ADR 0010](../adr/0010-nix-flake-as-reference-build.md), [SPEC 0009](0009-fedora-44-pre-alpha-baseline.md)
- **Supersedes / Superseded by:** —

## Purpose

Ubuntu 24.04's default Rust toolchain is below helm's MSRV. Debian packaging
therefore must select a verified versioned toolchain before it invokes Cargo;
it must never silently fall back to an unrelated `cargo` on `PATH`.

## Scope

**In:** deterministic discovery of a complete Ubuntu versioned Rust toolchain,
an actionable missing-toolchain error, a focused resolver test, and a clean
Ubuntu 24.04 CI lane that builds the Debian package.

**Out:** selecting a Rust release for the project, changing the MSRV, vendoring
dependencies, or solving the separate River/wlroots packaging decision.

## Behaviour

1. The resolver searches `/usr/lib/rust-1.[89][0-9]/bin` and accepts only a
   directory containing executable `cargo` and `rustc`. If several complete
   candidates exist, it chooses the newest matching version deterministically.
2. `packaging/debian/rules` obtains that resolver result before the build and
   prepends it to `PATH`. If no complete candidate exists, Make stops before
   compilation with an error naming `/usr/lib/rust-1.[89][0-9]/bin`.
3. The existing Rust version guard remains the final assertion that the chosen
   compiler meets MSRV 1.85 before Cargo compiles the workspace.
4. Distro CI installs the Ubuntu 24.04 versioned toolchain and Debian build
   prerequisites, then builds the `.deb` from the repository's packaging
   directory. This job is the integration evidence for the real archive layout.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| A1 | Given a fixture root containing complete versioned Rust toolchains, when the resolver runs, then it emits the newest matching `/usr/lib/rust-X.Y/bin` path. | `packaging/debian/test-toolchain-path.sh` — `newest-complete-toolchain` |
| A2 | Given a fixture root without a complete versioned toolchain, when the resolver runs, then it fails and names `/usr/lib/rust-1.[89][0-9]/bin`. | `packaging/debian/test-toolchain-path.sh` — `missing-toolchain` |
| A3 | Given Debian rules with no resolver result, when Make evaluates them, then it fails before the Cargo build and names the expected versioned path glob. | `packaging/debian/test-toolchain-path.sh` — `rules-missing-toolchain` |
| A4 | Given the Ubuntu 24.04 distro lane, when it runs, then it installs `cargo-1.85` and `rustc-1.85` and completes `dpkg-buildpackage -us -uc -b -d`. | `.github/workflows/distro.yml` — `ubuntu-debian-package` |

## Failure modes

An incomplete or absent versioned directory is a packaging configuration error,
not a reason to use another compiler. The resolver's failure is intentionally
actionable and happens before the existing MSRV guard or Cargo dependency work.

## Open questions

None. The archive layout was verified in an Ubuntu 24.04 container and recorded
on [#104](https://github.com/Pipeliner/realms-de/issues/104).
