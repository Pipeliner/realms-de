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
Recent completed main runs must not hide older nonterminal runs. In addition
to the recent snapshot, query main runs by each supported nonterminal status
(`queued`, `in_progress`, `pending`, `waiting`, `requested`) with pagination,
deduplicating run IDs. Bound API requests and pagination by the remaining
monitor budget; an incomplete/error response cannot prove quiescence. A watch
returns zero only after a successful snapshot with no relevant nonterminal
ci/distro/palette runs. If its budget expires while runs remain nonterminal,
return 124; query failures remain nonzero. This is read-only observation, never
cancellation or workflow mutation.

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
5. Verify manual locking, 5-minute dim and 10-minute lock/blank.
6. Logout/relogin; verify clean restart and next-login theme selection.

Suspend/resume integration and verification are explicitly deferred until after
MVP by the owner (2026-09-27). They do not gate this journey or idle enablement;
lock-before-host-policy-suspend remains the intended post-MVP behavior, not a
verified launch guarantee. Retain the failed VM evidence for that follow-up.

Run the same journey on all supported distros; retain hardware-only limitations
explicitly. Begin #78's
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
Native installed-session doctor evidence must resolve Realm's private Yazi and
Starship paths independently of the caller's PATH; the intentionally unimported
private PATH is not evidence that the packaged executables are absent.

Reconcile old issues once against actual source and named tests/CI evidence.
Close only demonstrated acceptance, splitting partial issues into remaining
user-visible gaps. Stale titles, labels or a claimed implementation are not
proof. Preserve unresolved failures. Session recovery and theme migration tests
remain required wherever they affect the journey.

## Verification

**Accepted bounded Rust-test mitigation (2026-09-27).** Every direct workspace
Rust-test step in the launch workflows (`ci.yml` and `distro.yml`, including
both Ubuntu and Fedora smoke matrix entries) has a 20-minute execution limit
and its job a 25-minute limit, leaving
time for diagnostic upload and cleanup. Neighboring completed test jobs took
about two minutes; main run 36331319678 remained in its test step for over
100 minutes without downloadable logs. This bound prevents recurrence of an
unbounded test gate; it does not identify or fix a test deadlock or runner fault.
Keep the existing workspace/all-features selection and preserve failure through
the logging pipeline. Retain only its combined text output for seven days using
an always-run upload, including after step failure or timeout where the runner
remains available. Missing logs after runner loss are missing evidence, never
success. Main cancellation policy remains unchanged.

The initial correction covered `ci.yml` alone. Distro run 36337828094 completed
its build in 29 seconds but its Ubuntu test step, started at 17:41:26 UTC,
remained active beyond 50 minutes; another theme-candidate distro job completed
in 2m24s. Apply the same bound to the shared cause across both launch workflows,
retaining distro's `--locked` selection and matrix-unique seven-day text-log
artifacts. A workflow fixture discovers all direct workspace test steps in
these workflows so a new launch lane cannot silently omit the contract. This
does not diagnose a deadlock without logs. The optional `agentic-loop.yml`
workflow is outside the MVP launch lanes and remains deferred.

Workflow configuration tests guard event registration, draft/full boundaries,
main cancellation policy and routine dependency freeze. Remote CI supplies the
actual execution evidence; local configuration tests cannot certify a package.
The acceptance journey and daily-use record supply launch evidence, not this spec.
