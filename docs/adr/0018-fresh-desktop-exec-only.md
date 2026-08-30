# ADR 0018 — Desktop launch is fresh-process Exec only

- **Status:** Accepted (2026-08-30)
- **Deciders:** helm maintainers, repository owner
- **Issue:** [#133](https://github.com/Pipeliner/realms-de/issues/133)
- **Depends on:** [ADR 0017](0017-immutable-theme-activation-generations.md),
  accepted [SPEC 0012](../specs/0012-activation-launch-lifecycle.md)
- **Supersedes / Superseded by:** Narrows desktop-launch claims.  A later,
  separately accepted D-Bus-activation decision may add a distinct path without
  weakening this one.

## Context

A selected Helm generation is private, immutable launch input.  A normal
desktop `Exec` entry can start a new child with that input.  A
`DBusActivatable=true` entry instead asks a session-bus owner to activate; that
owner may predate the request and cannot be shown to have inherited one
request's private generation binding.  The D-Bus activation environment is
per-UID global state and has no public readback/compare-and-swap protocol from
which Helm could prove prior-value restoration.

The accepted M2 lifecycle contract already owns the only authoritative owner,
lease transfer, execution gate, durable state, and reconciliation protocol.
It deliberately exposes none of those powers as a public desktop-launch API.
The closed #163 draft predated that authority and is evidence only: its public
wire, request-history, pidfd, preparation-registry, and alternate-state-machine
designs must not be revived.

Plain `Exec` has no standard existing-owner identity.  Process names, PID
files, `StartupWMClass`, Wayland app IDs, window titles, and inferred bus names
are neither safe nor race-free substitutes.

## Decision

1. Helm admits only the main group of a supported `Type=Application` desktop
   file whose `DBusActivatable` value is absent or exactly `false`, whose
   `Terminal` value is absent or exactly `false`, and whose `Exec` value parses
   into a supported tokenized argv.  Each admitted request starts at most one
   new child through the M2 lifecycle facade.
2. `DBusActivatable=true` is an unconditional, read-only preflight refusal.
   Helm does not inspect `Exec` as a fallback, derive/query a bus name, call an
   application D-Bus method, create an owner, select a generation, create a
   lease/record/scope/group, mutate environment, or execute target code.
3. Plain `Exec` has no owner probe.  Helm does not deduplicate, find, retheme,
   signal, restart, replace, attach to, or make singleton claims about an
   already-running application.  Concurrent admitted requests may start
   independent fresh processes.
4. Admission captures desktop bytes, argv, executable/cwd bindings, and the
   base environment immutably.  After the M2 facade selects one generation, it
   derives the child-only profile overlay and explicit child environment.  It
   never publishes that overlay to the launcher process, systemd user manager,
   D-Bus activation environment, or global `XDG_CONFIG_HOME`.  Exact target
   overlays and evidence remain [#135](https://github.com/Pipeliner/realms-de/issues/135).
5. A picker may provide an entry identity only.  Helm resolves, admits, and
   launches it itself.  Direct fuzzel application mode and raw `Spawn(argv)`
   remain unverified fire-and-forget requests, not themed/generation-selected
   launches.
6. The M2 facade is the only interface from this decision into lifecycle
   machinery.  It consumes an immutable admitted plan; it internally creates
   the owner, selects the generation, records/prepares/adopts it, authorizes
   the one gate, and permits one `Exec`.  It exposes no lifecycle lease,
   transfer, record-replacement, ownership-evidence, or gate-token authority.

## Alternatives considered

| Option | Why it lost |
|---|---|
| Treat `DBusActivatable=true` as an `Exec` fallback | It contradicts the desktop-entry model and would falsely imply ownership of a D-Bus result. |
| Query an existing owner before plain `Exec` | No standard owner exists; every available heuristic is racy and can target an unrelated process. |
| Publish a session-global theme environment | Per-UID mutation cannot prove restore/CAS ownership and cannot retheme an existing process. |
| Expose M2 lifecycle internals to a launcher | Caller-constructed transfer/release/state authority would duplicate or bypass the accepted lifecycle proof. |

## Consequences

- D-Bus activation, existing-owner activation, session-global Qt policy,
  activation proxies, bus restart, and activation teardown are deferred to a
  dedicated decision.
- A D-Bus refusal is invariant whether a bus or owner is absent, present, or
  racing; a previously established session environment is not launch-attributed
  and is not rolled back.
- Supported script execution is not presumed.  The implementation must first
  prove a descriptor-pinned execution strategy that preserves interpreter
  semantics without a pathname race or leaked authority; until then such entry
  is refused rather than downgraded to an unsafe pathname exec.

## Guard

SPEC 0013 requires red-first fixtures for identical zero-side-effect D-Bus
refusal, fresh-only concurrent plain `Exec`, shell-free argv parsing, immutable
generation pinning, child-only environment, and the private lifecycle facade.

## Needs a human

None.  The owner ruled this boundary on #133 on 2026-08-29 and corrected the
plain-`Exec` owner distinction on the same issue.
