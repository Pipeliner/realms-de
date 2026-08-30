# Fresh Desktop Exec Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement accepted SPEC 0013 as independently reviewable, red-first M2 slices without exposing lifecycle authority or claiming a deployable launch path before #135.

**Architecture:** Add a private `generation::desktop_exec` module for immutable desktop admission, beginning with a pure byte-oriented `Exec` parser that has no filesystem, generation, lifecycle, environment, or process side effects. A later private lifecycle facade alone may consume an admitted plan. It must first solve the existing safe low-level process-execution boundary; no `GenerationSelection`, lifecycle record, lease, transfer, or gate capability becomes public or `pub(crate)` merely to make launch code convenient.

**Tech Stack:** Rust 2021 / MSRV 1.85, existing `helm-theme` test conventions, `rustix` descriptor-relative Linux APIs, `tempfile` fixtures.

**Spec:** `docs/specs/0013-truthful-fresh-desktop-exec.md`, `docs/specs/0012-activation-launch-lifecycle.md`, `docs/adr/0018-fresh-desktop-exec-only.md`.

## Global Constraints

- `DBusActivatable=true` refuses before every lifecycle, generation, environment, D-Bus, owner-probe, or execution operation.
- The request input is a desktop identity only; raw argv, desktop action, payload, terminal wrapper, caller environment override, and existing-owner probing are out of scope.
- The only initially accepted `Exec` output is one executable argv element: standalone `%f`, `%F`, `%u`, `%U` may disappear without creating an argument; `%%` is a literal nonempty second argument and therefore refuses under this one-argv cut. Every static argument, embedded/quoted code, deprecated/unknown code, malformed escape, shell feature, or control byte refuses.
- Admission is pure/read-only until it produces an immutable plan. It must not open or initialize a `GenerationStore`.
- Lifecycle code remains a private child module. Do not export or reconstruct `GenerationSelection`, lease, record, transfer, ownership-evidence, or gate-token authority.
- #133 supplies only synthetic allowlist/overlay fixtures. Production target assets and real allowlist entries remain #135.
- No execution implementation may bypass `helm-theme`'s `#![forbid(unsafe_code)]`; a child `fexecve` runner requires a separately reviewed safe abstraction or ADR-backed low-level crate.

---

### Task 1: Add a private, zero-side-effect `Exec` parser

**Files:**

- Modify: `crates/helm-theme/src/generation.rs:1-5`
- Create: `crates/helm-theme/src/generation/desktop_exec.rs`
- Test: `crates/helm-theme/src/generation/desktop_exec.rs` `#[cfg(test)]`

**Interfaces:**

- Produces private `fn parse_exec(value: &[u8]) -> Result<Vec<Vec<u8>>, DesktopExecError>`.
- Produces private `enum DesktopExecError { Empty, StaticArgument, UnsupportedFieldCode, MalformedEscape, UnsafeByte }` with stable test matching only inside the module.
- The only successful result is a one-element `Vec` holding the executable token bytes. It never resolves a pathname or executes anything.
- `generation.rs` declares `mod desktop_exec;`; it does not re-export the module or parser.

- [ ] **Step 1: Write the failing table-driven parser tests**

Create `desktop_exec.rs` with only its test module. Add a table containing one accepted form and refusal classes:

```rust
#[test]
fn exec_parser_accepts_only_one_payload_free_executable_argv() {
    assert_eq!(parse_exec(b"profile"), Ok(vec![b"profile".to_vec()]));
    assert_eq!(parse_exec(b"profile %%"), Err(DesktopExecError::StaticArgument));
    assert_eq!(parse_exec(b"profile %f"), Ok(vec![b"profile".to_vec()]));
    assert_eq!(parse_exec(b"profile %F"), Ok(vec![b"profile".to_vec()]));
    assert_eq!(parse_exec(b"profile %u"), Ok(vec![b"profile".to_vec()]));
    assert_eq!(parse_exec(b"profile %U"), Ok(vec![b"profile".to_vec()]));
}

#[test]
fn exec_parser_refuses_shell_and_field_code_ambiguity() {
    for value in [
        b"profile --flag".as_slice(), b"profile;id", b"profile$HOME",
        b"profile %c", b"profile %i", b"profile %k", b"profile %x",
        b"profile %Fextra", b"profile \\"quoted\\"", b"profile\\\\",
        b"profile\nnext", b"profile\0next",
    ] {
        assert!(parse_exec(value).is_err(), "accepted {value:?}");
    }
}
```

Also cover leading/trailing ASCII whitespace, a quoted executable, multiple tokens, an escaped whitespace, a standalone `%F`, and every `Terminal`/payload handling case only insofar as the parser refuses non-one-token input. The parser tests must not import `GenerationStore`, `lifecycle`, `std::process`, or filesystem APIs.

- [ ] **Step 2: Verify red**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests -- --nocapture
```

Expected: FAIL because `generation::desktop_exec` and `parse_exec` do not exist.

- [ ] **Step 3: Implement the smallest byte parser**

Perform exactly the specified Desktop Entry general-unescape, whole-argument double-quote tokenization, and one field-code expansion pass. Accept one nonempty safe executable token plus, at most, one unquoted standalone `%f`, `%F`, `%u`, or `%U` that disappears; reject two payload codes, any `%%` result as a nonempty second final argument, and every other extra token. Return a dedicated refusal, never a shell-transformed approximation.

```rust
fn parse_exec(value: &[u8]) -> Result<Vec<Vec<u8>>, DesktopExecError> {
    let tokens = tokenize_and_expand_once(value)?;
    if tokens.len() != 1 || tokens[0].is_empty() || tokens[0].contains(&b'=') {
        return Err(DesktopExecError::StaticArgument);
    }
    Ok(tokens)
}
```

Do not use `Command`, shell splitting, `sh -c`, path lookup, environment reads, or a generic desktop-entry crate.

- [ ] **Step 4: Verify green and non-interference**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests -- --nocapture
cargo test -p helm-theme --lib generation::lifecycle::tests -- --nocapture
cargo clippy -p helm-theme --all-targets --all-features -- -D warnings
```

Expected: all PASS; parser tests have no lifecycle setup and existing registry tests remain unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs crates/helm-theme/src/generation/desktop_exec.rs
git commit -m "feat: add fail-closed desktop Exec parser"
```

### Task 2: Specify and build immutable XDG capture as a separate PR

**Files:**

- Modify: `crates/helm-theme/src/generation/desktop_exec.rs`
- Test: `crates/helm-theme/src/generation/desktop_exec.rs` `#[cfg(test)]`

**Interfaces:**

- Produces private `DesktopFileId` and `AdmittedDesktopPlan` holding captured bytes plus descriptor identities; it imports no lifecycle type.
- Produces private `admit_desktop(id: DesktopFileId, inputs: &AdmissionInputs) -> Result<AdmittedDesktopPlan, AdmissionError>`.
- An injected test-only audit records downstream attempts and remains empty for every refusal; it is not a production capability.

- [ ] **Step 1: Write red descriptor-relative capture tests**

Create `mod xdg` and use `tempfile` roots plus real symlinks/modes. Each refusal asserts that the admission audit is empty and no `GenerationStore` was opened. Require these exact tests:

The mandatory test names are:

- `xdg_empty_or_unset_uses_defaults_and_requires_home_for_data_home`
- `xdg_first_matching_root_wins_and_hidden_masks_lower_root`
- `desktop_id_collision_in_one_root_refuses`
- `capture_refuses_symlink_unsafe_mode_duplicate_group_or_duplicate_main_key`
- `capture_revalidates_identity_and_never_rereads_replaced_path`
- `capture_enforces_root_entry_depth_file_line_and_spelling_bounds`

The first fixture deletes `HOME` only when `XDG_DATA_HOME` is unset/empty and asserts refusal before filesystem/generation mutation. The precedence fixture writes a valid high-priority entry and a lower-priority decoy, then replaces the former with `Hidden=true` and asserts the lower candidate stays masked. The identity fixture swaps the pathname after byte capture through a test-only capture checkpoint and asserts the held bytes/identity either stay identical or admission refuses; it never rereads the new pathname.

- [ ] **Step 2: Verify red**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests::xdg -- --nocapture
```

Expected: FAIL because admission does not exist.

- [ ] **Step 3: Implement descriptor-relative capture**

Reuse the generation module's no-follow validation style. Capture roots once; use only `openat`/`statat`/held fds after capture; bound roots, entries, depth, total spelling, file size, lines, and line length exactly as SPEC0013 §1 states. The candidate read must revalidate device/inode/size/mtime/ctime before and after. It must not open `GenerationStore`.

- [ ] **Step 4: Verify green**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests -- --nocapture
cargo test -p helm-theme --lib generation::lifecycle::tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation/desktop_exec.rs
git commit -m "feat: capture desktop entries immutably"
```

### Task 3: Gate D-Bus and N-independent desktop preflight before the facade

**Files:**

- Modify: `crates/helm-theme/src/generation/desktop_exec.rs`
- Test: `crates/helm-theme/src/generation/desktop_exec.rs` `#[cfg(test)]`

**Interfaces:**

- Produces private preflight-only validation returning an immutable plan or refusal; no lifecycle facade is called here.
- Produces held static executable and `TryExec` descriptor/header facts, but never observes generation N or decides a generation-bound allowlist match.

- [ ] **Step 1: Write red structural, D-Bus, and static-preflight tests**

Create `tests::dbus_refusal_after_structural_validation` and prove that a valid main group with `DBusActivatable=true` returns one refusal before parsing or using `Exec`, `TryExec`, `Path`, base environment, bus name, owner, generation, lifecycle, or exec observation. Its test-only audit must be empty. Pair it with a malformed `Type`, empty `Name`, `Hidden=true`, `NoDisplay=true`, and `Terminal=true` matrix proving structural main-group validation occurs first.

Create `tests::static_preflight_refuses_without_generation_effect` for malformed `TryExec`, invalid/duplicate/unallowed/oversized environment, every `LD_*`, unsafe PATH/executable/cwd, shebang/short/foreign ELF, and descriptor replacement. It must assert refusal before any `GenerationStore` operation. It may capture `TryExec` and static descriptor/header facts, but must not call an N-bound allowlist or acquire `/dev/null`: the latter belongs to the owner/facade.

- [ ] **Step 2: Verify red**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests::dbus_refusal_after_structural_validation -- --nocapture
cargo test -p helm-theme --lib desktop_exec::tests::static_preflight_refuses_without_generation_effect -- --nocapture
```

Expected: FAIL because the effects audit and preflight stages do not exist.

- [ ] **Step 3: Implement fail-closed preflight**

Capture and structurally validate the main group first; then reject `DBusActivatable=true` before parsing or using `Exec`, `TryExec`, `Path`, base environment, or any downstream interface. For an allowed plain entry, build the bounded sorted base environment, hold static descriptor identities, and validate ELF headers. Capture/resolve `TryExec` under that same static policy but defer the generation-bound allowlist comparison for both executable identities to Task 4. Keep all descriptors and binding types private.

- [ ] **Step 4: Verify green**

Run:

```bash
cargo test -p helm-theme --lib desktop_exec::tests -- --nocapture
cargo test --workspace --all-features --locked --quiet
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation/desktop_exec.rs
git commit -m "feat: preflight desktop Exec admission"
```

### Task 4: Design and implement the private lifecycle-consuming facade separately

**Files:**

- Modify: `crates/helm-theme/src/generation.rs`
- Modify: `crates/helm-theme/src/generation/lifecycle.rs`
- Create or modify only after an ADR-approved safe low-level child-runner boundary exists
- Test: `crates/helm-theme/src/generation/lifecycle.rs` and an external compile-fail fixture

**Interfaces:**

- Produces private `FreshExecFacade` consuming `AdmittedDesktopPlan`; it alone resolves/validates N before publishing the process lease, then uses `ActivationRegistry`.
- Produces private `GenerationStore::select_current_for_fresh_exec<T>(owner_pid: u32, validate: impl FnOnce(&CurrentGenerationInspection) -> Result<T, String>) -> Result<(GenerationSelection, T), String>`. Under the existing shared generation lock it validates `current`, exposes only the held generation/manifest/allowlist inspection to the private facade callback, creates no lease on callback failure, and only then creates/fsyncs the exact process lease returned in `GenerationSelection`.
- Produces no public launch DTO/API and no public lifecycle capability.
- The owner remains supervisor; exactly one profile child changes cwd/stdin/env, close-sweeps internal descriptors, and calls safe wrapped `fexecve`. The owner receives a CLOEXEC error report, reaps the exact failed child, and writes the existing terminal witness.

- [ ] **Step 1: Write the required safe-execution ADR and red ordering/topology tests**

The checked pinned `rustix 1.1.4` exposes only `unsafe runtime::execveat`, so it cannot satisfy `helm-theme`'s `#![forbid(unsafe_code)]` directly. Before façade code, add and accept an ADR naming the exact separately audited low-level crate/API that confines `fork`, `fchdir`, `dup2`, close sweep, and `execveat(AT_EMPTY_PATH)` to one safe child-runner interface. The ADR must define its error-report and fd ownership contract.

Write a red `fresh_exec_nonmatching_allowlist_creates_no_lease_or_record` test for `select_current_for_fresh_exec`: a nonmatching synthetic `ExecutableAllowlist<N>` callback returns refusal and the leases directory plus `launches/` inventory remain unchanged. Add red subprocess fixtures for two concurrent plain requests, a pre-existing same-name sentinel, one child exec per owner, no owner probe, exact error-report/reap, fd isolation, and no lifecycle authority visible in `/proc/self/fd` after exec.

- [ ] **Step 2: Verify red**

Run: `cargo test -p helm-theme fresh_exec -- --nocapture`

Expected: FAIL because no production façade or safe child runner exists.

- [ ] **Step 3: Implement only the approved façade and runner**

Keep selection, allowlist, process lease, record, transfer, gate, and release internal. Implement `select_current_for_fresh_exec` exactly as above; use its callback to compare both held executable identities to `ExecutableAllowlist<N>` before any lease/record. Only the accepted safe child-runner interface may acquire verified `/dev/null`, change cwd, assemble envp, close sweep, and execute by held descriptor. The parent owner never execs; it tracks/reaps the exact child and follows SPEC0012 recovery ordering.

- [ ] **Step 4: Verify green**

Run the dedicated subprocess suite, full workspace test/clippy/fmt/doc suite, and remote CI including Nix. Require E2/E5/E6/E8/E9/E10/E12 evidence before merging.

- [ ] **Step 5: Commit**

```bash
git add crates/helm-theme/src/generation.rs crates/helm-theme/src/generation/lifecycle.rs
git commit -m "feat: consume admitted desktop Exec through lifecycle facade"
```

## Plan self-review

- SPEC coverage: Tasks 1–3 cover E1/E3/E4 and N-independent parts of E7. Task 4 covers generation-bound allowlisting, `/dev/null`, overlay and actual execution evidence in E2/E5/E6/E7/E8/E9/E10/E11/E12 only after its safe-execution ADR is accepted.
- Scope: the parser is intentionally its own mergeable task; this plan does not pretend #133 can ship a real arbitrary desktop launch before #135 provides consumer assets.
- Placeholder scan: no task delegates a safety decision to unspecified error handling; every deferred execution step is explicitly blocked on an ADR-backed safe wrapper, not silently omitted.
