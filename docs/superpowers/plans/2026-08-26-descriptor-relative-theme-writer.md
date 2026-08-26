# Descriptor-relative Theme Writer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ensure #110's theme-output writer cannot be redirected outside its configuration root after validation.

**Architecture:** `theme.rs` replaces post-validation `std::fs` pathname operations with a descriptor-owned writer. It retains a root `OwnedFd`, opens destination parents with rustix `openat`, and retains each parent FD with the temporary and final basename until `renameat` or `unlinkat`.

**Tech Stack:** Rust 2021, rustix 1.1 `fs`, tempfile.

**Spec:** `docs/specs/0002-theme-pipeline.md`; `docs/superpowers/specs/2026-08-26-descriptor-relative-theme-writer-design.md`.

## Global Constraints

- Keep Rust 1.85 compatibility and `#[forbid(unsafe_code)]`.
- Never use a reconstructed target pathname for output reads, writes, commits, or cleanup after descriptor acquisition.
- Preserve render-before-write, reload-after-commit, and the 150 ms warm-cache budget.

---

### Task 1: Lock the race contract in tests

**Files:**
- Modify: `crates/helm-theme/src/theme.rs`
- Modify: `docs/specs/0002-theme-pipeline.md`

**Interfaces:**
- Consumes: `apply_with(&Palette, &Path, &[Template], &mut dyn Reloader)`.
- Produces: private descriptor helpers testable inside `theme::tests`.

- [ ] **Step 1: Write failing unique-staging test**

Plant `target.helm-tmp` as a symlink to a victim, call `apply_with`, and assert
the victim remains unchanged while the target receives rendered bytes. Rename
the current test to `a_symlinked_staging_file_is_not_touched`.

- [ ] **Step 2: Write failing parent-replacement test**

Open `root/live` with the new private parent helper, rename it to `root/held`,
replace `root/live` with a symlink to `victim`, then stage and commit
`theme.conf` through the held FD. Assert `victim/theme.conf` is absent.

- [ ] **Step 3: Observe the red tests**

Run `cargo test -p helm-theme symlinked_staging_file` and
`cargo test -p helm-theme replaced_output_parent`. Both fail because the
existing writer uses a fixed temporary pathname and `std::fs` paths.

### Task 2: Implement descriptor-owned staging and commit

**Files:**
- Modify: `crates/helm-theme/src/theme.rs`

**Interfaces:**
- Consumes: `rustix::fs::{openat, mkdirat, renameat, unlinkat, fsync, AtFlags, Mode, OFlags, CWD}` and `std::os::fd::OwnedFd`.
- Produces: private `OutputRoot` and `StagedOutput` records.

- [ ] **Step 1: Add root and parent traversal**

Define `OutputRoot { fd: OwnedFd, display: PathBuf }`. Construct it with
`openat(CWD, root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW |
OFlags::CLOEXEC, Mode::empty())`. For each normal parent component, call
`openat(&parent, component, OFlags::RDONLY | OFlags::DIRECTORY |
OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())`; on `NotFound`, call
`mkdirat(&parent, component, Mode::RWXU)` and retry the same `openat`.

- [ ] **Step 2: Replace comparison and staging**

Read a final file only with `openat(&parent_fd, final_name, OFlags::RDONLY |
OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())`. Build temporary basenames
as `.<final>.helm-tmp.<pid>.<sequence>` from a process-local `AtomicU64`; retry
`CREATE | EXCL | NOFOLLOW | CLOEXEC` on `AlreadyExists`. Write bytes and call
`fsync` on the opened file.

- [ ] **Step 3: Replace commit and cleanup**

Store the parent FD and both basenames in `StagedOutput`. Commit with
`renameat(&staged.parent_fd, &staged.temporary, &staged.parent_fd,
&staged.final_name)` followed by `fsync(&staged.parent_fd)`. Cleanup uses
`unlinkat(&staged.parent_fd, &staged.temporary, AtFlags::empty())`, ignoring
only `NotFound`. Keep `root.join(template.target)` solely for
`Applied::written` reporting.

- [ ] **Step 4: Verify the focused tests**

Run the two commands from Task 1. Expected: PASS and no victim mutation.

### Task 3: Verify and publish the increment

**Files:**
- Modify: `crates/helm-theme/src/theme.rs`
- Modify: `docs/specs/0002-theme-pipeline.md`

**Interfaces:**
- Consumes: SPEC 0002 A1–A14 tests.
- Produces: a verified #110 increment, while #22 remains the explicit multi-file-publication boundary.

- [ ] **Step 1: Run crate verification**

Run `cargo fmt --check` and `cargo test -p helm-theme`. Expected: clean
formatting and every theme test passes.

- [ ] **Step 2: Run workspace verification**

Run `cargo test --workspace` and `cargo clippy --all-targets -- -D warnings`.
Expected: all tests pass with no warnings.

- [ ] **Step 3: Commit and push**

Stage `crates/helm-theme/src/theme.rs` and `docs/specs/0002-theme-pipeline.md`,
commit with message `Use descriptor-relative theme writes`, push, and comment
on #110 with the exact tests and the remaining #22 boundary.

## Self-review

- A12 maps to Task 1's unique staging test; A14 maps to its parent-replacement test; Tasks 2–3 implement and verify both.
- The plan names concrete flags, functions, types, files, commands, and expected results.
- `OutputRoot` owns the root FD; `StagedOutput` owns its parent FD and names, so later filesystem operations cannot fall back to an absolute path.
