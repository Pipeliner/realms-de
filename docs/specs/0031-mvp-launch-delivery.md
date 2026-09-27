# SPEC 0031 — Bounded MVP launch delivery

- **Status:** Accepted (owner approved recommendations 1–7, 2026-09-27)
- **Scope:** M0–M3 delivery; NixOS, Ubuntu and Fedora remain required

## CI and dependency freeze

Main pushes, PRs and manual dispatch remain verification entry points. Feature
branch pushes do not separately run the same workflows as their PRs. Draft PRs
run source checks; ready-for-review candidates also run packaging, native build
fixtures and VM checks. Marking ready must trigger that full verification even
without a new commit. Manual dispatch and main always retain the full matrix.
Never interpret skipped draft packaging as release evidence or merge a candidate
without its required full checks. Workflow names and existing tests remain.

New main pushes must not cancel a running main verification. Superseded PR runs
may be cancelled. GitHub may replace an older pending run with a newer pending
run; this is not a promise to verify every intermediate main commit. Batch docs
and queue bookkeeping into one push rather than repeatedly invalidating evidence.
No package builds, installation or archives run locally.

The read-only `scripts/ci-monitor` queries main-branch workflow runs at the API
boundary, before any result limit, so recent PR traffic cannot hide main.
It reports the required ci/distro/palette workflows. Query or response errors
fail `--once`; `--watch` retries only within its configured wall-clock budget,
including failed queries and sleeps, then fails if no successful final query
was possible. It never treats an API failure as an empty successful snapshot.
Interval and maximum-duration settings must be positive decimal integers;
invalid settings fail before querying, rather than disabling timeouts or polling
without delay.

Pause routine Dependabot version PR creation until MVP acceptance. Continue
inspecting the complete incoming PR queue; defer routine upgrades with a visible
post-MVP disposition, and admit only a documented launch/CI-blocking exception.
Do not disable security alerts as a side effect. Restore the previous version-PR
limits (five per ecosystem) after MVP, with verification.

## Three delivery lanes

Use the existing GitHub issues, not another orchestration framework. Each lane
has at most one active implementation owner; independent work can proceed in
parallel. Review and merge independently verified changes promptly. Shared
launch plumbing has one writer; another lane reports findings instead of
concurrently rewriting it.

| Lane | Immediate outcome | Existing work |
| --- | --- | --- |
| Session/themes | Login-bound coherent theme and reliable lifecycle | #117/#135/#68, reconcile #240/#241 |
| Desktop | Lock/idle, portals, usable window controls and key discovery | #79/#69/#102/#43, review #247 |
| Packaging/verification | Installable candidate and evidence for all three distros | #74/#75/#76/#98/#107, review #236/#243 |

Bound the Waybar experiment to one implementation-and-measurement pass and one
focused correction pass. If it does not demonstrate a cheaper route to the
required functioning bar, keep the existing bar for launch and record why.
UWSM is optional and cannot block launch. No further open-ended framework study.

## One acceptance journey

Track the journey on #78. Record commit, exact CI package artifact/run, distro,
VM or hardware, result and defect links. Reuse this evidence for issue closure,
install documentation and README screenshots; do not build parallel proof sets.

1. Fresh install and graphical login; run doctor and inspect the bar.
2. Open terminal, browser and files; verify theme, shell and ordinary launch.
3. Exercise focus/swap/orbits/layout, which-key and full key help.
4. Open a real file chooser and perform a real browser screen share.
5. Verify 5-minute dim, 10-minute lock/blank, and lock before host-policy suspend.
6. Logout/relogin; verify clean restart and next-login theme selection.

Run the same journey on all supported distros; retain hardware-only limitations
explicitly rather than claiming VM suspend proves laptop behavior. Begin #78's
real week of daily use as soon as the journey is usable; cosmetic work continues
in parallel. A week cannot be fabricated or replaced by a green CI run.

### Native window controls and key discovery (accepted refinement, 2026-09-27)

Step 3's installed Ubuntu/Fedora VM probe opens three real terminals through
the default terminal binding, then uses their interactive shells to give them
distinct titles and visible labels. All focus, swap, orbit, layout and help
actions use real QEMU keyboard input. Protocol-v2 Hello/GetState/ShowLedger are
read-only observations, never action injection. The probe requires exact
window identities/order and focused titles for forward/back focus, neighbour
swap and restoration, empty-orbit switch and return, and mono/triptych changes.
Which-key must hide/show/dismiss and full key help must open/dismiss through
their shipped bindings. Preserve the initial which-key setting and close the
three fixture windows through the normal close binding afterward.

Every transition has a finite readiness deadline; absent state or unchanged
incorrect focus/order fails rather than passing on command dispatch. Retain
structured observations and complete framebuffer screenshots of the meaningful
states, plus bounded failure diagnostics.
The Nix companion probe must retain these observations with explicit collection
types compatible with the test driver's enabled static type checking; its
heterogeneous result object must not obscure the appendable observation list.
This proves the installed keyboard path and compositor-backed state; screenshots remain CI visual evidence, not
an assertion that a local unit fixture rendered the UI. No new control command,
window-management feature or browser-automation dependency is introduced.

## Doctor cut line and reconciliation

For MVP, #72 covers actionable session/backend connectivity, required executable
availability, selected theme validity and portal availability, with truthful
bounded results and remedies. File chooser/screenshare success is proved by
the journey, not inferred from service presence. Existing cheap diagnostics and
JSON compatibility remain; adding every historical failure-register row is not
a launch gate. This supersedes that completeness requirement in SPEC 0006.

Reconcile old issues once against actual source and named tests/CI evidence.
Close only demonstrated acceptance, splitting partial issues into remaining
user-visible gaps. Stale titles, labels or a claimed implementation are not
proof. Preserve unresolved failures. Session recovery and theme migration tests
remain required wherever they affect the journey.

## Verification

**Accepted bounded Rust-test mitigation (2026-09-27).** The workspace Rust
test step has a 20-minute execution limit and its job a 25-minute limit, leaving
time for diagnostic upload and cleanup. Neighboring completed test jobs took
about two minutes; main run 36331319678 remained in its test step for over
100 minutes without downloadable logs. This bound prevents recurrence of an
unbounded test gate; it does not identify or fix a test deadlock or runner fault.
Keep the existing workspace/all-features selection and preserve failure through
the logging pipeline. Retain only its combined text output for seven days using
an always-run upload, including after step failure or timeout where the runner
remains available. Missing logs after runner loss are missing evidence, never
success. Main cancellation policy remains unchanged.

Workflow configuration tests guard event registration, draft/full boundaries,
main cancellation policy and routine dependency freeze. Remote CI supplies the
actual execution evidence; local configuration tests cannot certify a package.
The acceptance journey and daily-use record supply launch evidence, not this spec.
