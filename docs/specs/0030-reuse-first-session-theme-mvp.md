# SPEC 0030 — Reuse-first MVP: session themes and bar comparison

- **Status:** Accepted (owner approval, 2026-09-27); implementation pending
- **Milestones:** M1–M3
- **Decision:** [ADR 0023](../adr/0023-reuse-first-session-theme-mvp.md)

## Authority and preserved behavior

This specification controls conflicting MVP theme selection/launch-ownership
clauses in SPECs 0002/0003/0005/0006/0011/0012/0013/0020 and INTERFACES.md.
It controls renderer-choice clauses in SPEC 0004 and ADR 0008. Other clauses
remain applicable: palette format/lint, window semantics, control transport,
user-file preservation, coherent assets, portal routing, packaging and lock/idle.
The historical generation specifications are shelved as product obligations,
not evidence that their current implementation may be bypassed unsafely.

## T1 — Theme selection and publication

`realmctl theme apply` validates and renders a complete set of supported outputs
from the selected palette and prepares it for the **next graphical login**.
It reports that scope explicitly. Validation/render/publication failure leaves
the previously prepared complete selection available; never select a partial
output set. Existing reusable staging/validation code may remain an internal
implementation detail. Success is not a claim that any live process changed.

At login, resolve one complete prepared selection before starting themed Realm
clients and use it for that session. If no selection exists, prepare the shipped
default with the same validation. A corrupt selection is diagnosed, not silently
replaced by guessed data. Later apply operations must not mutate the selected
session assets or environment. MVP permits one active Realm session per user
(owner confirmed 2026-09-27). Reject a competing same-UID Realm login without
disturbing the existing session. Sessions belonging to different users keep
independent theme selections. Persist only the lifetime information needed.

The session must not reclaim configuration still in use. For MVP, leaving old
published theme data intact is acceptable; automatic generation GC and bounded
retention are not launch gates. Do not delete legacy leases or protected trees
as part of switching launch semantics. A later cleanup change needs its own
ownership proof. This exception concerns theme data, not the separate required
bounded Cargo/build-cache cleanup system.

### Login-selection implementation boundary

Reuse the generation store's complete validation and process-identity leases.
A lookup by the already-selected generation ID SHALL validate and lease that
generation without consulting or changing the next-login `current` pointer.
Missing or corrupt selected data fails rather than falling back to `current`.
A login helper may explicitly retain its existing lease for the graphical
entry process after successful handoff. This consumes the helper's selection
and closes its descriptors without removing the lease. Normal drop still
releases an unhanded-off lease. Existing live-process identity checks protect
the retained generation until the graphical entry exits; existing stale-lease
reconciliation remains usable. No new lifecycle manager is required.

The graphical entry publishes `realm/session-theme.json` under its existing
per-user runtime directory before starting the compositor. The record contains
the absolute configuration root, immutable generation ID, graphical-entry PID,
Linux process start time and boot ID. Publication is atomic; a live existing
owner cannot be replaced. A new login may replace a well-formed dead-owner
record after taking the exclusive login claim. Malformed records fail with a
diagnostic. Consumers validate the owner identity and selected generation; a
missing or stale record is an error, never a request to select `current`.
The helper retains the owner's process lease only after successful publication.
WM restarts read this record instead of bootstrapping a new selection.
The record is private runtime state, not global toolkit activation environment.
The entry invokes the private `realm-wm --prepare-session-theme PID` command
with its own PID; failure aborts before compositor startup. The existing
explicit no-WM diagnostic mode remains available when the WM is absent.
The graphical entry takes a nonblocking exclusive `flock` on
`$XDG_RUNTIME_DIR/realm-session.lock` before compositor/PID/environment changes.
The entry owns the descriptor directly, preserving its signal/teardown identity;
compositor and direct-client subprocesses close their inherited copy before
exec. Contention exits 73 without running session cleanup. The lock file is not
unlinked on exit; entry exit releases the claim after teardown. Native packages
declare the flock executable dependency and the Nix wrapper supplies it.

## T2 — Consumer activation

Configure foot, fuzzel, Yazi, btop, zsh/Starship, GTK and Qt using their supported
configuration/environment/argument mechanisms, selected at login. Retain useful
existing templates and adapters. Preserve existing user files and avoid changing
another desktop's defaults. A session-local config overlay may be used where a
tool cannot import a theme. Do not claim GTK/Qt coverage without real packaged
consumer evidence, including supported runtime/plugin versions.

Applications launched from the session inherit its selected configuration;
standard desktop launchers and D-Bus activation are allowed. Already-running
single-instance applications or services outside that session may retain their
old appearance. Document this boundary and do not globally overwrite theme
environment to retheme unrelated sessions. No exact process ancestry, fresh-Exec
only admission, per-launch seal verification, generation lease transfer or
descendant-drain witness is part of MVP acceptance. Existing browser default
selection in SPEC 0027 is unchanged.

Session shutdown uses ordinary service/scope lifecycle management. Required
environment discovery, portal startup ordering and client isolation remain;
UWSM is an optional implementation evaluation, not a new prerequisite.

## B1 — Waybar comparison, not automatic replacement

Compare the existing bar with packaged Waybar on the same CI-built River session.
Use upstream modules for commodity metrics and a narrow event-fed Realm adapter
for orbit/layout/mode/title/chord state. Do not assume River tag modules reflect
Realm's ledger. Avoid adding polling for state already available as events.
Sampled system metrics and a clock may use bounded periodic updates.

Preserve six orbit states, layout/mode, focused title, clock, CPU/memory/network/
battery, which-key and the grimoire. Preserve readable glyphs/fallbacks, scaling,
reserved workarea, fullscreen behavior and focus. A temporary hybrid experiment
is allowed; do not establish permanent duplicate bars or supervisors.

Record cold-start-to-usable, state-to-visible update latency, idle CPU and memory
with environment, versions and sample method. Compare against the existing
ARCHITECTURE.md targets; report misses rather than masking them. Renderer-specific
damage rectangles, pure-Rust code and an exact number of timers are not acceptance
criteria for the alternative. Select it only with functional evidence and a
recorded comparison; otherwise retain the existing bar. New custom metric/render
work waits for that result. Repairing a real current-bar usability defect does not.

## Verification obligations before implementation is declared complete

### Overlap scope resolved

The owner confirmed one session on 2026-09-27 in response to the explicit
one-Realm-session-per-user recommendation. This preserves SPEC 0005's single
active same-UID claim. Same-user concurrent Realm sessions are not an MVP
requirement. The login handoff must survive window-manager restart without
reselecting the latest prepared theme; a second login must not overwrite the
active selection or its activation environment. Different users remain isolated.

| ID | Required evidence |
| --- | --- |
| V1 | Login with A; apply B; both an existing app and a newly launched app in that session still use A; next login uses B. |
| V2 | Failed render/publication does not disturb active A or replace the last complete next-login selection. |
| V3 | A competing same-user login is rejected without affecting the active selection; different-user sessions and a single-instance app demonstrate the documented scope; another desktop's files/environment are not overwritten. |
| V4 | Actual packaged foot, Yazi, btop, shell, GTK/Qt and launcher consume the session configuration; no placeholders counted as coverage. |
| V5 | Logout/restart leaves no Realm-owned orphan client; legacy theme data is not deleted while possibly in use. |
| V6 | Waybar comparison covers B1 behavior, measurements and inspected real screenshots; failures remain visible. |

Write failing tests for changed behavior before implementation. Packages,
installed-consumer tests and VM verification run in CI only. This spec update
does not make current code conformant or mark any of V1–V6 passed.

## Queue disposition

- Keep lock/idle (#79), portals (#69/#102), login/session (#68/#70), XWayland
  (#107), distro installation and week-long use (#78) on the MVP path.
- Rescope #117/#135 and theme asset issues #25–#31 to T1/T2. They no longer wait
  for SPEC 0012's transfer/gate machinery. PRs #240/#241 require reconciliation;
  retain useful assets and consumer probes, not obsolete lifecycle obligations.
- Keep #43/#45–#53 as functional bar outcomes, evaluated through B1. Shelve #54's
  renderer-specific damage/benchmark harness; B1 still measures performance.
- Shelve generation-aware live upgrade (#22). Retire automatic upstream-tool
  replacement (#88); shelve custom launcher/harness/dialog/compositor work
  (#82–#87, #89–#92) without treating it as an eventual mandatory replacement.
- Do not close historical implemented generation issues as regressions, reopen
  them, or delete their code merely because their contract is no longer required.
