# ADR 0018 — M1 desktop launch is fresh-process Exec only

- **Status:** Accepted (2026-08-29 product ruling)
- **Deciders:** helm maintainers, repo owner
- **Issue:** [#133](https://github.com/Pipeliner/realms-de/issues/133)
- **Supersedes / Superseded by:** Narrows M1 launcher claims; a later accepted
  D-Bus activation ADR may add a separate path without weakening this one.

## Context

M1 needs to launch an application against one immutable generation selected
under SPEC 0011. Desktop Entry Specification 1.5 defines two materially
different paths. A plain `Exec` entry can start a new process which inherits a
private environment. A `DBusActivatable=true` entry instead asks an application
owner on the session bus to activate; its process may already exist and cannot
be made to inherit one launch's private environment.

Fuzzel's application mode executes `Exec` directly and does not provide Helm's
generation selection, lifecycle lease, or D-Bus semantics. Publishing theme
variables to the systemd user manager or D-Bus activation environment would be
per-UID global mutation, not per-launch ownership. The D-Bus activation
environment also has no public readback with which Helm could prove prior-value
restoration or compare-and-swap ownership.

Accepted ADR 0011 already requires `helm-session` entry to publish its exact
display/session variables to both the systemd user-manager and D-Bus activation
environments; Draft SPEC 0005 specifies that path and #60 schedules it. This
decision does not make that import optional. #132 owns only the prior per-UID
claim/order prerequisite. #133 owns restoration/compare-and-restore semantics
for values the session wrote and refusal of launch modes whose ownership cannot
be proven, not the baseline import list, timing, portal or `doctor` guards.

The 2026-08-29 owner correction is decisive: “existing owner” is meaningful in
this decision only for D-Bus activation. A plain `Exec` entry has no standard
owner identity. Process names, `StartupWMClass`, Wayland app ids and window
titles cannot supply one.

## Decision

1. M1 admits only the main `[Desktop Entry]` of a supported
   `Type=Application`, `DBusActivatable=false` or absent, tokenized `Exec`
   entry. Each admitted request starts one fresh process under the immutable
   generation and lifecycle contracts.
2. `DBusActivatable=true` is an unconditional preflight refusal before any
   generation or lifecycle side effect. Helm does not run its `Exec` as a
   fallback, query or activate its bus name, call
   `org.freedesktop.Application`, call `StartServiceByName`, or inspect whether
   a bus owner exists.
3. Plain `Exec` has no owner probe. Helm makes no promise to find, retheme,
   signal, restart, replace, attach to, or deduplicate an already-running
   application. Concurrent independent `Exec` is part of the fresh-process
   boundary.
4. Theme bindings are an environment overlay and immutable argv/cwd plan for
   the one child. Helm does not publish them to the systemd user manager or
   D-Bus activation environment, mutate the launcher's process-global
   environment, or set a global `XDG_CONFIG_HOME`.
5. Fuzzel may be a picker UI only when it returns a Helm-issued entry identity
   and Helm performs resolution, admission and launch. Direct fuzzel
   application mode and raw `Spawn(argv)` are not verified, themed or
   generation-selected Helm launches.
6. Successful D-Bus activation, activation of an existing owner, a
   session-global Qt policy, an activation proxy, bus restart, and activation
   teardown are deferred to a later dedicated decision. #135 owns evidence
   that fresh Qt5/Qt6 and other named consumers actually consume their exact
   generation bindings; this ADR makes no Qt support claim by itself.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Standards-aware M1 launcher supporting both `Exec` and D-Bus activation | Matches the full desktop-entry model and could activate singleton applications | No end-to-end proof connects an existing or bus-spawned process to one private immutable generation; accepting it would turn an unproved activation result into a theming claim |
| Session-global Qt and activation environment | Existing and D-Bus-spawned applications could observe one session policy | It is per-UID shared state with no public readback/CAS restoration protocol; it also cannot make already-running applications consume a new generation |
| Helm D-Bus proxy/name owner | Could serialize activation and mediate ownership | It introduces a persistent service, bus-name recovery, proxy protocol and teardown contract that M1 does not yet own or test |
| Probe plain `Exec` owners with process/window metadata | Appears to prevent duplicate applications cheaply | Desktop Entry 1.5 defines no owner for plain `Exec`; basename, PID, window class/app-id and title matching can target unrelated processes and are inherently racy |

## Consequences

### Good

- Every successful M1 claim is observable as a new owned process with one
  child-only immutable generation.
- D-Bus refusal is independent of bus availability and has a complete
  zero-launch-side-effect witness.
- Plain `Exec` does not invent unsafe singleton or retheming semantics.
- A later D-Bus slice has an honest boundary instead of compatibility behavior
  that users could mistake for support.

### Bad

- Applications whose selected entry declares `DBusActivatable=true` cannot be
  launched through the verified M1 path, even when they carry a tempting
  `Exec` fallback.
- Direct fuzzel application mode cannot count as the M1 launcher.
- Existing plain-Exec applications are not deduplicated or rethemed.

### Neutral

- The session entry may already have made the display/portal environment
  changes required by ADR 0011 before an application request. Refusing a
  launch neither creates nor rolls back that prior session state.
- The decision says how a fresh process is admitted, not which application
  profiles and static assets M1 supports; #135 remains authoritative for those
  details.

## Reversal

**Medium and additive if kept separate; structural if retrofitted into this
path.** A later slice may add a distinct typed D-Bus request, activation owner,
global-policy or proxy contract, persistent evidence and hostile bus fixtures.
It must not silently reinterpret `DBusActivatable=true` as Exec fallback or
weaken the fresh-process proof. Reconsider when an accepted design can prove
generation identity for new and existing owners across bus restart, teardown
and concurrent sessions.

## Guard

- *Planned (#133, M1):* E4–E6 prove unconditional D-Bus refusal, no owner query
  or activation side effect, and fresh-only plain Exec semantics.
- *Planned (#133, M1):* E12 proves direct fuzzel application mode and raw argv
  admission cannot report a verified themed launch.

## Needs a human

None. The owner selected and then corrected this M1 boundary on 2026-08-29.
