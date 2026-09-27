# Remaining MVP assessment — 2026-09-27

Snapshot of main `e0b5d61`, the 75 open GitHub issues and 11 open PRs inspected
today. This is an assessment, not a release certification. Accepted authority:
[MVP](MVP.md), [SPEC 0030](specs/0030-reuse-first-session-theme-mvp.md),
[SPEC 0031](specs/0031-mvp-launch-delivery.md), and
[SPEC 0032](specs/0032-upstream-idle-lock-integration.md).

## Verdict and execution order

Not launch-ready. There are 48 open issues without post-MVP disposition, but
many are overlapping verification/reconciliation records, not missing features.
27 issues are explicitly deferred. Do not estimate percent complete by issue
count or restart implemented components because their issues remain open.

1. Packaging lane: get #236's corrected native fixture diagnostics from CI;
   resolve the actual timeout, then verify installed native graphical sessions.
   #243 supplies XWayland diagnostics, not a presumed fix.
2. Session lane: implement the approved once-per-login theme selection and
   reconcile #240/#241. This is real remaining implementation, not just CI.
3. Desktop lane: finish #247/#79 real locker, idle and suspend verification;
   verify actual chooser/screenshare flows. Do not enable dormant idle units
   solely because package checks pass.
4. Run the common #78 journey, repair user-visible failures, reconcile completed
   issues, update install claims and start the real week as soon as usable.

Three independent lanes may progress together; shared startup/theme plumbing
has one writer. No routine dependency upgrade, security-hardening review,
framework expansion or optional bar replacement should delay these steps.

## Complete remaining issue map

Every open issue not marked post-MVP appears below. Existing implementation is
not equivalent to fresh runtime evidence. Evidence references describe what
was inspected, not a claim that all current checks pass.

| Issues | Assessment | Remaining MVP action |
|---|---|---|
| #134, #98, #73 | Package source contracts/code exist; #236 still unmerged | Resolve native fixture timeout with preserved logs; verify private tools and target-specific River dependency payloads. #73's every-target vendoring title is stale: Fedora uses its native candidate. |
| #74, #75, #76 | Nix flake/module and native recipes exist; historical installed Nix VM and native installroot evidence exist | Full current candidate builds and graphical login on NixOS, Ubuntu 24.04 and Fedora 44. #76's Fedora 41 title and SELinux gate are stale; hardening is deferred. |
| #117, #135 | Accepted simpler theme contract; existing assets and per-launch implementation differ | Select once at login, apply only for next login, preserve user configs, prove real consumers and relogin. Reconcile PRs #240/#241 instead of extending old lease machinery. |
| #25, #26, #27, #28, #29, #30, #31 | GTK/Qt/foot/Yazi/btop/Starship/fuzzel templates exist in configs/templates | Treat as consumer acceptance under #117/#135; verify appearance/configuration, shell and files/monitor launches. Old live-retint and needs-human wording must not reintroduce superseded semantics. |
| #79 | Upstream choice and defaults accepted; helpers/package staging on main; binding suppression in #247 | Real PAM unlock/readiness, duplicate starts, suppressed/restored bindings, 300s dim, 600s lock, restoration and host-policy suspend. New VM round-trip at #247 head 2560988 is unexecuted; idle stays disabled. Native/hardware proof remains. |
| #69, #102 | Portal routing config exists; Nix test contains portal exercise | Verify real application FileChooser and browser ScreenCast, Settings and Inhibit=none on supported installed sessions. Service presence is insufficient. |
| #107 | DISPLAY integration and mapping diagnostics exist; #243 unmerged | Distinguish X11 mapping from Realm projection using current runtime results, fix demonstrated failures, verify activated X11 clients. |
| #58, #60, #68, #70, #108 | Accepted startup spec, entry, environment handshake and systemd units exist | Verify fresh login, environment propagation, ordering, clean logout/relogin and failed-WM return to login across targets. Reconcile River attachment diagnostics; no replacement wrapper project. |
| #38, #40, #61, #62, #63, #64, #65, #66, #42 | Session/backend implementation and source tests exist; historical Nix desktop evidence | Current tests plus live window policy/layer-shell/chords/input repeat/tap, quantising-terminal geometry, liveness, supervised restart/replay and bar-crash isolation. Repair concrete failures; do not rebuild the backend. |
| #43 | Current bar works in historical VM; comparison remains bounded | One Waybar comparison and at most one correction pass; retain current bar if replacement is not cheaper. Optional replacement cannot become launch critical path. |
| #45, #46, #47, #48, #49, #52, #53 | Bar/orbit/layout/mode/title/key-discovery rendering and real screenshots exist | Reuse current installed-session journey to verify live changes and usability; close only with matching acceptance evidence. |
| #50, #51 | Sampler/runtime and module assertions exist | Verify required net/cpu/mem/clock/battery and everyday controls on appropriate VM/hardware. Audit stale GPU/volume title scope against accepted bar spec; do not invent extra instruments. |
| #55, #71 | Font/scale/cursor/XWayland consistency remains an acceptance review item | Verify ordinary single-output desktop usability and configured cursor/theme paths; distinguish actual launch defects from deferred polish. No claim of completed hardware/scale coverage. |
| #72 | Doctor exists with bounded diagnostics and JSON compatibility | Verify actionable backend/executable/theme/portal results and remedies on installed candidate; no exhaustive historical failure-register expansion. |
| #77 | docs/INSTALL.md exists and states limitations | Update against actual candidate evidence and once-per-login contract; remove stale mandatory lease-lifecycle wording. Verify commands through CI-installed artifacts, not local packaging. |
| #78 | Acceptance journey defined; no completed daily-use result recorded | Record all three distro journeys and actual week using Realm as only desktop, with failures and verdict. Cannot be replaced by CI or fabricated elapsed time. |

README real screenshots are already delivered, with original provenance and
[visual review](assets/capture-review.md). They prove an older Nix VM desktop,
not complete themes or this candidate. Refresh from the verified candidate if
appearance changes; do not recreate this deliverable from scratch.

## PR disposition

- #236: updated to e568e14 with diagnostic retention; full CI pending.
- #247: updated to 2560988 with upstream package integration and real VM lock
  round-trip; full runtime evidence pending. It is not full #79 completion.
- #240/#241: reconcile with approved session-scoped theme contract before merge.
- #243: diagnostics; native fixture timed out after 15 minutes without internal
  logs on the older branch. Update diagnostics and inspect actual results.
- #248/#249/#250/#252/#253/#254: routine dependency PRs remain deferred; still
  inspected. Failures are not silently waived or merged.

Main CI was pending behind an older active verification at inspection. The
native fixture timeout cause is unknown; do not assume it only needs more time.

## Explicitly deferred issue inventory

#16, #17, #22, #32, #44, #54, #56, #67, #82, #83, #84, #85, #86, #87,
#89, #90, #91, #92, #93, #94, #95, #96, #97, #106, #111, #116, #199.

Keep these out of launch gating unless a concrete required user journey exposes
a dependency. Security-hardening remains post-MVP; ordinary functional lock
correctness is still required.

## Time and forecast confidence

Prospective task timing starts in [task-time.csv](task-time.csv). Historical
effort is unmeasured, not zero. Separate work, review, verification and waits.
There is not enough measured throughput or current runtime evidence for a
defensible delivery date. The actual daily-use week remains a calendar floor
after a usable candidate exists; development and fixes may extend it.
