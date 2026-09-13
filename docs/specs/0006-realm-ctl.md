# SPEC 0006 — realmctl

- **Status:** Accepted (2026-08-26; generation contract reconciled by #159;
  endpoint capability correction 2026-09-10; control-transport correction
  2026-09-11; doctor observability and owned-health corrections 2026-09-13) —
  `theme apply`, `theme lint`, `theme diff`, and `doctor` implemented; the
  `doctor` VM acceptance and numeric tool floors remain open, and the other
  command groups are not yet implemented
- **Milestone:** M3, with `theme` and the argument surface in M1 and
  `orbit` / `ledger` in M2
- **Decisions:** [ADR 0004](../adr/0004-ndjson-control-socket.md),
  [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md),
  [ADR 0011](../adr/0011-session-integration-contract.md),
  [ADR 0012](../adr/0012-font-fallback-is-a-contract.md),
  [ADR 0013](../adr/0013-river-window-management-backend.md)
- **Implements:** [INTERFACES.md §4](../INTERFACES.md)
- **Supersedes / Superseded by:** Theme apply/diff behavior is refined and
  superseded by [SPEC 0011](0011-theme-activation-generations.md).

> Written before the code, as S14 requires. Implemented acceptance rows name
> their direct test evidence; remaining rows stay empty until their tests are
> written, watched to fail, and implemented against.

## Purpose

Two jobs, and they are less unlike each other than they look.

The first is the scriptable surface. realm is a keyboard-first desktop for
people who will want to bind something we did not think of, and ADR 0004 chose
a protocol they can drive from a shell. `realmctl` is the ergonomic front of
that: `orbit switch`, `ledger show`, `run`, `theme apply`.

The second is `doctor`, and it is the component a user meets when something is
wrong. Minimal Wayland desktops break in a small number of well-known ways —
a file dialog that hangs for twenty-five seconds, a screen share that offers
nothing, a black X11 cursor, six tofu boxes where the orbits should be — and
none of them look like a bug in the desktop, so the user files a bug against
their browser. [MVP.md](../MVP.md) lists `doctor` as capability 13 for that
reason, and M3's exit criterion is a fresh box on three distributions passing
it clean.

The same argument makes them one program: the fastest diagnosis is the one the
user runs themselves, and the second-fastest is the one they paste into an
issue.

## Scope

**In:** the command grammar and its arguments; the mapping from each command to
`realm_core::ipc::Request`; output discipline, `--json` and the exit-code table;
every `doctor` check with its probe, its verdict and its remedy text; behaviour
with no session, and with no session bus.

**Out:** rendering templates and publishing or comparing immutable generations
— that is `realm-theme`, [SPEC 0002](0002-theme-pipeline.md), and
[SPEC 0011](0011-theme-activation-generations.md); `realmctl` calls those
boundaries in-process and reports their generation-aware results. Serving the
socket, owning `RealmState`, and everything
`Capabilities` describes (`realm-session`). Drawing anything ([SPEC
0004](0004-realm-bar.md)). Deciding what a healthy session *is* — that is
ADR 0011's checklist, which this spec turns into checks rather than restates.

**Command name.** The installed binary and every documented invocation use
`realmctl`. There is no spaced spelling and no forwarding alias. This is part
of the accepted project namespace contract in SPEC 0025, not a packaging-time
choice.

## Behaviour

### 1. Command surface

```
realmctl [--json] [--palette PATH] <group> <verb> [args]
realmctl [--json] [--palette PATH] doctor [--portal-roundtrip]
realmctl --version
```

The control endpoint is always the fixed descendant resolved by SPEC 0007;
`realmctl` has no `--socket` or `REALM_SOCKET` override. Isolated tests use its
non-production `RuntimeDirResolver`, never an arbitrary socket path.
`--palette` overrides the palette search order (`$XDG_CONFIG_HOME/realm/palette.toml`,
then the shipped `palette.toml`) for theme commands and for `doctor`'s palette,
theme and font checks. The two global options may appear before or after
`doctor`; isolated tests may instead inject their non-production resolvers.
The override and `--portal-roundtrip` let tests and the NixOS VM point at a
fixture without an environment dance. `--portal-roundtrip` is
`doctor`-only and is named by [SPEC 0005](0005-session-startup.md) §8: it makes
a real `FileChooser.OpenFile` call, which opens a dialog, so it is opt-in.

| Command | Request sent | Response consumed |
|---|---|---|
| `theme apply` | none — generation-only `realm_theme::apply` runs in this process and reports `GenerationPublicationOutcome`; no post-apply notification | — |
| `theme lint` | none — `realm_theme::lint` | — |
| `theme diff` | none — generation-aware, read-only `realm_theme::diff` | — |
| `orbit list` | `ShowLedger(None)` | `Ledger(Vec<OrbitLedger>)` |
| `orbit switch N` | `SwitchOrbit(N)` | `Ok` \| `Error` |
| `ledger show [N]` | `ShowLedger(Some(N))` or `ShowLedger(None)` | `Ledger(Vec<OrbitLedger>)` |
| `run ARGV…` | `Spawn(argv)` | `Ok` \| `Error` |
| `doctor` | `Hello`, then `GetHealth` (new); skipped entirely when no session is running | `Hello`, `Health` (new) |

For each ordinary invocation that needs a live session, `realmctl` calls its
selected `RuntimeDirResolver` exactly once. If resolution succeeds, it consumes
that one `RuntimeDir` to derive one `ClientEndpoint` and retains that endpoint
across SPEC 0007's bounded connect schedule. Every attempt calls
`ClientEndpoint::connect(&self, "realmctl")`; that call makes exactly one
descriptor-relative connection attempt, reopens and validates `realm` relative
to the retained runtime fd without rereading `XDG_RUNTIME_DIR`, and completes
Hello before returning `Client`. `realmctl` owns the retry driver and its
injected sleeper/time seams. At driver start it fixes absolute not-before
offsets of 0, 10, 30, 70, 150, and 310 ms. Attempts run sequentially and never
overlap. After a retryable completion, it sleeps only until the next target if
that target remains in the future; if the attempt finished late, the next
attempt starts immediately without moving backward. Immediate failures
therefore start exactly at all six targets, and no sleep follows the sixth
failure. Only `ClientError::MissingRealm` and `ClientError::Refused` advance the
schedule; version mismatch and every other terminal client error fail
immediately.

`theme apply`, `theme lint`, and `theme diff` remain session-independent: they
do not call a runtime resolver, derive a `ClientEndpoint`, or open the control
socket. `doctor` does not require a successful socket connection. Its optional
session-health connection uses the same single resolver/capability rule.
Missing `XDG_RUNTIME_DIR` or `realm`, including an absent socket, is the
existing no-session diagnostic path in §4. A retained endpoint that remains
refused after the retry schedule, an unsafe path capability, timeout, EOF,
malformed response, or other terminal transport failure makes
`session/socket` fail: an existing but unusable daemon endpoint is not the same
as no session. A version mismatch makes `session/socket` pass,
`session/protocol-version` fail with both versions, and health-only checks skip
because `GetHealth` was correctly not sent. `doctor` continues every other
check that can run and never maps any of these outcomes to exit 3.

`run` is fire-and-forget by construction. `Response::Ok` means the session
accepted the argv, not that the program started — `execve` fails after the fork
and there is nowhere to report it to. The session logs the failure; `realmctl`
says "accepted" and not "launched", because the difference matters to someone
debugging a `.desktop` entry.

#### Gaps in the current `Request` enum

Four of these are additions and one is a correction. Each is a revision of
[SPEC 0001](0001-realm-core-contracts.md) made in the same commit as the change.
Before the first production `realmctl` control command ships, the typed Error
kind, complete `OrbitLedger` fields, `GetHealth`/`Health` schema, and a
corresponding `PROTOCOL_VERSION` bump must land together. The pre-v2 enum
may support transport tests, but it is not a final production server/client
contract. Across that bump, the Hello request/response envelope remains
decodable in both directions so a mismatch can always report both versions.

Two things worth correcting first, because they are easy to misremember.
`Response::Ledger` is **not** orphaned: `Request::ShowLedger(Option<usize>)`
produces it, and `ipc::tests::responses_round_trip` exercises it. And there
*is* a theme-adjacent request, `Request::ReloadTheme`; its old apply/reload
meaning is retired and it is not sent by the supported theme path.

| # | Gap | Addition |
|---|---|---|
| 1 | `Request::ReloadTheme` is documented as "re-read `palette.toml`, re-render templates, hot-reload clients", which conflicts with generation-only future-launch activation | Retire that meaning. The supported `theme apply` neither sends this request nor preserves a notify-only replacement. Any later wire compatibility or live upgrade is a separate #22 design and must be generation-aware; it cannot reload on pointer switch. A key action may spawn `realmctl theme apply`, whose effect is still future-launch-only |
| 2 | No request reports the session's health. `WmBackend::name()` is documented "shown by `realmctl doctor`" and `Capabilities` "`realmctl doctor` prints this" ([INTERFACES.md §1](../INTERFACES.md)) — and neither is reachable over the wire | `Request::GetHealth` → `Response::Health(Box<SessionHealth>)` carrying only facts the live daemon owns: session build version, `PROTOCOL_VERSION`, backend `name()`, `Capabilities`, the actual bound compositor interface names and negotiated versions, whether `river-layer-shell-v1` is being served, the current session-entry `DEGRADED` codes, and monotonic uptime in integer milliseconds. The daemon does not select a palette or render glyphs; `doctor` resolves its own `--palette` and runs its own font probe |
| 3 | `Capabilities` lives in `docs/INTERFACES.md` as a sketch, not in `realm-core`, and its `unsupported: Vec<&'static str>` cannot be deserialised into an owned value — yet it is exactly what `doctor` must print, entries like `"unclipped-dimension-quantisation"` included | Move it into `realm_core::ipc` with `Serialize`/`Deserialize`, and make `unsupported` a `Vec<String>`. It is a wire type now, not just a trait's return |
| 4 | `Response::Error { message }` carries prose only, so a caller can map a refusal to an exit code only by matching on English | Add `kind`, a kebab-case enum: `unknown-request`, `bad-argument`, `no-such-orbit`, `no-focused-window`, `backend-refused`, `internal`. Exit codes come from data, not from a string |
| 5 | `OrbitLedger` has `orbit`, `rune`, `name` and `windows`, but nothing says which orbit is active or what layout it holds, so `orbit list` cannot print what the bar shows without a second round trip | Add `active: bool` and `layout: Layout` |

The closed v2 health shape is:

```rust
pub struct InterfaceVersion {
    pub name: String,
    pub version: u32,
}

pub struct SessionHealth {
    pub session_version: String,
    pub protocol_version: u32,
    pub backend_name: String,
    pub capabilities: Capabilities,
    pub bound_interfaces: Vec<InterfaceVersion>,
    pub layer_shell_served: bool,
    pub degraded_codes: Option<Vec<String>>,
    pub uptime_ms: u64,
}
```

`Some(empty)` is the positive statement that the current entry finalized no
degradation; `None` means its bounded handoff was absent or rejected and makes
`session/degraded` warn rather than lie. Codes are the known SPEC 0005 spellings
in entry observation order without duplicates. River's interface vector is the
five SPEC 0003 rows in that table's order and records the versions actually
bound, not every advertised global. All strings and vectors are owned and
bounded by the existing control-frame and backend limits.

[SPEC 0004](0004-realm-bar.md) lists a further three additions the bar needs
(`RealmState::grimoire`, an elision glyph, `GetKeymap`). They are disjoint from
these.

### 2. Output discipline

**Human-readable by default.** Aligned columns, lower case, no decoration that
does not carry information. Diagnostics and progress go to stderr; the answer
goes to stdout, so `realmctl orbit list | grep` behaves.

**`--json` for scripts.** Exactly one JSON object on stdout, no NDJSON, nothing
else on the stream — a warning that would have gone to stdout goes to stderr
instead. The object's shape is the `Response` it came from wherever there is
one, so `realmctl ledger show --json` round-trips `Response::Ledger` and a
script can rely on `realm-core`'s serde definitions rather than on a format
invented here. `theme lint` and `theme diff` shapes are refined by
[SPEC 0020](0020-realmctl-theme-json.md); `doctor` remains defined here and
versioned by `PROTOCOL_VERSION`.

**Colour** only when stdout is a terminal and `NO_COLOR` is unset, and only as
ANSI indices — never a truecolor literal. The generated ANSI theme already maps
those indices to the palette (SPEC 0002), so `realmctl` picks up the user's
theme for free and no colour is written down twice
(`docs/PITFALLS.md`).

**Exit codes.**

| Code | Meaning |
|---|---|
| 0 | Success. For `doctor`, every check passed or warned |
| 1 | The command ran and the answer is negative: a fatal lint finding, one or more failed `doctor` checks, or a `theme diff` that found changes |
| 2 | Usage error — unknown group or verb, missing or invalid argument. Caught client-side; the socket is never opened |
| 3 | No session: the fixed endpoint remains absent or refused after SPEC 0007's bounded startup retry |
| 4 | Protocol version mismatch at the handshake |
| 5 | The session refused the command; derived from `Response::Error::kind` |
| 6 | Any terminal control path/transport/I/O failure other than version mismatch, or a filesystem/I/O failure while applying or diffing a theme, including `OutcomeAmbiguous`. An apply not known to have committed is not reported as activated; diff never mutates state (SPEC 0011) |

Code 1 is deliberately the same for "your palette is unreadable", "a doctor
check failed" and "the theme is stale": all three mean *the command worked and
the answer is no*, which is what a script branches on. `theme diff` exiting 1 on
a difference follows `diff(1)`, and is documented in `--help` because it
surprises people otherwise.

`doctor` never exits 3. A missing session is one of the things it is there to
report, not a reason for it to fail.

#### `theme apply` publication outcomes

The CLI uses the `GenerationPublicationOutcome` variant directly. It does not
probe `current`, recover, retry, or translate an ambiguous result into success:

| Outcome | Human stdout | Human stderr | Exit |
|---|---|---|---|
| `Committed(generation)` | `generation <id> selected for future launches` | empty | 0 |
| `CommittedWithCleanupPending { generation, cause }` | `generation <id> selected for future launches` | `warning: generation <id> is durably selected for future launches; committed cleanup is pending: <cause>` | 0 |
| `OutcomeAmbiguous { candidate, cause }` | empty | `theme apply failed: activation is unconfirmed for candidate <id>; inspect generation state before retrying: <cause>` | 6 |

`<id>` is the validated lowercase generation identifier. In human output,
`<cause>` is one JSON string literal produced by the same serializer as
`--json`; this escapes quotes, backslashes, newlines, carriage returns, tabs,
and other control characters and therefore cannot inject another diagnostic
line. The cleanup-pending warning says the pointer is durably selected and only
journal cleanup is uncertain. The ambiguous diagnostic deliberately uses
`candidate` and `unconfirmed`, never `selected`, `active`, `current`, `applied`,
or another success word.

With `--json`, stdout is exactly one object in the field order shown and stderr
is empty. The exit codes remain those above:

```json
{"status":"committed","generation":"<id>"}
{"status":"committed-cleanup-pending","generation":"<id>","cause":"<cause>"}
{"status":"outcome-ambiguous","candidate":"<id>","cause":"<cause>","activated":false}
```

Only the object matching the returned variant is emitted. JSON encoding, not
string concatenation, supplies `<cause>`. `OutcomeAmbiguous` is a CLI failure
even though it is an outcome variant: the filesystem state does not justify an
activation claim. This mapping is local result handling, not a live-upgrade or
wire-compatibility protocol.

### 3. `doctor`

One check per row of [PITFALLS.md](../PITFALLS.md) that a running system can be
interrogated about. The rows that are closed by a test rather than by a probe
are listed at the end, so the mapping is complete and visibly so.

**The design principle:** every failing check prints the **user-visible symptom
it predicts**, not just the fact of the failure. A user who is here because
their file dialog hangs must be able to recognise their own bug in the output.
A check that says `env/wayland-display/dbus: FAIL` and stops has told them
nothing they could not see. It is also why the four `env/list-matches-entry`,
`units/restart-policy`, `portal/config` and `session/protocol-version` checks
are marked runnable in ordinary CI: a check nobody can run without a display is
a check that rots.

#### Verdicts

`ok`, `warn`, `FAIL`, `skip`, in a fixed six-character column so the output is
greppable. [SPEC 0005](0005-session-startup.md) §8 requires a non-zero exit on
any failure so `doctor` can be the CI gate for the three distribution jobs;
that is exit code 1 in the table above. `warn` is an honest degradation that leaves realm usable — missing
symbol fonts, a backend that cannot do server-side borders — and does not
affect the exit code. `skip` means the check could not be run and says why.

#### Deadlines

Every probe has one, and the largest is 2 s. A diagnostic that hangs *is* the
bug it is diagnosing: the 25-second portal hang would otherwise be reproduced
faithfully inside `doctor`. No blocking read is issued without a timeout.

#### The checks

**The check ids are not this spec's to invent.** [SPEC 0005](0005-session-startup.md)
§8 is explicitly written as input here: it names every check the session
contract needs, in the form `group/thing`, and its own acceptance criteria cite
those names (`env/wayland-display/dbus`, `wm/attached`). They are adopted
verbatim. This spec adds four the startup contract has no opinion about —
`env/stale`, `palette/lint`, `theme/outputs` and `fonts/attribution` — and
implements the rest.

**Two constraints inherited from SPEC 0005 §8.** `doctor` must not shell out to
tools that may be absent: `xlsclients`, `xdpyinfo` and `wayland-info` are on
none of the three targets by default. It opens the sockets and makes the bus
calls itself — the systemd user manager's environment and each unit's
`ActiveState` are read as D-Bus properties on `org.freedesktop.systemd1`, not
by parsing `systemctl` output, and XWayland is checked by connecting to its
socket. The only commands it runs are the reused tools' own `--version`, where
the tool's absence *is* the finding, and `gsettings get` for the two cursor
values. Each command has a deadline inside `doctor`'s one overall deadline.
`gsettings` is already a declared session dependency; absence, timeout, a
missing schema, or malformed output is reported rather than treated as a
passing cursor check.

The installed command must make those deliberately permitted command probes
reachable without inheriting an interactive shell's `PATH`. Native packages
use their normal system paths. The Nix package wraps `realmctl` with only
`gsettings` and the reused tools it probes; it does not import or publish that
private path through the session environment. Likewise, when packaging wraps
the session entry with an executable launcher shim, `env/list-matches-entry`
must inspect the installed entry payload rather than treating the readable
shim as a malformed copy of the source.

| id | What it checks | Probe | Fail, and what it prints |
|---|---|---|---|
| `env/identity` | `XDG_CURRENT_DESKTOP=realm`, `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP=realm` in this process | `std::env` | *"Portals will pick the wrong backend and screen share will silently fail."* |
| `env/wayland-display/process` | `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` in this process | `std::env` | *"You are not inside a realm session."* Remedy: run this from a terminal in the session |
| `env/wayland-display/systemd` | it reached the systemd user manager | `Environment` property on `org.freedesktop.systemd1.Manager` | *"User units come up displayless; nothing is themed after login."* Remedy: the `import-environment` line, step 3 of the session entry |
| `env/wayland-display/dbus` | the bounded functional outcome of activating the portal in place of an unreadable D-Bus activation value | activate the portal and read `org.freedesktop.portal.FileChooser` `version`, 2 s deadline. There is no public API that reads the activation environment back, so this is a usability proxy, not evidence of any exact value | *"File dialogs will hang for about 25 seconds and then fail."* Reports only the failed activation/property observation. Possible causes name an absent or stale activation environment, a broken portal service, and a missing or broken selected backend; the first remedy is the `dbus-update-activation-environment --systemd WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE XDG_SESSION_DESKTOP XDG_RUNTIME_DIR` step, run before anything touches the bus |
| `env/desktop/systemd`, `env/desktop/dbus` | `XDG_CURRENT_DESKTOP=realm` reached systemd exactly; the D-Bus id reports only the functional portal-proxy outcome because its exact activation value is unreadable | systemd's exact `Environment` property; for D-Bus, the same functional portal proxy as above, explicitly marked `value unobservable` | *"Screen share will offer no sources, with no error anywhere."* The D-Bus result never claims that the proxy proves which desktop value was inherited or which policy was selected |
| `env/agree` | the process and systemd views hold the same exact values; the D-Bus activation channel is reported separately as a functional proxy | process and systemd compared; D-Bus value shown as `unobservable` beside the proxy result | *"The import ran too early, or ran twice with different values."* Prints the two observable views side by side and fails on their disagreement; it never claims that three exact values agreed |
| `env/stale` | systemd's `WAYLAND_DISPLAY` names a socket that exists | the property, then `stat` under `$XDG_RUNTIME_DIR` | `systemd --user` and the session bus outlive a logout, always when lingering, so a login can inherit a display name pointing at a dead socket. *"Symptoms identical to never importing it at all."* Remedy: `systemctl --user unset-environment WAYLAND_DISPLAY`, then log in again |
| `env/cursor` | `XCURSOR_THEME`/`XCURSOR_SIZE` agree in the process and systemd views, the theme resolves to a directory containing `cursors/` under the icon search path, and GSettings agrees | process env, the systemd property, the icon search path, the GSettings value; the D-Bus activation values remain unobservable and are not claimed | *"The cursor will be the default black X11 arrow, or invisible over some surfaces, or will change size as it crosses a window."* Names the observable places, because setting one leaves it wrong in the other |
| `env/xwayland` | `DISPLAY` agrees in the process and systemd views when XWayland is up and its socket answers | compare the observable values and connect to the X11 socket directly; report the D-Bus activation value as unobservable rather than inventing it. Realm's integer-only XWayland policy has no separately published runtime value, so this check does not claim to have probed one | `warn`. Reports honestly that the session entry's `DISPLAY` import is a known gap where the compositor does not hand its `:N` back. *"X11 apps absent, or blurred on a scaled output."* |
| `env/list-matches-entry` | `doctor`'s variable list is identical to the session entry's | both lists, compared; a **CI** check with no session needed | *"A variable was added in one place and forgotten in the other."* SPEC 0005 A3 |
| `units/target` | `realm-session.target` is active and its `.wants` symlinks exist | `ActiveState` over D-Bus | *"A target that starts nothing and reports success."* |
| `units/wm` | the window manager's unit is `ActiveState=active`, with `ConditionResult` reported **separately** | both properties | an unmet `ConditionEnvironment=` leaves a unit `inactive (dead)` with `ConditionResult=no`, and `systemctl start` still exits 0 with nothing in `--failed`. *"Nothing started and nothing complained."* Prints the condition that was not met |
| `units/bar` | `realm-bar.service` is active or cleanly restarting | `ActiveState`, `NRestarts` | *"The bar is gone and nothing said so."* |
| `units/restart-policy` | the shipped units carry SPEC 0005 §4's policy | the unit files; a **CI** check | *"A crashed bar takes the session with it."* |
| `units/idle-lock` | an idle and a lock unit are part of `graphical-session.target` | the dependency list over D-Bus | `skip` until the lock-screen `needs-human` question in ADR 0011 is answered, and says so. *"The lid closes and the session stays unlocked."* |
| `wm/attached` | a live health response proves realm holds river's window-management global; a refusal reports a possible foreign holder without inventing its identity | `GetHealth`; outside a live session, exit 69 and independently available process evidence. River's `unavailable` response does not identify the holder, so a pid or command line is printed only when a separate observation proves it | river answers `unavailable` to a second window-management client, so the supervised window manager never starts and a naive restart policy loops forever, burying the message. Without independent holder evidence, prints *"another window manager may hold river's global; holder identity unavailable"*. *"An inert compositor: windows are never placed."* |
| `wm/layer-shell` | realm is serving `river-layer-shell-v1` | `GetHealth`'s flag, plus `units/bar` | *"The bar never appears, and it looks like the bar's fault rather than the window manager's."* Points at ADR 0013 |
| `wm/capabilities` | the backend's name and the `Capabilities` it reports, `unsupported` included | `GetHealth` | any `unsupported` entry — `"unclipped-dimension-quantisation"` is the one river can produce — is a `warn` naming the realm behaviour that will not work. *"A backend gap that looks like a bug."* |
| `wm/protocol-version` | the actual negotiated interface versions satisfy SPEC 0003 §1's accepted bind/refusal ranges | `GetHealth`'s interface list | *"A routine river upgrade breaks the session."* Remedy: install the supported river 0.4.x realm ships. Neither this check nor the header claims an unobservable compositor package version |
| `portal/answers` | a backend answers on `org.freedesktop.portal.Desktop` without a pause | a cheap property read, 2 s deadline | *"'Open File' does nothing in Firefox."* Remedy names the packages: `xdg-desktop-portal` plus `xdg-desktop-portal-gtk` and `xdg-desktop-portal-wlr` |
| `portal/config` | the effective `realm-portals.conf` is found, names a backend per interface, and the named installed `.portal` metadata advertises those interfaces | the config search path and the `.portal` metadata; a **CI** check on the shipped file and a VM check on the installed effective files. The portal exposes no API for the selected backend identity, so this check never claims which backend the running frontend chose | *"Behaviour that changes with whatever happens to be installed."* |
| `portal/filechooser` | `--portal-roundtrip` only: a real `FileChooser.OpenFile` returns a request handle within 2 s | the D-Bus call, then cancel the request | `skip` unless `--portal-roundtrip` is given, because it opens a dialog. *"The 25-second hang, reproduced deliberately and bounded"* |
| `portal/screencast` | the `ScreenCast` interface exists and the configured backend implements it | introspection | *"Screen sharing silently produces nothing."* The real capture is hardware-only and is not attempted |
| `session/socket` | SPEC 0007's fixed endpoint exists and answers `Hello` | one frame | `skip` with no session (§4). `FAIL` when the path exists but nothing answers: *"The session daemon has died; windows are unplaced and keys are dead."* |
| `session/protocol-version` | the session's version equals `ipc::PROTOCOL_VERSION` | `Response::Hello` | *"The CLI and the session are from different builds and would misread each other's frames."* Both versions printed |
| `session/degraded` | every `DEGRADED <CODE>` in force for this session | `GetHealth`'s startup-code list, sampled from SPEC 0005's bounded per-incarnation handoff rather than inferred from the append-only historical log | any code present is a `warn` reprinting the entry's own sentence. *"A degraded session pretending to be healthy."* This is the check that stops a session that started with no cursor theme from reading as clean |
| `palette/lint` | the palette parses and passes `Palette::lint` | `Palette::load` on the resolved path, then `lint()`; names the file used, user or shipped, and prints the accent hue separations | `FAIL` on any fatal `Finding`, `warn` otherwise. Prints every finding through its `Display` impl — `error text.normal: contrast 1.02:1 on background.pane is below 4.5:1` — not the first |
| `theme/outputs` | the fully validated current generation matches the candidate rendered from the palette | the generation-aware comparison `theme diff` makes | `warn`: *"Future launches still select the previous generated theme."* Remedy: `realmctl theme apply` |
| `fonts/glyphs` | the glyph inventory against the chain in `palette.toml` | build the database from `typography.family` + `typography.fallback`, then `glyphs::Probe::run`; print `Probe::summary()` verbatim | **`warn`**, never `FAIL`: the runes are non-essential by ADR 0012 and realm degrades to digits rather than tofu. *"Orbit runes draw as the digits 1–6 and the bar looks plain."* Prints `substituting for ᚠᚢᚦ…` and the package to install |
| `fonts/attribution` | **which** family supplied each at-risk glyph — the six runes and `𓂃` | the resolved chain, per codepoint | `warn` when a family outside `typography.fallback` supplied one. *"Runes render in colour at the wrong size."* This is how an emoji font hijacking the symbol range becomes visible instead of merely puzzling |
| `tools/floors` | the reused tools are installed and their parsed versions are reported; numeric floor comparison remains unresolved below | each tool's own `--version` | `warn` naming any absent or unparseable tool and its package. With all versions observed, `skip` explicitly says that cross-target numeric floors are not yet accepted; it must not report `ok` merely because a version string exists. *"charon, horus or thoth will be missing or unthemed"* (ADR 0007) |

The report order is the following exact 32-id sequence, independently of probe
completion order:

1. `session/socket`, `session/protocol-version`, `session/degraded`;
2. `wm/attached`, `wm/layer-shell`, `wm/capabilities`, `wm/protocol-version`;
3. `env/identity`, `env/wayland-display/process`,
   `env/wayland-display/systemd`, `env/wayland-display/dbus`,
   `env/desktop/systemd`, `env/desktop/dbus`, `env/agree`, `env/stale`,
   `env/cursor`, `env/xwayland`, `env/list-matches-entry`;
4. `units/target`, `units/wm`, `units/bar`, `units/restart-policy`,
   `units/idle-lock`;
5. `portal/answers`, `portal/config`, `portal/filechooser`,
   `portal/screencast`;
6. `palette/lint`; `theme/outputs`; `fonts/glyphs`, `fonts/attribution`;
   and `tools/floors`.

Human and JSON output contain all 32 ids in that order, including every
`skip`; neither sorting nor probe completion order may change it. The unit name
is the settled `realm-wm.service` from SPEC 0005 and its shipped unit.

**When D-Bus is absent entirely.** The design says nothing about this and it is
a real case: a container, a `su -` shell, a distribution without
`dbus-user-session`. If `DBUS_SESSION_BUS_ADDRESS` is unset and no socket
exists at `$XDG_RUNTIME_DIR/bus`, `doctor` attempts no bus call at all. The
verdict then depends on context, because the same absence means two different
things:

- **A realm session is running** (`session/socket` answered): every bus-dependent
  check is `FAIL`. There is no session bus in a graphical session that should
  have one; portals cannot work, `DBusActivatable` entries will not launch, and
  the session entry should already have logged `DEGRADED NO-SESSION-BUS` for
  `session/degraded` to find.
- **No session is running:** the same checks are `skip`, with *"no session bus:
  nothing in this group is meaningful outside a session"*. The user is checking
  their configuration from a TTY, and failing them for the absence of something
  they did not ask for would train them to ignore the output.

#### Output shape

```
realmctl doctor - realmctl 0.1.0, protocol 2
2026-08-26T14:32:11+01:00 | Fedora 44 | kernel 6.12.4
  river backend | realm-wm 0.1.0

session
  ok    socket            /run/user/1000/realm/ctl.sock - realm-wm 0.1.0
  ok    protocol-version  2 == 2
  ok    degraded          no DEGRADED codes in this session

wm
  ok    attached          realm-wm holds river's window-management global
  ok    layer-shell       river-layer-shell-v1 served; realm-bar.service active
  warn  capabilities      river - unsupported: unclipped-dimension-quantisation
  ok    protocol-version  river_window_manager_v1 v5, river_xkb_bindings_v1 v3

environment
  ok    identity          XDG_CURRENT_DESKTOP=realm  XDG_SESSION_TYPE=wayland
  ok    wayland/process   WAYLAND_DISPLAY=wayland-1
  ok    wayland/systemd   present
  FAIL  wayland/dbus      portal did not answer within 2000 ms
        symptom  file dialogs hang for about 25 seconds, then fail;
                 screen sharing offers nothing
        cause    D-Bus-activated portal did not become usable; activation
                 environment, portal service, or selected backend may be broken
        fix      ensure the session entry runs, before first bus use:
                 dbus-update-activation-environment --systemd \
                   WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_TYPE \
                   XDG_SESSION_DESKTOP XDG_RUNTIME_DIR
                 then verify the portal service and selected backend packages

fonts
  warn  glyphs            fonts: 24/37 glyphs covered; substituting for ᚠᚢᚦᚨᚱᚲ𓂃
        symptom  orbit runes draw as the digits 1-6; the bar looks plain
        fix      install Symbols Nerd Font Mono, or Symbola
  ok    attribution       runes <- Symbols Nerd Font Mono (chain position 2)

32 checks: 25 ok, 3 warn, 1 failed, 3 skipped
```

(An excerpt: the `units`, `portal`, `palette`, `theme` and `tools` groups are
omitted above, which is why fewer lines are shown than the tally counts.)

The header line carries the facts a bug report needs and the user does not have
to be asked for: tool version, protocol version, distribution, kernel, backend
name, negotiated compositor interface versions, and session version. Wayland
does not expose River's package version, so the report does not relabel the
backend's tested target as an observed compositor release. The whole output is
plain ASCII apart from glyph names, wraps at 80 columns, contains no ANSI when
redirected, and is therefore pasteable as-is.

`--json`:

```json
{
  "tool": "realmctl doctor",
  "version": "0.1.0",
  "protocol": 2,
  "checks": [
    {"id": "env/wayland-display/dbus", "group": "environment", "status": "fail",
     "summary": "portal did not answer within 2000 ms",
     "symptom": "file dialogs hang for about 25 seconds, then fail",
     "cause": "D-Bus-activated portal did not become usable; activation environment, portal service, or selected backend may be broken",
     "remedy": "publish the documented activation environment before first bus use, then verify the portal service and selected backend packages",
     "data": {"deadline_ms": 2000, "elapsed_ms": 2000,
              "activation_environment_values": "unobservable"}}
  ],
  "summary": {"ok": 25, "warn": 3, "fail": 1, "skip": 3}
}
```

The `checks` array has one entry per check id, in the same order as the human
output, whatever the verdicts — so a script can index by `id` and a diff
between two machines lines up.

#### Rows of the register `doctor` does not check

Named so the mapping is complete, and because a check that cannot exist is
worth writing down once rather than proposing repeatedly:

| PITFALLS row | Why not, and what guards it instead |
|---|---|
| Rounding loss, off-by-one on odd resolutions | `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` — a property of the code, not of the machine |
| Focus causes relayout | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` |
| Redraw on a timer | `state::tests::revision_alone_does_not_force_a_redraw`, plus ADR 0008's idle frame-count test |
| A client quantises its proposed size | planned M2 pixel-diff test; not observable from a CLI |
| `realm-session` stalls | planned M2 watchdog. By the time `doctor` could report it the session is dead |
| Position submitted during the management phase | planned M2 sequence-order test. It kills the connection at startup, so there is no session left to ask |
| Contrast implemented as a filter | `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating` |
| Colour written down twice | the M1 CI grep |
| Fractional scaling blur | planned M2 scale test |
| Flatpak apps ignore the theme | a documented limit, not a fault (ADR 0005) |
| Works on the author's distro only | the three distro CI jobs — whose gate is `doctor` exiting 0, so this row is checked *by* running it, not *in* it |

### 4. Working without a session

`theme lint` and `doctor` must be useful when realm is not running, because that
is exactly when a user needs them: before a first login, in a container, over
SSH into a machine that will not start, or from a TTY after a failed session.

- **`theme lint`** never opens the socket. It resolves the palette
  (`--palette`, then `$XDG_CONFIG_HOME/realm/palette.toml`, then the shipped
  file), parses it, lints it, and prints every finding. It works on a machine
  with no Wayland, no D-Bus and no realm installed beyond the binary itself.
  `theme diff` is the same, plus read-only resolution and complete validation of
  `current` followed by comparison with its manifest-listed normalized outputs.
  It does not initialize or recover generation control state, create a lease,
  publish, write an output, or reload a process.
- **`doctor`** runs every check that does not need the socket, marks the rest
  `skip` with `no realm session is running`, and prints a banner saying so:
  `no realm session is running — 19 of 32 checks skipped`. It exits 0 if nothing
  that *ran* failed. Palette lint, font coverage and attribution, portal
  backend presence, cursor theme resolution, theme staleness and tool versions
  all still run and are all still worth having: they are precisely the checks
  that predict whether the *next* login will work.
- **`theme apply`** is session-independent. It publishes a sealed generation,
  reports its `GenerationPublicationOutcome`, and sends no notification whether
  or not a session is running. A committed pointer affects future launches
  only; existing processes remain pinned to their selected generation.

A missing session is never spelled as a crash. `realmctl orbit switch` with no
session prints the socket path it tried and how to start a session, and exits
3.

## Acceptance criteria

Each row is one happy path and becomes one test.

| # | Given / When / Then | Test |
|---|---|---|
| B1 | Given the shipped palette and no session running, when `theme lint` runs, then it exits 0 and prints the six accents' hue separations | `theme_cli::lint_shipped_palette_is_session_independent_and_prints_hue_separations` |
| B2 | Given a palette with a fatal finding, when `theme lint` runs, then it exits 1 and prints every finding, not only the first | `theme_cli::lint_bad_explicit_palette_exits_one_and_prints_every_fatal_finding` |
| B3 | Given a fully validated current generation and one edited accent, when `theme diff` runs, then it exits 1, reports only lexicographically sorted `added`, `removed`, and `byte-different` normalized outputs, and performs no control initialization, recovery, lease, publication, pointer change, output write, or reload | `theme_cli::diff_after_palette_edit_is_sorted_and_does_not_mutate_generation_tree`; `theme_cli::diff_refusal_for_missing_current_does_not_mutate_generation_tree` |
| B4 | Given a session with windows in orbits 1 and 3, when `orbit list` runs, then it prints six rows carrying rune, name, window count and layout, with orbit 1 marked active | |
| B5 | Given a running session, when `orbit switch 3` runs, then it sends `Request::SwitchOrbit(3)`, prints the new orbit and exits 0 | |
| B6 | Given a session whose backend has disconnected, when `orbit switch 2` runs and the session answers `Error { kind: backend-refused }`, then the CLI prints the session's message and exits 5 | |
| B7 | Given immediate retryable MissingRealm or Refused results and an `XDG_RUNTIME_DIR` change after the first attempt, when `orbit switch 2` runs, then exactly one `RuntimeDir` is resolved, one `ClientEndpoint` is reused for sequential named-client attempts at absolute not-before offsets 0, 10, 30, 70, 150, and 310 ms, no sleep follows the sixth failure, and exhaustion exits 3 naming the original capability's display path and suggesting how to start a session. If an attempt completes after the next target, the next starts immediately without overlap or backward time | `realm_ctl::tests::startup_retry_uses_exact_attempt_timestamps_and_exit_classes`, `realm_ctl::tests::late_retryable_attempt_skips_elapsed_sleep_without_overlap` |
| B8 | Given a session answering `Hello` with a different `version`, when any command runs, then the CLI refuses before sending anything else, prints both versions and exits 4 | |
| B8a | Given any terminal client path/transport/I/O error other than version mismatch, when a live-session command runs, then it is not retried and exits 6; an application `Response::Error` remains a normal response and maps by its typed kind to exit 5 | `realm_ctl::tests::terminal_transport_errors_exit_six_without_retry` |
| B9 | Given a session with three windows in orbit 1, when `ledger show 1 --json` runs, then stdout is exactly one object that deserialises as `Response::Ledger` with the windows in ledger order and the focused one marked | |
| B10 | Given a running session, when `run foot -e yazi` runs, then it sends `Request::Spawn(["foot","-e","yazi"])`, exits 0 without waiting, and reports the argv as accepted rather than launched | |
| B11 | Given a healthy fresh-login session, when `doctor` runs, then every resolved check reports `ok` or `warn`, idle lock, the unrequested file-chooser round trip and unresolved tool floors report their accepted explicit `skip`, the header names the tool, protocol, distribution, kernel, backend and negotiated compositor interfaces without inventing a compositor package version, and it exits 0. The session entry starts portal activation non-blockingly after importing its environment, rather than making the fixture warm it manually; the installed-package fixture invokes `realmctl` without an interactive-shell path and proves its permitted `gsettings` and reused-tool probes remain reachable | `session-boots` NixOS VM (pending first passing run); `realmctl-command-runtime` Nix package check |
| B12 | Given the session entry deliberately suppresses the D-Bus activation-environment import and no earlier activation supplied the graphical-session values, when `doctor` runs and its portal proxy cannot become usable, then `env/wayland-display/dbus` fails within its 2 s deadline, prints the 25-second-hang symptom, reports the observed portal failure without claiming to have read a missing variable, names the activation environment, portal service and selected backend as possible causes, prints the `dbus-update-activation-environment` remedy, and exits 1 | `realmctl::doctor::tests::portal_failure_reports_an_observation_and_only_possible_causes` (diagnostic mapping only; missing-activation VM case pending) |
| B13 | Given no session running and a font stack that covers ASCII only, when `doctor` runs, then its optional session probe resolves at most one `RuntimeDir` and reuses one `ClientEndpoint`, the session checks are `skip` with a banner, `fonts/glyphs` warns with `Probe::summary()`'s wording, and it exits 0 rather than 3 | `doctor_cli::no_session_report_is_ordered_bounded_and_keeps_independent_warnings` |
| B14 | Given no session bus and no session running, when `doctor` runs, then the D-Bus and portal checks are `skip` and not `fail`, and it exits 0 | `doctor_cli::no_session_report_is_ordered_bounded_and_keeps_independent_warnings` |
| B14a | Given no runtime directory or no `realm` entry, a retained endpoint that remains refused, an unsafe endpoint, and a version-mismatched live endpoint, when `doctor` runs each case, then only the first case is the no-session `skip`; refused and unsafe endpoints fail `session/socket`; and the mismatched endpoint passes `session/socket`, fails `session/protocol-version` with both versions, sends no `GetHealth`, and never exits 3 | |
| B14b | Given the fixed result set completes in a different order and the portal proxy reaches its 2 s deadline, when human and JSON reports are emitted, then both contain the exact same 32 ids in the specified order, every shared portal-derived check uses that one bounded observation, and the command completes in under 3 s | `realmctl::doctor::tests::{human_and_json_reports_preserve_the_exact_32_check_order,portal_result_is_not_blocked_by_a_hung_systemd_probe,bus_deadline_is_absolute_from_probe_start,exited_probe_with_inherited_open_pipe_still_obeys_deadline}`; `doctor_cli::no_session_report_is_ordered_bounded_and_keeps_independent_warnings` |
| B14c | Given all three reused tools answer with parseable versions but no cross-target floors have been accepted, when `doctor` runs, then `tools/floors` reports the observed versions and an explicit unresolved-floor `skip`, never `ok`; absence or malformed version remains `warn`, and the acceptance row stays open until package/template compatibility establishes real minima | `realmctl::doctor::tests::observed_tool_versions_never_claim_unresolved_floors_pass` (floor acceptance remains open) |
| B15 | Given `theme apply` returns `Committed(generation)`, when the CLI reports it, then it exits 0 and reports exactly that generation as selected for future launches | `theme_cli::apply_reports_selected_future_generation_without_reload_or_session` |
| B16 | Given `theme apply` returns `CommittedWithCleanupPending { generation, cause }`, when the CLI reports it, then it exits 0, reports exactly that generation as selected for future launches, and emits the safely escaped committed-cleanup warning | `realmctl::tests::cleanup_pending_reports_selected_generation_with_escaped_warning` |
| B17 | Given `theme apply` returns `OutcomeAmbiguous { candidate, cause }`, when the CLI reports it, then it exits 6, emits no human stdout, safely reports the candidate and unconfirmed activation, claims no success, and performs no recovery or retry | `realmctl::tests::ambiguous_reports_no_success_stdout_and_escaped_cause` |

## Budgets

From [ARCHITECTURE.md §4](../ARCHITECTURE.md):

| Path | Budget | How it is held |
|---|---|---|
| `realmctl theme apply` | **< 150 ms** | templates rendered serially, then one complete generation is validated, sealed, fsynced, and published (SPEC 0011); no mutable-target equality shortcut is promised |

Two budgets belong to this component alone:

- **`doctor` completes in under 3 s wall clock**, with every probe individually
  bounded and the largest deadline 2 s. A diagnostic that hangs is the bug it
  is diagnosing.
- **No `realmctl` invocation may stall `realm-session`.** Under river a stalled
  window manager is a dead session, not a slow frame (ADR 0013). A CLI that
  stops reading its side of the socket must not block the daemon; the session
  is entitled to drop a wedged client, and `realmctl` must therefore treat a
  closed connection as an ordinary outcome rather than an error to retry.

`doctor` builds a font database to run the probe, which costs a fontconfig
scan — the largest single cost in the command and the reason the budget is
seconds rather than milliseconds. It happens once, in a process that exits.

## Failure modes

From [PITFALLS.md](../PITFALLS.md), `doctor` is the guard for the whole
"session integration — the classic killers" section: `WAYLAND_DISPLAY` never
reaching D-Bus, `XDG_CURRENT_DESKTOP` unset, no portal backend installed, the
cursor theme unset, XWayland unstyled or unimported, and no lock handling. It
is also the guard for "a unit is *skipped* rather than failed", "the user
manager outlives the session", "layer-shell not served", "a stale window manager
holds river's global", "protocol version drift after a river bump", "version
skew between components", and — via the three distro CI jobs whose gate is
`doctor` exiting 0 — "works on the author's distro only".

`theme lint` guards "unreadable palette after a tweak"; `theme diff` reports
candidate/current generation drift, while SPEC 0011's validation and pointer
commit rules prevent a partial generation from becoming selectable.

The failure this component must not cause itself: **a diagnostic that lies**. A
check that reports `ok` because its probe silently failed is worse than no
check, in the same way a diverged spec is worse than no spec. Every probe
distinguishes "ran and passed" from "could not run", which is what `skip`
exists for, and ADR 0011's planned negative test — deliberately skip the
`dbus-update-activation-environment` call and assert `doctor` reports it — is
the guard. A health check that has never been seen to fail is not a health
check.

## Open questions

- **Should `doctor` be able to fix anything?** A `--fix` that re-runs the
  environment import and re-applies the gsettings keys would close most
  failures in one step.
  *Recommendation: no, in M3. A diagnostic that mutates is a diagnostic you
  stop trusting, and the remedy lines are copy-pasteable. Revisit if the same
  remedy is pasted often enough to be worth automating.*
- **Reading the D-Bus activation environment directly.** The functional probe
  is a proxy; it cannot distinguish "the variable is missing" from "the backend
  is broken", and it says so in its output. Exact process and systemd values
  are compared; D-Bus activation values are labelled `unobservable`, never
  inferred from a successful or failed proxy.
  *Recommendation: keep the functional probe, since it tests what the user
  actually experiences, and revisit if a bus API appears.*
- **Numeric version floors for reused tools.** The accepted check exists, but
  no cross-target compatible minima have been accepted. The retained-source
  inventory currently records yazi 26.8.15 and starship 1.26.0 for prospective
  Debian/Fedora routes; NixOS uses the locked nixpkgs packages, and no verified
  btop minimum is recorded. Those concrete package selections are evidence to
  test, not floors to copy. Before implementing `tools/floors`, verify the
  shipped theme keys against the supported package versions on all three
  targets and accept the resulting minima. Until then `doctor` implements the
  fixed id by reporting each observed version and the explicit
  unresolved-floor `skip`; it never guesses a comparison or reports `ok`.
  Settling the floors and closing B14c remains a follow-up acceptance
  obligation before #72 or the full M3 gate can be called complete.
- **Resolved by SPEC 0003:** `Capabilities` lives in `realm_core::ipc` beside
  the other wire types, with owned unsupported-capability names. The backend,
  health response, and `doctor` share that one type.
- **A subscribe surface.** `Request::Subscribe` exists and nothing in this
  command set uses it. A `realmctl watch` emitting NDJSON `RealmState` frames
  would make the socket scriptable from a shell loop without `socat`.
  *Recommendation: M4; ADR 0004's `socat` example already covers the need, and
  a command with no consumer is a command with no test.*
