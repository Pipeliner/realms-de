# Packaged `helm-sdd` Git Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Nix-installed `helm-sdd` resolve Git from its package
runtime path and prove that its session wrapper does not receive Git.

**Architecture:** The unified `helm` output obtains a Git store reference only
through a `helm-sdd` wrapper. A focused Nix check creates a clean disposable
Git record-carrier, invokes that installed wrapper with an unusable inherited
`PATH`, and inspects the separate `helm-session` wrapper for Git's absence.
The check is a flake attribute under the existing CI invocation, not a workflow,
daemon, hook, or service.

**Tech Stack:** Nix `buildRustPackage`, `makeWrapper`, Nix `runCommand`, Git,
existing Rust `helm-sdd` fixture contract.

**Spec:** [SPEC 0010](../../specs/0010-packaged-helm-sdd-git-runtime.md),
[ADR 0016](../../adr/0016-packaged-helm-sdd-carries-git.md)

## Global Constraints

- Wrap `helm-sdd` with `pkgs.git` only; do not add Git to
  `support.wrapperRuntime` or `helm-session`'s wrapper path.
- Preserve #144's separate `nativeBuildInputs = [ pkgs.makeWrapper pkgs.git ]`
  check-time dependency.
- The runtime proof must use a real local disposable Git repository, never
  host `PATH`, network access, record mutation outside that disposable fixture,
  or automatic memory infrastructure.
- The installed gate must exit zero with the canonical SPEC 0008 result.
- Do not add a GitHub Actions workflow or publish infrastructure.

---

### Task 1: Add the package-level runtime regression

**Files:**
- Modify: `packaging/nix/checks.nix`

**Interfaces:**
- Consumes: `helm`, `pkgs.git`, `pkgs.coreutils`, and the source tree passed to
  the existing checks function.
- Produces: `checks.<system>.helm-sdd-git-runtime`, a derivation that exits
  successfully only when the installed wrapper supplies Git and the session
  wrapper omits Git's store bin path.

- [x] **Step 1: Add the regression before the wrapper change**

  Add a `runCommand` check which creates a base commit, writes a minimal valid
  `.agent/work/120` checkpoint/evidence carrier in a second commit, then runs:

  ```nix
  ${pkgs.coreutils}/bin/env -i PATH=/nonexistent \
    ${helm}/bin/helm-sdd gate --issue 120 --from probe --to spike \
    > "$TMPDIR/actual.json"
  printf '%s\n' '{"issue":120,"from":"probe","to":"spike","outcome":"pass","obligations":[{"code":"accepted_refs","status":"met"},{"code":"clean_workspace","status":"met"},{"code":"decision_provenance","status":"met"},{"code":"fresh_checkpoint","status":"met"},{"code":"fresh_evidence","status":"met"},{"code":"git_objects","status":"met"},{"code":"hygiene","status":"met"},{"code":"issue_directory","status":"met"},{"code":"schema","status":"met"},{"code":"transition","status":"met"},{"code":"transition_evidence","status":"met"},{"code":"transition_fields","status":"met"}]}' > "$TMPDIR/expected.json"
  cmp --silent "$TMPDIR/expected.json" "$TMPDIR/actual.json"
  ! grep -F '${pkgs.git}/bin' ${helm}/bin/helm-session
  ```

  The complete expected JSON is the canonical A1 output from SPEC 0008; use
  `${pkgs.git}/bin/git` for every fixture operation so fixture construction does
  not accidentally prove host `PATH` availability.

- [x] **Step 2: Establish the red condition without pushing a red branch**

  Run the check with the current unwrapped package using a Nix version capable
  of evaluating the locked nixpkgs. It must fail at the installed
  `helm-sdd` invocation because `PATH=/nonexistent` contains no Git. If this
  host cannot evaluate the lock, retain the test locally and do not push until
  Task 2 supplies the wrapper; remote CI is the required green proof.

- [x] **Step 3: Verify CI executes the check on every runner path**

  Run:

  ```bash
  python3 -c 'from pathlib import Path; text = Path(".github/workflows/distro.yml").read_text(); start = text.index("nix build --print-build-logs"); block = text[start:text.index("fi", start)]; assert ".#checks.$system.helm-sdd-git-runtime" in block'
  nix-instantiate --parse packaging/nix/checks.nix >/dev/null
  ```

  Expected: `distro.yml` builds the check in its no-KVM fallback; Nix source
  parses. This scope correction followed adversarial review, which found that
  merely evaluating the check would not execute the runtime regression.

- [x] **Step 4: Commit only after Task 2 produces a passing runtime check**

  The red test and production wrapper belong to one non-red-push increment.

### Task 2: Wrap only `helm-sdd` and correct stale package comments

**Files:**
- Modify: `packaging/nix/package.nix`
- Modify: `flake.nix`

**Interfaces:**
- Consumes: the `pkgs.git` input already used at check time.
- Produces: `$out/bin/helm-sdd` whose wrapper prefixes the Git store bin path;
  `$out/bin/helm-session` keeps its existing `support.wrapperRuntime` path.

- [x] **Step 1: Add the minimal runtime wrapper**

  Immediately after the existing session wrapper, add:

  ```nix
  wrapProgram $out/bin/helm-sdd \
    --prefix PATH : ${lib.makeBinPath [ pkgs.git ]}
  ```

  Do not alter `support.wrapperRuntime`.

- [x] **Step 2: Correct stale pre-alpha comments**

  In `package.nix` and `flake.nix`, replace the claim that the workspace only
  has `helm-core` and installs no Helm binaries. State precisely that desktop
  binaries remain pending while the local `helm-sdd` validator is installed.

- [x] **Step 3: Run green verification**

  Run:

  ```bash
  git diff --check
  cargo test --offline -p helm-agent-sdd --test gate -- --test-threads=1
  nix-instantiate --parse flake.nix >/dev/null
  nix flake check --print-build-logs
  ```

  The final command is authoritative and may run remotely when the local Nix
  version cannot evaluate the lock. Require `helm-sdd-git-runtime` and the
  existing package/session checks to pass.

- [x] **Step 4: Commit**

  ```bash
  git add packaging/nix/checks.nix packaging/nix/package.nix flake.nix \
    docs/superpowers/plans/2026-08-29-packaged-helm-sdd-git-runtime.md
  git commit -m "fix: wrap packaged helm-sdd with Git (#146)"
  ```

### Task 3: Review, land, and record proof

**Files:**
- Modify: `docs/specs/0010-packaged-helm-sdd-git-runtime.md`
- Update: GitHub issue #146

**Interfaces:**
- Consumes: remote CI evidence for the installed package regression.
- Produces: implemented acceptance references and a closed, accurately labeled
  GitHub issue.

- [x] **Step 1: Request adversarial review**

  Review the exact package closure claim, the empty-PATH fixture, the session
  wrapper assertion, and execution of the check on both CI runner paths.

- [x] **Step 2: Record the real guard**

  Replace the planned verification text for A1–A3 with the exact Nix check
  attribute after it passes. Do not claim completion from a derivation diff.

- [ ] **Step 3: Merge only after green remote CI**

  Open a PR, require both remote `nix flake check` runs, merge into `main`,
  then remove `status:in-progress` from the automatically closed issue.
