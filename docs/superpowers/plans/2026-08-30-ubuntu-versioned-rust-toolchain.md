# Ubuntu Versioned Rust Toolchain Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Debian packaging choose a verified Ubuntu versioned Rust toolchain and prove it in focused tests and a package-build CI lane.

**Architecture:** A small POSIX-shell resolver owns toolchain discovery and is the only new production interface. `debian/rules` consumes its result and fails explicitly if absent; a fixture test drives the resolver without changing the host filesystem. Distro CI independently builds the Debian package in Ubuntu 24.04.

**Tech Stack:** GNU Make, POSIX shell, Debian packaging, GitHub Actions.

**Spec:** `docs/specs/0019-ubuntu-versioned-rust-toolchain.md`

## Global Constraints

- Select only complete executable `cargo` and `rustc` toolchains under `/usr/lib/rust-1.[89][0-9]/bin`.
- Fail before Cargo when none is selected, naming that expected glob.
- Keep the existing MSRV 1.85 assertion.
- Do not add a daemon, service, external infrastructure, or new Rust-selection policy.

---

### Task 1: Test and implement deterministic toolchain discovery

**Files:** `packaging/debian/test-toolchain-path.sh`,
`packaging/debian/toolchain-path.sh`, and `packaging/debian/rules`.

1. Write fixture tests for newest complete selection, missing/incomplete
   candidates, and Make's explicit guard. Run them red before the resolver
   exists.
2. Add the minimal resolver and Make guard; run the fixture test green.

### Task 2: Add real Ubuntu package-build evidence

**Files:** `.github/workflows/distro.yml` and `packaging/nix/checks.nix`.

1. Add `ubuntu-debian-package`, installing `cargo-1.85`, `rustc-1.85`, and
   Debian build prerequisites before `dpkg-buildpackage -us -uc -b -d`.
2. Include the new shell scripts in the existing Nix ShellCheck check.
3. Run syntax and focused verification, then commit the code/spec/CI change.

### Task 3: Record the first real pilot iteration

**Files:** `.agent/work/104/checkpoint.toml`, `.agent/work/104/evidence.jsonl`,
and `.claude/memory/40-loop.md`.

1. Capture the fresh, clean result against the preceding code commit using only
   the two SPEC 0008 record files.
2. Run `helm-sdd gate --issue 104 --from probe --to spike`, record recovery,
   overhead, and limits as revisable operational memory, then commit the record
   separately and open the PR.

## Review

The plan covers every SPEC 0019 criterion: fixture selection/failure (A1-A3)
and real clean Ubuntu package evidence (A4). It introduces no placeholder,
policy decision, or artifact beyond the selected resolver, tests, CI check, and
pilot record.
