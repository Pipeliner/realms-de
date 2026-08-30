# Helmctl Theme CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` task-by-task. Steps use checkbox syntax.

**Goal:** Ship the `helmctl theme apply`, `lint`, and `diff` executable surface required by SPEC 0002, SPEC 0006, and SPEC 0011.

**Architecture:** Add a small `crates/helm-ctl` binary crate which parses only the theme command group with `clap` and calls `helm_theme::{apply,diff,lint,load_palette}` in-process. It owns configuration-root and optional palette selection plus human diagnostics/exit mapping; it does not open an IPC socket, reload processes, alter the sealed-generation protocol, or implement the wider M2 CLI surface.

**Tech Stack:** Rust 1.85, `clap` derive from workspace dependencies, `helm-theme`, `tempfile` integration fixtures, `std::process::Command`.

**Spec:** `docs/specs/0002-theme-pipeline.md` A6/A9/A10; `docs/specs/0006-helm-ctl.md` §§2/4 and B1–B3/B15–B17; `docs/specs/0011-theme-activation-generations.md` G10–G12.

## Global Constraints

- The installed binary name is exactly `helmctl`; do not claim or install a `helm` alias.
- Theme commands are in-process and session-independent: they do not connect to IPC, require Wayland, signal, reload, notify, or mutate a session.
- Apply publishes only a sealed generation for future launches. It reports `Committed`, `CommittedWithCleanupPending`, or `OutcomeAmbiguous`; it never reports legacy written/unchanged/reloaded counts or elapsed-time success semantics.
- Diff is read-only: no generation-control initialization, recovery, lease, publication, pointer update, output write, or reload.
- Configuration root is `--config-root PATH` when supplied; otherwise `$XDG_CONFIG_HOME`, otherwise `$HOME/.config`. A missing `HOME` is a usage failure. `--palette PATH` is an explicit read-only lint input; apply/diff use the secure Helm-owned `helm/palette.toml` path until a separate accepted input-snapshot API exists.
- Exit codes: 0 success/no diff; 1 fatal lint or a non-empty diff; 2 usage/input-selection error; 6 `OutcomeAmbiguous`; library/I/O/generation refusal is 1 with a safe diagnostic.

---

### Task 1: Establish the binary crate and command parser

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/helm-ctl/Cargo.toml`
- Create: `crates/helm-ctl/src/main.rs`
- Create: `crates/helm-ctl/tests/theme_cli.rs`

**Interfaces:**
- Consumes: `clap::{Parser,Subcommand}` and `std::process::ExitCode`.
- Produces: executable `helmctl` with `theme { apply | lint | diff }`; `run(args, env) -> ExitCode` is unit-testable without process-global mutation.

- [ ] **Step 1: Write failing command-shape tests**

```rust
#[test]
fn rejects_missing_or_unknown_theme_verb() {
    assert_eq!(run_from(["helmctl", "theme"]), ExitCode::from(2));
    assert_eq!(run_from(["helmctl", "theme", "reload"]), ExitCode::from(2));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p helm-ctl rejects_missing_or_unknown_theme_verb`

Expected: FAIL because package `helm-ctl` does not exist.

- [ ] **Step 3: Add the workspace member and minimal parser**

```toml
[[bin]]
name = "helmctl"
path = "src/main.rs"
```

```rust
#[derive(Parser)]
struct Cli { #[command(subcommand)] command: Command }
#[derive(Subcommand)]
enum Command { Theme(Theme) }
#[derive(Args)]
struct Theme { #[command(subcommand)] command: ThemeCommand }
#[derive(Subcommand)]
enum ThemeCommand { Apply(RootArgs), Lint(LintArgs), Diff(RootArgs) }
```

- [ ] **Step 4: Re-run parser tests**

Run: `cargo test -p helm-ctl rejects_missing_or_unknown_theme_verb`

Expected: PASS; `helmctl theme --help` documents only `apply`, `lint`, and `diff`.

- [ ] **Step 5: Commit the isolated parser slice**

```bash
git add Cargo.toml crates/helm-ctl
git commit -m "feat: add helmctl theme command parser"
```

### Task 2: Implement `theme lint` with exact palette selection and diagnostics

**Files:**
- Modify: `crates/helm-ctl/src/main.rs`
- Modify: `crates/helm-ctl/tests/theme_cli.rs`

**Interfaces:**
- Consumes: `helm_theme::load_palette`, `helm_theme::lint`, `helm_core::Palette::load`.
- Produces: `run_lint(args: &LintArgs, root: &Path) -> Result<ExitCode, String>`; stdout has every hue pair as `<a>/<b>: <degrees>°`; stderr has every fatal finding.

- [ ] **Step 1: Write failing subprocess tests**

```rust
#[test]
fn lint_shipped_palette_is_session_independent_and_prints_hue_separations() {
    let out = helmctl(["theme", "lint", "--config-root", temp.path()]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("violet/"));
}

#[test]
fn lint_bad_explicit_palette_exits_one_and_names_finding() {
    let bad = write_bad_palette(temp.path());
    let out = helmctl(["theme", "lint", "--palette", bad.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("text.normal"));
}
```

- [ ] **Step 2: Run the lint tests red**

Run: `cargo test -p helm-ctl lint_`

Expected: FAIL because lint dispatch and output do not exist.

- [ ] **Step 3: Implement root resolution and lint rendering**

```rust
fn default_config_root(env: &impl Env) -> Result<PathBuf, String> {
    env.var_os("XDG_CONFIG_HOME").map(PathBuf::from)
        .or_else(|| env.var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .ok_or_else(|| "no XDG_CONFIG_HOME or HOME for helm configuration".into())
}
```

Use `Palette::load(--palette)` without seeding when `--palette` is present; otherwise use `load_palette(root)`. Print all separations, then all findings. Return 1 when `LintReport::is_clean()` is false.

- [ ] **Step 4: Re-run lint tests and no-session evidence**

Run: `cargo test -p helm-ctl lint_ -- --nocapture`

Expected: PASS with no socket fixture or session process.

- [ ] **Step 5: Commit lint delivery**

```bash
git add crates/helm-ctl
git commit -m "feat: add helmctl theme lint"
```

### Task 3: Implement read-only `theme diff`

**Files:**
- Modify: `crates/helm-ctl/src/main.rs`
- Modify: `crates/helm-ctl/tests/theme_cli.rs`

**Interfaces:**
- Consumes: `helm_theme::apply`, `helm_theme::diff`, `helm_theme::ThemeOutputChange`.
- Produces: sorted output lines `added <path>`, `removed <path>`, or `byte-different <path>`; exit 0 for no changes and 1 for a non-empty diff/refusal.

- [ ] **Step 1: Write failing read-only integration test**

```rust
#[test]
fn diff_after_palette_edit_is_sorted_and_does_not_mutate_generation_tree() {
    let root = temp.path();
    assert!(helmctl_at(root, ["theme", "apply"]).status.success());
    edit_palette(root, "accent.violet", "#b07aff");
    let before = tree(root);
    let out = helmctl_at(root, ["theme", "diff"]);
    assert_eq!(out.status.code(), Some(1));
    assert_sorted_change_lines(&stdout(&out));
    assert_eq!(tree(root), before);
}
```

- [ ] **Step 2: Run the diff test red**

Run: `cargo test -p helm-ctl diff_after_palette_edit_is_sorted_and_does_not_mutate_generation_tree`

Expected: FAIL because diff dispatch/output does not exist.

- [ ] **Step 3: Implement strict diff mapping**

```rust
match helm_theme::diff(&root) {
    Ok(changes) if changes.is_empty() => ExitCode::SUCCESS,
    Ok(changes) => { for change in changes { print_change(change); } ExitCode::from(1) }
    Err(error) => fail(&error.to_string(), 1),
}
```

Do not create a generation on diff error and do not add a `--palette` path that bypasses the captured generation snapshot contract.

- [ ] **Step 4: Re-run focused and library diff evidence**

Run: `cargo test -p helm-ctl diff_ && cargo test -p helm-theme generation_diff_`

Expected: PASS; the CLI regression and existing no-mutation library tests both pass.

- [ ] **Step 5: Commit diff delivery**

```bash
git add crates/helm-ctl
git commit -m "feat: add helmctl theme diff"
```

### Task 4: Implement generation-only `theme apply` outcome mapping

**Files:**
- Modify: `crates/helm-ctl/src/main.rs`
- Modify: `crates/helm-ctl/tests/theme_cli.rs`
- Modify: `crates/helm-theme/src/theme.rs`

**Interfaces:**
- Consumes: `helm_theme::apply` and `GenerationPublicationOutcome::{Committed,CommittedWithCleanupPending,OutcomeAmbiguous}`.
- Produces: a safe CLI result: `Committed`/`CommittedWithCleanupPending` exit 0 and name selected generation; `OutcomeAmbiguous` exit 6, prints no success stdout, and no retry/recovery is attempted.

- [ ] **Step 1: Write failing public-path regression and CLI tests**

```rust
#[test]
fn public_apply_fatal_candidate_preserves_current_generation() {
    let root = tempfile::tempdir().unwrap();
    helm_theme::apply(root.path()).unwrap();
    let before = current_target(root.path());
    write_fatally_unreadable_palette(root.path());
    assert!(helm_theme::apply(root.path()).is_err());
    assert_eq!(current_target(root.path()), before);
}

#[test]
fn apply_reports_selected_future_generation_without_reload_or_session() {
    let out = helmctl_at(temp.path(), ["theme", "apply"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("selected for future launches"));
    assert!(!stdout(&out).contains("reloaded"));
}
```

- [ ] **Step 2: Run apply tests red**

Run: `cargo test -p helm-ctl apply_ && cargo test -p helm-theme public_apply_fatal_candidate_preserves_current_generation`

Expected: CLI test FAIL because apply dispatch is absent; add the library regression only if it first exposes a real gap.

- [ ] **Step 3: Implement exact outcome reporting**

```rust
match helm_theme::apply(&root) {
    Ok(Committed(id)) => success(format!("generation {id} selected for future launches")),
    Ok(CommittedWithCleanupPending { generation, cause }) => { /* success stdout + warning stderr */ }
    Ok(OutcomeAmbiguous { candidate, cause }) => ambiguous(candidate, cause),
    Err(error) => fail(&error.to_string(), 1),
}
```

Never send IPC, signals, commands, notifications, recovery, or retry. Escape control characters in `cause` before the diagnostic.

- [ ] **Step 4: Run apply contract tests**

Run: `cargo test -p helm-ctl apply_ && cargo test -p helm-theme public_apply_fatal_candidate_preserves_current_generation`

Expected: PASS; apply has no session dependency and leaves `current` unchanged on fatal candidate.

- [ ] **Step 5: Commit apply delivery**

```bash
git add crates/helm-ctl crates/helm-theme/src/theme.rs
git commit -m "feat: add helmctl theme apply"
```

### Task 5: Package, document, and verify the delivered surface

**Files:**
- Modify: `packaging/nix/package.nix`
- Modify: `packaging/debian/helm.install`
- Modify: `packaging/fedora/helm.spec`
- Modify: `docs/specs/0002-theme-pipeline.md`
- Modify: `docs/specs/0006-helm-ctl.md`

**Interfaces:**
- Consumes: cargo binary `target/release/helmctl`.
- Produces: all packaging lanes install `helmctl`; accepted-spec acceptance tables name real test evidence.

- [ ] **Step 1: Write failing packaging/projection and CLI install checks**

```bash
rg -n 'helmctl' packaging/nix/package.nix packaging/debian/helm.install packaging/fedora/helm.spec
cargo build -p helm-ctl --release --locked
test -x target/release/helmctl
```

Expected: packaging assertions fail before their installation records are added.

- [ ] **Step 2: Add exact binary installation records and test names**

Use each packaging format’s existing binary-install convention. Do not add a `helm` binary, package publication channel, network job, daemon, or alias. Replace only B1–B3/B15–B17 test-table blanks with the actual `helm-ctl` integration test names.

- [ ] **Step 3: Run complete local verification**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features --locked && cargo build -p helm-ctl --release --locked && bash packaging/fedora/check-projections.sh`

Expected: every command exits 0; no command requires a session socket.

- [ ] **Step 4: Commit final delivery**

```bash
git add Cargo.toml crates/helm-ctl crates/helm-theme/src/theme.rs packaging docs/specs
git commit -m "feat: ship helmctl theme commands"
```

## Self-review

- B1/B2 are Task 2, B3/G10 Task 3, G11/G12/B15–B17 Task 4, and distribution/spec evidence Task 5.
- The plan excludes #22’s retired reload fan-out and does not implement draft #132/#133/#135 contracts.
- The plan contains no unresolved placeholders or generic error-handling instructions before execution.
