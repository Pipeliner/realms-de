# M1 Private Tool Bundle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and verify the offline, Helm-private Yazi/`ya`/Starship package route required by SPEC 0024.

**Architecture:** `packaging/tool-sources/` becomes the checked, typed inventory for three independently retained Cargo closures: Yazi, Starship, and Helm workspace. Native recipes consume only those inputs and install private tool binaries; systemd units receive a Helm-only PATH without mutating the user manager. A shell fixture proves policy and ownership first, then hermetic package/runtime integration proves it end to end.

**Tech Stack:** Python 3.10-compatible validation, POSIX shell/Bash, TOML, Cargo 1.85, Debian rules, RPM spec, systemd user units, Rust template renderer.

**Spec:** `docs/specs/0024-m1-private-tool-bundle.md`.

## Global Constraints

- Build Yazi `25.4.8` and Starship `1.23.0` with Rust 1.85 using `cargo --frozen --offline --locked`.
- Retain and hash each source archive, lockfile, vendor tree, source-replacement config, provenance, notices, and dependency license report.
- Every Cargo command in Debian/Fedora recipes, including Helm workspace commands, is offline and uses retained closure inputs.
- Install Helm-owned tools only at `/usr/lib/helm/bin/{yazi,ya,starship}`; never own `/usr/bin` tool paths.
- Private PATH reaches direct and systemd Helm launches but never systemd-user/DBus environment import, including `HELM_IMPORT_PATH=1`.
- Keep A3/A4, full user configuration integration, and public infrastructure out of scope.

### Task 1: Strict source-bundle inventory and B1/B5 fixtures

**Files:** Modify `packaging/tool-sources/check-intake.py`, `manifest.toml`, `README.md`; create `test-bundle-linkage.sh`, `bundles/`; extend `test-intake.sh`.

**Interfaces:** `check-intake.py ROOT` exits nonzero unless every declared bundle has exactly its archive, lockfile, vendor directory, `.cargo/config.toml`, provenance fields, and report; `test-bundle-linkage.sh` mutates one field per fixture.

- [ ] Write fixtures that first fail for an unrepresented Cargo.lock source, a missing vendor checksum entry, a source-replacement config without `replace-with`, and a dependency row without a license/notice link.
- [ ] Run `packaging/tool-sources/test-bundle-linkage.sh`; observe each failure before extending the validator.
- [ ] Extend the manifest schema with `kind`, `commit`, `commit_timestamp`, `lockfile`, `vendor`, `cargo_config`, and `license_report`; validate regular non-symlink files/directories and exact inventory.
- [ ] Add explicit bundle records for Helm workspace, Yazi 25.4.8, and Starship 1.23.0 only after their retained bytes and reports exist.
- [ ] Rerun `test-intake.sh`, `test-bundle-linkage.sh`, and `python3 check-intake.py`; run `shellcheck` on both tests.
- [ ] Commit `build: validate offline Cargo bundle closure`.

### Task 2: Retain selected closures and prove offline Cargo builds

**Files:** Create checked-in selected archives/locks/vendor/config/reports under `packaging/tool-sources/bundles/`; create `test-offline-cargo-build.sh`.

**Interfaces:** `test-offline-cargo-build.sh ROOT` creates empty `CARGO_HOME`, sets `CARGO_NET_OFFLINE=true`, copies each bundle to two different temporary directories, and invokes its named package command.

- [ ] Write the test so it fails when a fixture has no vendor config and when a sentinel `git`/network executable is placed before PATH.
- [ ] Generate each vendor tree from its retained lockfile, include Cargo's checksum files, and write source replacement to the bundle-local vendor tree.
- [ ] Use selected commands: `cargo build --frozen --offline --locked --release --package starship`; `cargo build --frozen --offline --locked --release --package yazi-fm --package yazi-cli`; and Helm workspace build/test commands.
- [ ] Set the version-pinned Yazi `vergen` metadata from recorded commit SHA/timestamp; build twice in distinct paths and compare the declared normalized outputs.
- [ ] Rerun the offline test with empty registry/Git caches and save test names in SPEC 0024 B1/B2/B5.
- [ ] Commit `build: retain M1 offline Cargo closures`.

### Task 3: Native package recipes and private file ownership

**Files:** Modify `packaging/debian/rules`, `packaging/debian/control`, `packaging/fedora/helm.spec`; create `packaging/tool-sources/test-native-builds.sh`.

**Interfaces:** package recipes receive `HELM_TOOL_SOURCES=$CURDIR/packaging/tool-sources`; each uses only bundle config/vendor paths and installs resulting binaries below `/usr/lib/helm/bin`.

- [ ] Add a failing static fixture asserting each Cargo invocation includes `--frozen --offline --locked`, and that neither recipe contains fetch commands or `/usr/bin/yazi`, `/usr/bin/ya`, `/usr/bin/starship` install paths.
- [ ] Change Debian and RPM build stages to unpack bundle sources, export isolated Cargo homes/configs, build selected binaries, and install exactly the private paths.
- [ ] Add actual Debian `debian/rules build` and RPM `%build` fixture runs with network namespace disabled/blocked and empty Cargo caches; inject a fetch command into copied recipes and assert failure.
- [ ] Verify package file inventories include private files and exclude public collisions.
- [ ] Run `shellcheck packaging/tool-sources/test-native-builds.sh` and the scoped package fixtures.
- [ ] Commit `build: package private M1 tool binaries offline`.

### Task 4: Session-scoped PATH delivery and B3 regression tests

**Files:** Modify `packaging/session/helm-session`, `packaging/systemd/helm-wm.service`, `packaging/systemd/helm-bar.service`, Nix unit projections; create `packaging/session/test-private-tool-path.sh`.

**Interfaces:** direct launch exports `/usr/lib/helm/bin:$PATH`; Helm-owned systemd units use an explicit unit-level PATH; exported `SESSION_ENV_VARS` never contains the private prefix.

- [ ] Write failing shell fixtures for direct lookup, unit PATH, absence from `systemctl --user import-environment`/`dbus-update-activation-environment`, and `HELM_IMPORT_PATH=1` importing the caller path without the private prefix.
- [ ] Capture caller PATH before private prefixing; make the direct route inherit the private directory only locally.
- [ ] Add the explicit PATH environment to Helm units and identical Nix projections; do not add it to global manager imports.
- [ ] Run fixture and `shellcheck packaging/session/helm-session packaging/session/test-private-tool-path.sh`.
- [ ] Commit `feat: scope Helm private tool path to session`.

### Task 5: Yazi schema migration and selected-runtime configuration checks

**Files:** Modify `configs/templates/yazi-theme.toml`; create `packaging/tool-sources/test-tool-configs.sh` and a strict schema allowlist.

**Interfaces:** rendered Yazi output contains `[manager]`, `[mode]`, `perm_sep`, `perm_type`, `perm_read`, `perm_write`, `perm_exec`; it contains no legacy names. Starship fixture invokes selected binary with controlled HOME/cwd/keymap/status/cmd duration.

- [ ] Add a failing rendered-template check for every legacy key and for the six mode variants/permission names required by SPEC 0024.
- [ ] Rewrite `[mgr]` to `[manager]`; migrate status modes to `[mode]`; map permission styles and omit `permissions_s` while using its palette role for `perm_sep`.
- [ ] Render `templates()` through Helm's Rust renderer into `YAZI_CONFIG_HOME/theme.toml`, run selected Yazi in terminal-capable controlled environment, and assert canonical field consumption.
- [ ] Run selected `starship prompt` with `STARSHIP_CONFIG`, assert no stderr diagnostic and the rendered prompt sigil rather than merely nonempty output.
- [ ] Run the strict schema and runtime fixtures; commit `fix: migrate Helm Yazi theme for selected bundle`.

### Task 6: Full evidence, review, and gradual merge

- [ ] Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, all tool-source/session fixtures, and `git diff --check`.
- [ ] Have adversarial reviewers inspect source provenance, package ownership/PATH leakage, template semantics, and offline test bypasses; resolve every P0/P1 finding with fresh tests.
- [ ] Update SPEC 0023 A2 and SPEC 0024 B1–B5 test references only with exact passing test commands.
- [ ] Commit/push each cohesive slice, open PRs using `scripts/gh-body-file`, and merge only after all remote checks pass.

## Self-review

Tasks 1–2 cover B1/B2/B5 closure, licensing, offline cache, and reproducibility requirements. Task 3 covers actual Debian/RPM paths and ownership. Task 4 covers B3 direct/systemd path isolation. Task 5 covers B4's strict schema and real runtime requirements. Task 6 requires full evidence and preserves the A3/A4 boundary. No task publishes infrastructure or creates a user configuration integration claim.
