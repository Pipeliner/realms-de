# Helm Workspace Source Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retain and mechanically validate the immutable Helm workspace source archive and complete offline Cargo closure required before native package recipes may build Helm.

**Architecture:** `packaging/tool-sources/bundles/helm-workspace/` will contain exactly one digest-bound source archive plus independently digest-bound lockfile, source-replacement configuration, vendor archive, and license report. The existing Python linkage checker gains source-archive validation without weakening selected-tool checks. A POSIX fixture mutates a same-name archive and proves the checker and later package-path contract reject it before Cargo.

**Tech Stack:** Python 3.10, POSIX shell, Cargo 1.85, deterministic `tar`/`zstd` archives.

**Spec:** `docs/specs/0024-m1-private-tool-bundle.md` (Helm workspace source authority and B1/B2/B5).

## Global Constraints

- The canonical source authority is `packaging/tool-sources/bundles/helm-workspace/source.tar.gz` and is the only archive either native package path may unpack for Helm Cargo invocations.
- Its record binds SHA-256, repository commit, commit timestamp, and retained provenance/notice data; its root `Cargo.lock` matches the retained lockfile byte-for-byte.
- Cargo inputs use retained source replacement, complete vendor closure, lockfile checksums, dependency license report, and `--frozen --offline --locked`.
- Debian/RPM preparation and build must stage the retained authority, never invoke Git/create an archive/fetch, and run every Helm Cargo command with `--frozen --offline --locked`.
- No publication infrastructure or user-configuration integration is in scope.

---

### Task 1: Extend the linkage contract for a source-authoritative bundle

**Files:** Modify `packaging/tool-sources/check-bundle-linkage.py`, `packaging/tool-sources/test-bundle-linkage.sh`.

**Interfaces:** `check-bundle-linkage.py BUNDLE_ROOT` accepts a Helm-only exact record with `name = "helm-workspace"`, `source`, `source_sha256`, `source_archive_format = "tar.gz"`, `source_provenance`, `source_provenance_sha256`, `version`, `commit`, `commit_timestamp`, and the existing fully-bound lock/config/vendor/license fields. It validates a regular source archive whose unpacked root contains a byte-identical retained lockfile.

- [ ] **Step 1: Write failing source-authority fixtures**

Add fixture records for a source-authoritative bundle and cases for missing source archive, digest mismatch, provenance mismatch, unsafe archive member, multiple roots, and source-root lockfile mismatch. The same-name/different-digest case must expect `source SHA-256 mismatch`.

- [ ] **Step 2: Run the fixture to verify red**

Run: `packaging/tool-sources/test-bundle-linkage.sh`

Expected: failure because the checker does not yet recognize/verify the Helm source fields.

- [ ] **Step 3: Implement minimal source-archive validation**

Parse only quoted fields as today; require the exact Helm record field set; acquire `source.tar.gz` through a bundle-root-constrained, no-follow regular-file descriptor, `fstat` it once, copy bytes from that descriptor into a private temporary file while hashing, compare the completed staged copy digest to `source_sha256`, and never reopen the source pathname. Safely inspect and extract only the staged gzip tar copy; require one regular top-level root; reject absolute/traversal/duplicate/symlink/special members; compare `<root>/Cargo.lock` bytes with the retained lockfile. Add deterministic fixtures for replacement-after-open and symlink-swap; both must prove the named replacement is not accepted as staged input. Do not claim source-tar reproducibility: source identity is its bound digest and provenance.

- [ ] **Step 4: Run focused green verification**

Run: `packaging/tool-sources/test-bundle-linkage.sh && shellcheck packaging/tool-sources/test-bundle-linkage.sh && python3 -m py_compile packaging/tool-sources/check-bundle-linkage.py`

Expected: exit 0; each negative fixture is rejected for its asserted reason.

- [ ] **Step 5: Commit**

Commit: `test: bind Helm source archive to retained closure`

### Task 2: Create the retained Helm source and dependency closure

**Files:** Create `packaging/tool-sources/bundles/helm-workspace/{source.tar.gz,Cargo.lock,config.toml,vendor.tar.zst,licenses.tsv,bundle.toml,provenance.md}`; update `packaging/tool-sources/README.md`.

**Interfaces:** `bundle.toml` uses the checker’s fully-bound record schema; `provenance.md` records the exact source commit/timestamp and how the controlled intake produced the canonical digest-bound archive.

- [ ] **Step 1: Record an empty-closure failing check**

Run: `python3 packaging/tool-sources/check-bundle-linkage.py packaging/tool-sources/bundles/helm-workspace`

Expected: nonzero because the retained Helm workspace bundle does not yet exist.

- [ ] **Step 2: Generate intake only from current bound main commit**

Use an isolated temporary Cargo home and a clean checkout at the record-bound commit. Generate `source.tar.gz` once during intake, copy its root `Cargo.lock`, run `cargo vendor --locked --versioned-dirs`, generate a vendor-relative source replacement, create a license report from each vendored crate `Cargo.toml`, and normalize `vendor.tar.zst` using sorted names, epoch mtime, and numeric owner/group.

- [ ] **Step 3: Add the fully-bound record and provenance**

Calculate every SHA-256 after final artifact bytes exist. The record shall name only paths beneath its bundle root. The provenance record identifies the archive-generating commit and timestamp without treating a moving checkout as package input.

- [ ] **Step 4: Verify the retained bundle and an empty-cache build**

Run: `packaging/tool-sources/test-bundle-linkage.sh` and an isolated `CARGO_HOME` invocation from unpacked `source.tar.gz`, with the retained config/vendor staged at its root and sentinel `git`/network commands first on `PATH`: `cargo build --frozen --offline --locked --workspace`.

Expected: both exit 0 without registry/Git cache or attempted sentinel command.

- [ ] **Step 5: Commit**

Commit: `build: retain Helm workspace offline closure`

### Task 3: Consume the authority in both native package paths and prove B2

**Files:** Create `packaging/tool-sources/stage-helm-workspace.py`, `packaging/tool-sources/test-native-builds.sh`; modify `packaging/debian/rules`, `packaging/fedora/helm.spec`, and `.github/workflows/ci.yml`.

**Interfaces:** `stage-helm-workspace.py BUNDLE DESTINATION` invokes the same source-authority validation/staging logic, creates only `DESTINATION/source`, `DESTINATION/vendor`, and `DESTINATION/.cargo/config.toml`, and prints the absolute staged source path. The Debian source kit and RPM `Source0` kit carry `packaging/tool-sources/bundles/helm-workspace/` plus the staging helper and packaging metadata, but contain no second Helm workspace tree; both recipes stage Cargo exclusively from its canonical inner `source.tar.gz`. Neither path invokes Cargo until staging returns success.

- [ ] **Step 1: Write failing native-path fixtures**

Create `test-native-builds.sh` that builds a disposable Debian source kit and RPM source kit with only the retained bundle/helper/packaging metadata, substitutes a same-name/different-digest `source.tar.gz`, installs sentinel `cargo`, `git`, and network commands on `PATH`, then runs mandatory actual `debian/rules build` and test plus relevant actual `rpmbuild` preparation/build/check phases in a network namespace with empty `CARGO_HOME`. Shims may supplement argument/ordering assertions only. Assert source-digest refusal occurs before sentinel Cargo, and a valid bundle reaches Cargo only with `--frozen --offline --locked` and no sentinel Git/network call.

- [ ] **Step 2: Run red verification**

Run: `packaging/tool-sources/test-native-builds.sh`

Expected: failure because neither current native path stages/digest-verifies the retained Helm authority or uses the required Cargo flags.

- [ ] **Step 3: Implement one authoritative staging helper and recipe calls**

Implement the helper using the Task 1 no-follow staged-descriptor routine. Make both recipes construct their source kits without another Helm workspace tree, call the helper before their Helm build/test commands, set `CARGO_HOME` to an empty package-local directory, stage the retained source replacement and vendor tree at the authority root, and execute Cargo only there with all three required flags. Remove the RPM’s executable source-generation guidance rather than scanning comments; the fixture observes command execution.

- [ ] **Step 4: Run focused green and CI verification**

Run: `packaging/tool-sources/test-native-builds.sh && shellcheck packaging/tool-sources/test-native-builds.sh && git diff --check`.

Add the fixture to the existing package/CI gate so remote CI executes it; run the relevant local CI shell checks.

- [ ] **Step 5: Commit**

Commit: `build: stage Helm workspace authority in native packages`

## Self-review

Task 1 covers the accepted archive/record/lock equivalence and adversarial archive safety. Task 2 produces the actual fully retained closure and proves offline Cargo resolution with no cache or sentinel attempt. Task 3 consumes that exact authority in both existing native paths and provides the real network-disabled B2/substitution evidence. No task creates a release artifact, package repository, or public infrastructure.
