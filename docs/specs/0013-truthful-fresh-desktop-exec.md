# SPEC 0013 — Truthful fresh desktop Exec launch

- **Status:** Draft — candidate for acceptance; implementation is blocked until
  this contract is accepted, then proceeds red-first through its fixtures
- **Milestone:** M2
- **Decision:** [ADR 0018](../adr/0018-fresh-desktop-exec-only.md)
- **Issue:** [#133](https://github.com/Pipeliner/realms-de/issues/133)
- **Depends on:** accepted [SPEC 0011](0011-theme-activation-generations.md)
  and [SPEC 0012](0012-activation-launch-lifecycle.md)
- **Supersedes / Superseded by:** Replaces the superseded #163 candidate.  A
  later D-Bus activation spec may add a separate admission class.

## Purpose

Turn one desktop-file identity into at most one new child whose desktop argv and
directory come from one immutable desktop snapshot, and whose profile overlay
and generation-relative assets come from one sealed generation selected by the
accepted M2 owner.  Helm must never call an activation themed when it used a
shell, a mutable pathname, a D-Bus owner, or global environment state.

## Scope

**In:** typed desktop identity; bounded no-follow capture; a conservative main
desktop-entry and `Exec` subset; unconditional D-Bus refusal; immutable
execution-plan construction; child-only environment; and a one-shot handoff to
SPEC 0012's private lifecycle facade.

**Out:** D-Bus activation/owner discovery/retheming, public launch DTOs or
idempotency/history, session claims, lifecycle IDs/records/leases/transfer/gate
tokens/reconciliation, and exact consumer assets, packages, overlays and
consumption evidence.  Those belong respectively to a later D-Bus decision,
SPEC 0006/#117, SPEC 0012, and #135.  A raw argv, desktop action, file/URL
payload, caller environment override, or terminal wrapper is not this request.

## 1. Immutable admission before lifecycle mutation

The only request input is `DesktopFileId`: ASCII 1–255 bytes, ending in
`.desktop`, and otherwise `[A-Za-z0-9._-]`.  Slash, control bytes, NUL, a raw
argv, an action, or payload data is a preflight refusal.

Admission captures the XDG data roots once: `XDG_DATA_HOME` then
`XDG_DATA_DIRS`, using the XDG defaults whenever either is unset **or empty**.
`HOME` is required when `XDG_DATA_HOME` defaults.  Each root is absolute,
UTF-8, no-follow opened, and bounded to 64 roots/64 KiB combined spelling;
empty or relative explicit components refuse.  Each path component is a
directory owned by the effective UID or root and has no group/other write bit.
It searches the corresponding `applications/` trees descriptor-relatively, at
most 4096 entries and 64 levels across the entire request.  The desktop-file id
is the applications-relative path with `/` replaced by `-`.  The first root
containing one matching id wins; two paths in that root mapping to the id are
an ambiguity.  A winning `Hidden=true` masks lower entries and is refused as
absent.  A candidate is one no-follow regular file, owned by the effective UID
or root and with no group/other write bit.

The candidate is captured once as a no-follow regular file of at most 1 MiB.
Device, inode, size, mtime and ctime are checked before and after the read;
disagreement refuses.  The held bytes are UTF-8, have no BOM/NUL/CR, at most
4096 LF-terminated lines and 16 KiB per line.  Helm never rereads the pathname.
Duplicate groups, duplicate keys in the main group, malformed headers/keys, or
multiple `[Desktop Entry]` groups refuse.

Until admission produces an immutable `AdmittedDesktopPlan`, Helm may only do
bounded read-only lookup/capture and diagnostics.  It must not open, initialize,
recover, select, or GC a generation store; create an owner, process/lifecycle
lease, record, scope, group, or gate; mutate process/global/systemd/D-Bus
environment; query/contact an application or bus name; or execute profile or
desktop-entry code.  An independently live session claim is prerequisite state,
not a side effect of this request.

## 2. Main-group eligibility and D-Bus refusal

The sole authoritative group requires:

- `Type=Application`;
- non-empty unlocalized `Name`;
- `Hidden` and `NoDisplay` absent or exactly `false`;
- `Terminal` absent or exactly `false`; and
- no desktop action or payload request.

Desktop booleans are exactly lowercase `true` or `false`; another spelling
refuses.  Immediately after structural/main-group validation, Helm evaluates
`DBusActivatable`.  Absent or `false` may proceed.  `true` refuses identically
before parsing or using `Exec`, `TryExec`, or `Path`; before an owner or any
lifecycle/generation effect; and without deriving a bus name, querying an
owner, sending a D-Bus message, or treating `Exec` as fallback.  Bus presence,
owner presence, and ownership races cannot change this result.

A plain entry has no owner result.  Helm performs no process, PID-file,
executable-name, window, Wayland app-id, title, startup-class, or bus probe.
Two accepted requests for one entry may create two independent fresh launches.

## 3. Conservative Exec plan

`Exec` is mandatory ASCII source text and becomes argv, never a shell string.  Its value is
processed exactly once in this order: Desktop Entry general unescape;
whole-argument double-quote tokenization; one field-code expansion.  Expansion
is not reparsed as shell text, quoting, or another field-code pass.  Unmatched
quotes, malformed/unknown escapes, NUL/control bytes, empty argv/executable,
an executable containing `=`, unknown/lone/deprecated field codes, a field code
inside quotes, unquoted Desktop Entry reserved characters, or an embedded field
code other than `%%` refuse.

For a payload-free app-menu request, the only supported field codes are:

| Code form | Final argv effect |
|---|---|
| `%%` | literal `%` in its argument |
| standalone `%f`, `%F`, `%u`, `%U` | zero arguments |

At most one of `%f`, `%F`, `%u`, and `%U` may occur.  `%c`, `%k`, all deprecated
codes, `%i`, quoted field codes, and every other use refuse.  The parser does not
pretend to be a full desktop-entry launcher.  For this first contract, final
argv must contain exactly one argument: the supported executable itself.  A
payload field code that expands to zero may appear as a separate source token;
every static argument, `%%` result, or other nonempty second argument refuses.
Argument-bearing profiles are a later explicit extension, not a compatibility
fallback.

`TryExec`, if present, is one unquoted simple executable name or normalized
absolute path without field codes.  It is resolved and validated under the same
captured environment and executable policy as `Exec`; absence or failure
refuses.  Malformed/multi-token values refuse.
`Path`, if present and non-empty, is a normalized absolute no-follow-opened
directory; otherwise the plan uses `/` as its explicit working directory.  It
and every traversed parent are root-owned directories with no group/other write
bit.  Admission retains the directory fd and the facade revalidates it then
uses `fchdir` on that fd immediately before execution; a later pathname
replacement therefore cannot redirect the child.
Admission captures one `BaseEnvironment` at the request boundary: at most 256
UTF-8 `NAME=VALUE` entries totalling 128 KiB, with ASCII names matching
`[A-Za-z_][A-Za-z0-9_]*`, no duplicates, and unsigned-byte sorted names.
`PATH` is required and is at most 32 KiB/64 entries; every entry is a nonempty
absolute UTF-8 directory.  An unset, empty, relative, or duplicate `PATH`
component refuses.  A simple executable name is searched in that exact order;
the first existing candidate is authoritative and must satisfy the policy
below, otherwise admission refuses rather than searching past it.

The executable is that simple-name candidate or a normalized absolute
no-follow-opened file.  It must be regular, owned by root, executable, and not
writable by the effective UID; every traversed parent is root-owned and has no
group/other write bit.  Admission holds a no-follow `O_RDONLY|O_CLOEXEC`
descriptor and records its device/inode metadata.  It does not claim an fd alone
freezes bytes: this policy makes mutation by the invoking UID impossible; root
or equivalent package authority is outside this threat model.

The initial supported kind is Linux ELF64 little-endian: the held first 64 bytes
must carry `7f 45 4c 46`, class 2, data 1, version 1, `ET_EXEC` or `ET_DYN`, and
the running architecture's machine value (`EM_X86_64` on x86_64 or `EM_AARCH64`
on aarch64).  A short, malformed, foreign, or shebang file refuses.  After it
has transitioned to `running`, the lifetime owner forks exactly one profile
child.  Only that child applies the retained cwd/stdin/environment and calls
`fexecve(retained_fd, argv, envp)`; the owner remains the SPEC 0012 supervisor.
The child has a child-only close-on-exec error channel to its owner: successful
image replacement closes it, while unavailable or returned `fexecve` reports
the failure and exits.  The owner reads that report, reaps that exact child, and
writes SPEC 0012's terminal-failure witness.  Thus a failure before image
replacement executes no application.  Header validation deliberately does not
claim to predict invalid/truncated program headers, a missing interpreter,
noexec/LSM denial, or loader failure: those are the same post-gate no-exec
failure path.  A dynamic ELF's kernel-selected interpreter and any kernel
binfmt configuration are privileged platform trust, not a user-controlled
executable-path authority.  Scripts and every header-invalid/foreign kind
refuse before lifecycle effects.  No pathname retry or `PATH` re-resolution is
allowed after admission.

Header validity is necessary but not sufficient for admission.  During the
facade's locked resolution of N, before it publishes a process lease or a launch
record, it compares the held executable device/inode/header identity to its
private `ExecutableAllowlist<N>`.  This is a generation-bound,
deployment-provided policy; an empty, missing, or nonmatching policy kills/reaps
the still-inert owner and refuses with no lease or record.  #133 defines no
production entry: its tests inject one synthetic matching ELF identity.  #135
supplies real entries and must prove that none selects a shell, language runtime,
wrapper, or other interpreter of mutable code.  This division prevents a user
desktop entry from turning an arbitrary root-owned ELF into an approved profile.

## 4. Child-only immutable plan and lifecycle facade

Admission freezes the base environment above and produces an immutable
desktop-only plan:

```text
AdmittedDesktopPlan {
  desktop_snapshot: held bytes + source identity,
  argv: final argv bytes,
  executable: checked descriptor identity,
  cwd: checked directory descriptor identity,
  base_environment: bounded held entries,
}
```

Only after that complete desktop-only plan exists may Helm call the private,
one-shot M2 facade.  The facade alone creates the inert owner; under the
lifecycle and generation locks selects and validates N, including its
`ExecutableAllowlist<N>` before it publishes the process lease; then combines
the consumed admission and an opaque generation binding into its private
`ExecPlan<N>`.  #133 defines no concrete consumer overlay: its tests use a
synthetic facade-supplied binding, while #135 will define exact target names,
values, and consumer evidence.  Only then does the facade persist `preparing`;
verify/adopt ownership; transfer the lease; persist `adopted` and durable
`exec-open yes`; send the unique gate token; transition owner-side to `running`;
and permit one exec.  It exposes no selection, lease, record,
ownership-evidence, transfer, release, transition, or gate-token capability.

The facade combines the held base environment and child-only overlay into an
explicit sorted child `envp`.  It neither calls `setenv`/`unsetenv` nor modifies the
systemd manager, D-Bus activation environment, global `XDG_CONFIG_HOME`, or a
caller process.  It may not replace `PATH`, `HOME`, `XDG_RUNTIME_DIR`,
`DBUS_SESSION_BUS_ADDRESS`, `WAYLAND_DISPLAY`, `DISPLAY`, `LD_PRELOAD`, or
`LD_LIBRARY_PATH`.  The held base environment and every overlay name must not
begin `LD_`; otherwise admission refuses.  Every overlay name must be a valid
base-environment name, absent from the base environment, absent from the
protected name list, and unique.  The merged environment remains unsigned-byte
sorted and within the base 256-entry/128-KiB bounds.  Exact target-specific
names and values remain opaque #135 evidence.  A later N+1 commit cannot change
the consumed plan's argv, cwd, overlay, or generation-relative assets.

The held base environment permits only `HOME`, `PATH`, `XDG_RUNTIME_DIR`,
`DBUS_SESSION_BUS_ADDRESS`, `WAYLAND_DISPLAY`, `DISPLAY`,
`XDG_CURRENT_DESKTOP`, `LANG`, `LC_CTYPE`, `LC_MESSAGES`, `XDG_CONFIG_DIRS`,
`XDG_DATA_DIRS`, `XDG_CACHE_HOME`, `XDG_DATA_HOME`, and `XDG_STATE_HOME`.
Every other name refuses before lifecycle entry.  For `Terminal=false`, the
owner supplies child stdin from a retained no-follow `/dev/null` character
device descriptor (Linux major 1, minor 3); it does not inherit launcher stdin.
Output descriptors are not generation authority and remain the session's normal
launcher policy.

The profile child has an exact descriptor map.  Before `fexecve`, it changes
directory through the retained cwd fd and closes that original fd; duplicates
the retained `/dev/null` descriptor onto fd 0 and closes its original fd; and
retains only fd 0, the launcher-policy-approved non-authority stdout/stderr
descriptors, the executable descriptor marked `FD_CLOEXEC`, the child-to-owner
error-report descriptor marked `FD_CLOEXEC`, and any explicitly #135-authorized
read-only asset descriptors marked `FD_CLOEXEC` unless that later specification
states otherwise.  It closes or close-sweeps every other descriptor before the
call.  Therefore successful image replacement exposes neither lifecycle,
generation-store, lease/lock, gate, desktop-snapshot, cwd, `/dev/null` original,
nor other control authority to the application; the executable and error-report
descriptors disappear on success.  #133 defines no asset descriptors.

Picker code may return only `DesktopFileId`; Helm repeats this complete
admission.  Direct fuzzel application mode and raw `Spawn(argv)` cannot claim
this plan, generation, or verified themed launch.

## Acceptance criteria

Each row is red-first.  The exact test names are filled in when implementation
lands.

| # | Given / When / Then | Test |
|---|---|---|
| E1 | Given a valid, absent, or malformed tempting `Exec` with `DBusActivatable=true`, each bus state (absent, no owner, owner, racing owner), when admission runs, then every case returns the same refusal and spies observe zero owner/generation/lease/record/scope/group/gate/environment/exec or D-Bus/name-owner operations. | |
| E2 | Given two overlapping plain-`Exec` requests and a pre-existing same-name process, when both are admitted, then each owner forks and reaches exactly one distinct fresh profile-child exec while remaining the supervisor, and spies observe no owner/deduplication probe. | |
| E3 | Given quoting, escaping, supported field-code, and hostile shell metacharacter vectors, when `Exec` is parsed, then the final argv is byte-golden, no shell/glob/substitution/redirection runs, and malformed input refuses before lifecycle entry. | |
| E4 | Given XDG precedence, hidden masking, same-root collision, duplicate keys/groups, unsafe/symlink candidates, bounds, and replacement before/after capture, when admission runs, then it selects only one held valid snapshot or refuses without a lifecycle side effect. | |
| E5 | Given an admitted desktop argv/cwd/base environment, a synthetic binding to N, and a commit to N+1 before the owner receives its gate, when the child sentinel starts, then its argv/cwd/base environment equal the held desktop admission, its synthetic binding/assets are exclusively from N, and N remains protected until SPEC 0012 proves release. | |
| E6 | Given a synthetic generation binding, when the sentinel and parent/manager/bus environments are inspected, then only the child sees the exact synthetic overlay and the three external environments are byte-identical before/after. | |
| E7 | Given `Terminal=true`, an action/payload, unsupported code, malformed/absent/nonexecutable `TryExec`, invalid/over-bound/unallowed base environment, `LD_*` base entry, an invalid retained `/dev/null` authority, invalid PATH, user-writable executable/cwd path component, a shebang, short/header-invalid/foreign ELF, or no matching generation allowlist entry, when preflight runs, then it refuses with no leaked lease or record. | |
| E8 | Given an external integration-test crate, when it tries to name or construct the `pub(crate)` lifecycle ownership evidence, transfer/release/state/gate types, or bypass the facade, then compilation fails.  Existing public generation-selection APIs do not gain lifecycle-transfer/release/gate authority. | |
| E9 | Given failpoints from preflight through owner, lease, preparation, adoption, authorization, gate receipt, running transition, profile-child fork, child error report, and child exec (including `fexecve` unavailable, kernel, or loader failure after a header-valid ELF), when recovery runs, then there is at most one profile-child exec, no unleased live profile, no reopened gate, a returned child `fexecve` starts no application and makes the owner reap that exact child before it reaches SPEC 0012 terminal cleanup, and its existing recovery rows remain authoritative. | |
| E10 | Given a synthetic generation allowlist containing one root-owned matching-architecture header-valid ELF64, a shebang, arbitrary non-shebang text, short/header-invalid/foreign ELF, an unlisted root-owned interpreter, post-gate child `fexecve`/kernel/loader failure, and in-place/path replacement attempts by the invoking UID, when admission and child execution run, then only the listed ELF may run by the retained descriptor; every other candidate refuses before lifecycle effects; a post-gate failure executes no application and reaches terminal cleanup after the owner reaps the exact child; and a replacement cannot redirect executable or retained cwd. | |
| E11 | Given invalid, duplicate, oversized, `LD_*`, PATH-shadowed, unallowed, or overlay-colliding environment entries, an inherited stdin sentinel, and a synthetic binding, when admission/merge runs, then invalid input refuses before lifecycle entry; otherwise sentinel argv/envp are byte-exact/sorted and stdin is `/dev/null`. | |
| E12 | Given an admitted profile child with synthetic lifecycle, generation, lease/lock, gate, desktop-snapshot, cwd, `/dev/null`, and control descriptors, when its sentinel reaches the retained-descriptor `fexecve` boundary, then `/proc/self/fd` shows only fd 0, approved stdout/stderr, and explicitly authorized surviving assets; it contains no internal authority, and the executable/error-report descriptors have closed on successful image replacement. | |

## Failure modes

| Failure | Guard |
|---|---|
| A D-Bus entry falls back to `Exec` | E1 requires identical no-side-effect refusal. |
| A plain entry is mistaken for an existing owner | E2 requires no probe and fresh concurrent launch. |
| Shell or mutable pathname controls the child | E3/E4/E7/E10 require argv-only parsing, a retained descriptor, and a non-user-mutable executable policy. |
| A launched profile inherits internal control authority | The exact child fd map and E12 close/sweep every non-application descriptor before `fexecve`. |
| A new generation changes an admitted child | E5 and SPEC 0012 lease transfer retain N. |
| A launcher can bypass lifecycle proof | E8/E9 require the private consuming facade and existing M2 crash semantics. |

## Open questions

None in this admission contract.  Supporting scripts, user-owned executables,
or a broader desktop-entry grammar is a later explicit extension; it must not
weaken the native descriptor-pinned path.
