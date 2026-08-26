# NixOS Session-Discovery CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the NixOS VM test validate Helm's native display-manager session-discovery contract and verify the result on remote CI.

**Architecture:** `programs.helm` continues to publish its rewritten session package through `services.displayManager.sessionPackages`. The VM becomes a consumer of that NixOS interface by enabling the lightweight Ly display manager and asserting the desktop file in `config.services.displayManager.sessionData.desktops`, rather than asserting an unpromised system-profile path. The existing GitHub workflow remains the execution authority: KVM runs the full VM and no-KVM runners build the independent checks.

**Tech Stack:** Nix flakes, NixOS module test (`pkgs.testers.nixosTest`), GitHub Actions, ShellCheck.

**Spec:** `docs/superpowers/specs/2026-08-26-nixos-session-discovery-ci-design.md`; `docs/specs/0005-session-startup.md`

## Global Constraints

- Helm supplies a NixOS session through `services.displayManager.sessionPackages`; it must not select or enable a display manager for users.
- `/run/current-system/sw/share/wayland-sessions/helm.desktop` is not a Helm contract.
- The test fixture may enable Ly only as a concrete consumer of the interface.
- Retain the existing GitHub Actions KVM full-check path and non-KVM evaluation plus ShellCheck/package-build path.
- Do not add CI providers, GitHub Projects, daemons, cron jobs, or orchestration infrastructure.
- Local Nix 2.16.1 cannot evaluate this flake; remote CI is the release evidence for the VM.

---

### Task 1: Make the VM assert NixOS session data

**Files:**
- Modify: `packaging/nix/checks.nix:28-55`
- Test: `packaging/nix/checks.nix:28-55` (the NixOS VM test is the regression test)

**Interfaces:**
- Consumes: `config.services.displayManager.sessionData.desktops`, the NixOS-generated directory containing `/share/wayland-sessions`.
- Produces: a `checks.<system>.session-boots` VM whose failure names the session-data contract, not a system-profile path.

- [ ] **Step 1: Preserve the existing red evidence and write the corrected contract test**

  The baseline red evidence already exists in GitHub Actions run `33010932793`:
  the VM booted and failed only at the unpromised
  `/run/current-system/sw/share/wayland-sessions/helm.desktop` assertion.
  Keep that run URL in the issue record. Do not manufacture a second failure
  by changing package code: this task corrects the test's contract.

  Change `nodes.machine` to accept `config`, enable the supported lightweight
  consumer, and bind the evaluated session-data output once:

  ```nix
  nodes.machine =
    { config, ... }:
    {
      imports = [ nixosModule ];
      programs.helm.enable = true;
      services.displayManager.ly.enable = true;
      virtualisation.memorySize = 2048;
    };

  testScript =
    let
      desktops = nodes.machine.config.services.displayManager.sessionData.desktops;
    in
    ''
      # existing boot assertion
      machine.succeed("test -f ${desktops}/share/wayland-sessions/helm.desktop")
    '';
  ```

  Replace the current first desktop-file assertion, which uses
  `/run/current-system/sw/share/wayland-sessions/helm.desktop`, with the
  interpolated `desktops` path. Keep the assertion immediately after
  `machine.wait_for_unit("multi-user.target")`.

- [ ] **Step 2: Run the corrected test remotely for green evidence**

  Push only the Task 1 change and inspect the `nix flake check` job created by
  GitHub Actions. If `/dev/kvm` is present, the required green evidence is the
  session-data assertion rather than the former `/run/current-system/sw`
  assertion. If KVM is absent, do not claim VM coverage; confirm the log says
  it ran `nix flake check --no-build` and both independent targets:

  ```sh
  nix build ".#checks.$system.shellcheck" ".#checks.$system.package"
  ```

- [ ] **Step 3: Complete the native-contract assertion**

  In the same `testScript`, retain identity validation against the native path
  and add an executable-target check:

  ```nix
  machine.succeed(
    "grep -q '^DesktopNames=helm$' ${desktops}/share/wayland-sessions/helm.desktop"
  )
  machine.succeed(
    "grep -q '^Exec=/nix/store/.*/helm-session-launch$' "
    + "${desktops}/share/wayland-sessions/helm.desktop"
  )
  ```

  The `Exec` check intentionally accepts Nix's content-addressed store prefix
  but requires the module-generated `helm-session-launch`, proving the
  desktop file was rewritten by `sessionPackage` rather than copied directly
  from the package output.

- [ ] **Step 4: Verify the VM test remotely**

  Push the completed task and inspect the exact GitHub Actions run:

  ```sh
  gh pr checks 115 --repo Pipeliner/realms-de --watch
  gh run view <run-id> --repo Pipeliner/realms-de --log-failed
  ```

  Required green evidence on a KVM runner is a successful `nix flake check`
  whose log includes `checks.x86_64-linux.session-boots`. On a non-KVM runner,
  record the no-KVM limitation and require successful independent ShellCheck
  and package targets; do not mark #112 resolved until a KVM-capable remote run
  has produced the full-check evidence.

- [ ] **Step 5: Commit**

  ```bash
  git add packaging/nix/checks.nix
  git commit -m "test: assert NixOS session discovery contract"
  ```

### Task 2: Correct remote-CI explanation and close the loop

**Files:**
- Modify: `.github/workflows/distro.yml:152-173`
- Test: GitHub Actions `nix flake check` job

**Interfaces:**
- Consumes: `/dev/kvm` availability and flake check targets.
- Produces: accurate workflow logs that distinguish full VM evidence from the
  non-KVM fallback without claiming standard GitHub runners never expose KVM.

- [ ] **Step 1: Write the expected workflow messaging before changing it**

  Preserve the shell branches and their commands. Replace only stale comments
  that say GitHub standard runners do not have KVM with this policy:

  ```yaml
  # KVM availability is detected, not assumed. A KVM-capable runner executes
  # the NixOS VM; a runner without KVM still evaluates the flake and builds the
  # independent ShellCheck and package checks.
  ```

  Preserve `set -euo pipefail`, the `/dev/kvm` conditional, `nix flake check
  --print-build-logs`, `nix flake check --print-build-logs --no-build`, and the
  explicit ShellCheck/package `nix build` command.

- [ ] **Step 2: Verify workflow syntax locally**

  Run the repository's shell syntax checks that do not require Nix:

  ```bash
  bash -n packaging/session/helm-session
  git diff --check
  ```

  If a compatible ShellCheck binary becomes available, also run:

  ```bash
  shellcheck --shell=bash packaging/session/helm-session
  ```

  The host currently lacks an installable ShellCheck and cannot evaluate the
  flake, so these local commands do not replace remote CI.

- [ ] **Step 3: Verify the workflow remotely**

  Push Task 2 and inspect the new run. Require a successful `nix flake check`
  job. If KVM is present, require the VM result from Task 1; if absent, require
  both fallback build targets in the job log and leave #112 open pending a KVM
  run.

- [ ] **Step 4: Commit**

  ```bash
  git add .github/workflows/distro.yml
  git commit -m "ci: document KVM-detected Nix validation"
  ```

### Task 3: Symphony verification and PR disposition

**Files:**
- Modify: none unless CI evidence identifies a real defect.
- Test: GitHub Actions runs for PR #115, Dependabot #10, and Dependabot #11.

**Interfaces:**
- Consumes: verified remote checks, Issue #112, and PR labels.
- Produces: evidence-backed PR status updates; no red PR is merged.

- [ ] **Step 1: Record #112 evidence**

  Add a GitHub issue comment containing the run URL, whether KVM was detected,
  and the exact successful check names. Do not close #112 from a no-KVM fallback
  run.

- [ ] **Step 2: Reassess PRs only from fresh checks**

  - PR #115: remove `status:blocked` only after its fresh remote Nix check is
    green; then perform normal review before merge.
  - PR #10: retain its narrow Dependabot diff; rerun/review it only after the
    shared #112 gate is green.
  - PR #11: request/recreate the Dependabot branch rebased on current `main`,
    then require fresh green checks; do not attribute old helm-theme failures
    to toml 1.1.4.

- [ ] **Step 3: Commit**

  No repository commit is created for label/comment-only Symphony actions.
