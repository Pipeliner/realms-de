# SPEC 0013 — Truthful desktop Exec launch

- **Status:** Draft — candidate for acceptance; implementation is blocked until
  acceptance and until Draft SPEC 0012's handoff and Draft SPEC 0006/#117's
  typed idempotency/recovery amendments are accepted
- **Milestone:** M1
- **Decisions:** [ADR 0017](../adr/0017-immutable-theme-activation-generations.md),
  [ADR 0018](../adr/0018-m1-desktop-launch-is-fresh-exec-only.md)
- **Issue:** [#133](https://github.com/Pipeliner/realms-de/issues/133)
- **Dependencies:** Accepted
  [SPEC 0011](0011-theme-activation-generations.md); Draft
  [SPEC 0012](0012-activation-launch-lifecycle.md) remains non-authoritative
  until separately accepted; ADR 0011 / Draft SPEC 0005 own mandatory session
  publication scheduled by #60; #117 coordinates the public surface;
  [#135](https://github.com/Pipeliner/realms-de/issues/135) supplies exact
  consumer records and evidence
- **Supersedes / Superseded by:** Narrows any M1 claim that fuzzel application
  mode or raw `Spawn(argv)` is a verified themed launch. A later D-Bus launch
  spec may add a separate admission class.

## Purpose

Turn one desktop-file identity into exactly one fresh child process whose argv,
working directory and child-only theme bindings all come from one immutable
desktop snapshot and one leased sealed generation. A user must never be told
that a launch is themed when Helm bypassed desktop admission, used a shell,
activated an existing process, mixed generations, or published global
environment state.

## Scope

**In:** the typed desktop-id request; deterministic desktop-file lookup; bounded
immutable capture; the supported main-entry and `Exec` grammar; unconditional
`DBusActivatable=true` refusal; executable and working-directory pinning;
generation-bound launch-profile selection; child-only environment/argv/cwd
application; the handoff to lifecycle ownership; and truthful fuzzel/raw-argv
classification.

**Out:**

- D-Bus activation, owner discovery, existing-owner activation/retheming,
  activation proxies, bus restart and activation teardown are later dedicated
  work. M1 neither calls nor probes an application bus name.
- Session-entry publication into the systemd user-manager and D-Bus activation
  environment is required by Accepted ADR 0011 and specified by Draft SPEC
  0005; [#60](https://github.com/Pipeliner/realms-de/issues/60) schedules that
  implementation and [#117](https://github.com/Pipeliner/realms-de/issues/117)
  coordinates integration. #132 owns the prior per-UID claim and ordering.
  Within the issue allocation, #133 is the only place a separately accepted
  compare-and-restore/cleanup amendment for values this session wrote may land;
  this candidate authorizes no such mutation or restoration and refuses
  ownership-unprovable launch modes. It cannot weaken the
  import list, timing, two independently verified destinations, portal guard or
  `doctor` evidence. Prior session state is not a side effect of one launch.
- File/URL-bearing requests, desktop actions, MIME dispatch and temporary-file
  ownership are outside M1. The only request here is an app-menu launch with no
  payload and the main `[Desktop Entry]` group.
- Exact Foot, Yazi, btop, Zsh/Starship, GTK3/GTK4, Qt5 and Qt6 argv,
  environment, config grammar, assets, package floors, parser evidence, shim
  classification and non-UTF-8 generation-path policy remain #135. Merely
  passing a Qt variable or qt6ct file is not evidence that Qt5 or Qt6 consumed
  the selected palette.
- Launch ownership, records, scope/process-group adoption, crash reconciliation,
  teardown and transferred-lease release remain Draft SPEC 0012. This spec
  fixes the admission and semantic handoff boundary but does not accept that
  draft or duplicate its crash matrix.
- Generation construction, validation, pointer selection, process leases and GC
  remain SPEC 0011.

## Behaviour

### 1. Typed request and admission boundary

The verified entrypoint accepts only:

```text
DesktopExecRequest {
  session_id: SessionId,
  request_id: RequestId,
  desktop_file_id: DesktopFileId,
}
```

`SessionId` is the exact 32-lowercase-hex session incarnation returned by the
candidate Hello amendment. `RequestId` is a caller-generated random 128-bit
lowercase hexadecimal id, scoped to that session and reused unchanged for every
retry of one user action. It has no argv, shell string, file/URL payload,
desktop action, environment
override or caller-selected executable. `DesktopFileId` is valid UTF-8, 1–255
bytes, ends in `.desktop`, and otherwise contains only ASCII letters, digits,
dot, underscore and hyphen. A slash, NUL, control byte, `.`/`..`, an empty
basename or any other byte is refused.

After syntactic id validation, Helm may perform one bounded read-only lifecycle
scan for an existing session/request mapping. An existing mapping takes the
section 8 recovery path and does not rerun admission. For an absent mapping the
request first performs **desktop admission preflight**. Until that preflight
returns an immutable `DesktopExecSnapshot`, Helm may perform only that mapping
read, bounded read-only desktop search/capture, read-only executable/directory
opens and bounded diagnostic output. In particular it must not:

- open, initialize, resolve or recover Helm's generation store or `current`;
- create a process or lifecycle lease, lifetime owner, exec gate, launch record,
  session record, systemd transient unit/scope, cgroup or process group;
- publish or mutate any process-global, systemd-user or D-Bus activation
  environment;
- contact, query, activate, signal, restart or otherwise address an application
  or application bus name; or
- execute profile, desktop-entry or target code.

A valid session claim created independently at session start is a prerequisite,
not launch-attributable state. A refused request neither creates nor removes it.
Every preflight failure has the same forbidden-side-effect witness above.

### 2. Deterministic lookup and immutable desktop snapshot

At request start Helm captures one immutable `DesktopSearchPath`: the absolute
`XDG_DATA_HOME`, followed by the absolute `XDG_DATA_DIRS` entries in listed
order. Unset or empty values use the XDG defaults `$HOME/.local/share` and
`/usr/local/share:/usr/share`; the required `$HOME` and every explicit path must
be valid UTF-8 and absolute. Empty explicit list components, relative paths,
NUL, more than 64 roots or a combined path value above 64 KiB are refused.
Byte-identical roots after lexical normalization retain only their first
occurrence. Helm does not
write these roots or the calling process's environment. A missing
root/`applications` directory contributes no candidate; an existing unsafe,
unreadable or uninspectable component encountered while resolving the requested
id refuses.

For each root, candidates are current files below its `applications/`
directory. Their desktop-file id is their normalized relative path with `/`
replaced by `-`, as Desktop Entry Specification 1.5 defines. Traversal is
descriptor-relative. Helm does not follow a symlink in a root, directory or
candidate, and accepts only a regular candidate. Within one root, two paths
which derive the requested id (for example `foo-bar.desktop` and
`foo/bar.desktop`) are an ambiguity and refuse the request. Across roots, the
first matching id wins. A winning `Hidden=true` entry masks lower-precedence
entries and is refused as absent. Lookup examines at most 4096 directory entries
and 64 directory levels across all roots for one request; exceeding either bound
refuses.

The winning file is opened once with no-follow semantics. Its canonical source
identity is the captured search-path index, normalized applications-relative
path, desktop-file id, absolute diagnostic location, opened device/inode, size,
and SHA-256 of the bytes. The path itself is not authority after open. The file
must be at most 1 MiB, a regular file, and readable as one complete byte vector.
Metadata is checked before and after the bounded read; device, inode, size,
mtime and ctime disagreement refuses. The held bytes must be UTF-8 without BOM,
NUL or CR; every line ends in LF, has at most 16 KiB, and there are at most 4096
lines. Helm never rereads the desktop file after capture, even if its pathname
is replaced.

The file contains exactly one `[Desktop Entry]` group. Group names may not
repeat; keys may not repeat within a group; malformed headers/keys, text before
the main group other than comments/blank lines, and an unterminated final line
are refused. Unknown well-formed keys and non-selected groups are retained only
as non-authoritative snapshot bytes. The launch parser uses only the main group
and refuses duplicate localized variants. It requires:

- `Type=Application` and a non-empty unlocalized `Name`;
- `Version` absent or exactly one of `1.0`, `1.1`, `1.2`, `1.3`, `1.4` or
  `1.5`. All are parsed with this one conservative M1 grammar; historical
  numeric booleans/comma lists remain refused. Any other version refuses;
- `Hidden` and `NoDisplay` absent or exactly `false`;
- Helm visibility under `OnlyShowIn`/`NotShowIn`, evaluated against the captured
  colon-separated `XDG_CURRENT_DESKTOP` tokens with Desktop Entry 1.5 list
  escaping and case-sensitive comparison. The captured value is valid UTF-8,
  at most 4096 bytes, and has at most 64 non-empty tokens, each 1–64 ASCII bytes
  matching `[A-Za-z0-9_-]+`; empty/malformed tokens refuse. In a Helm session
  `OnlyShowIn`, when present, must contain `helm`, and `NotShowIn` must not. The
  same token in both lists is invalid; and
- no requested action. The `Actions` key and action groups do not extend this
  launch surface.

The effective message locale is captured once with POSIX precedence: first
non-empty `LC_ALL`, else non-empty `LC_MESSAGES`, else non-empty `LANG`, else
`C`. Its bytes must be valid UTF-8 and either `C`, `POSIX`, or match
`[A-Za-z0-9-]{1,32}(_[A-Za-z0-9-]{1,32})?(\.[A-Za-z0-9-]{1,32})?(@[A-Za-z0-9-]{1,32})?`;
the whole is at most 128 bytes and malformed values refuse. Localized `Name`
selection strips `.ENCODING`
and uses Desktop Entry 1.5's exact
`lang_COUNTRY@MODIFIER -> lang_COUNTRY -> lang@MODIFIER -> lang -> unlocalized`
fallback, omitting forms whose source component is absent. The selected name,
unlocalized `Icon`, source identity and all values below are derived once from
the held bytes and copied into the snapshot.

### 3. Unconditional D-Bus refusal and fresh-only Exec

`DBusActivatable` is absent/`false` by default and, when present, must be exactly
the Desktop Entry boolean `true` or `false`. `true` is an unconditional refusal
immediately after structural/main-group validation and before `Exec`, `TryExec`,
`Path`, generation selection or launch-local lifecycle mutation. The preceding
read-only request-id lookup is the only permitted lifecycle access. A
valid-looking `Exec` does not
become a fallback. The result is identical whether a session bus is absent,
present, has no owner, gains/loses an owner concurrently, or already has an
owner.

Helm sends no `NameHasOwner`, `GetNameOwner`, `StartServiceByName`,
`org.freedesktop.Application` or other application-directed bus call. It does
not derive or probe a bus name. Thus a D-Bus owner cannot be signalled,
activated, restarted, replaced or rethemed by this path.

A plain `DBusActivatable=false`/absent entry has no owner concept in this
contract. Helm performs no `pgrep`, PID-file, executable-name,
`StartupWMClass`, Wayland app-id, window-title or bus-name probe. Every admitted
request is allowed to create a fresh process, including concurrent requests for
the same id. Helm makes no singleton, deduplication, existing-process or
race-free exclusion promise.

### 4. Exact supported Exec grammar

M1 implements a strict subset of Desktop Entry Specification 1.5, not a shell
language. `Exec` is required and its source value after `Exec=` contains only
ASCII bytes. Three representations are distinct: **source bytes** in the held
desktop file, **post-general-unescape bytes**, and **final argv bytes**.

The source lexer is contextual and deterministic; it does not first apply an
unrestricted `5c 5c` replacement. It has `outside-quote` and `inside-quote`
states and consumes one complete production at a time. Outside quotes, any
byte in `21..=7e` other than `22` or `5c` copies unchanged; `20` copies as a
delimiter; `22` is accepted only at an argument boundary,
copies, and enters `inside-quote`; and exactly `5c 73`, `5c 6e`, `5c 74`,
`5c 72`, or `5c 5c` maps to `20`, `0a`, `09`, `0d`, or `5c`.

Inside quotes, any byte in `20..=7e` other than `22` or `5c` copies unchanged;
raw `22` copies and closes the whole argument. A `5c` must
begin exactly one of these source productions: `5c 73`, `5c 6e`, `5c 74`,
`5c 72`, `5c 5c 5c 22`, `5c 5c 5c 5c`, `5c 5c 24`, or `5c 5c 60`.
They map respectively to `20`, `0a`, `09`, `0d`, `5c 22`, `5c 5c`, `5c 24`,
or `5c 60`. The source lexer matches within this state only; it may not consume
a shorter outside-state production and then reinterpret the following byte.
Any other source byte/escape, unmatched quote, or bytes following a closing
quote before delimiter/EOF refuse. This is the complete source language.

The hexadecimal columns below are authoritative; display text is explanatory.

| Meaning | Desktop-file source bytes (hex) | Post-general-unescape bytes (hex) | Final quoted argv bytes (hex) |
|---|---|---|---|
| Empty argument | `22 22` (`""`) | `22 22` (`""`) | empty byte string |
| Space in one argument | `22 61 20 62 22` (`"a b"`) | same | `61 20 62` (`a b`) |
| Literal quote | `22 5c 5c 5c 22 22` | `22 5c 22 22` | `22` (`"`) |
| Literal backslash | `22 5c 5c 5c 5c 22` | `22 5c 5c 22` | `5c` (`\\`) |
| Literal dollar | `22 5c 5c 24 22` | `22 5c 24 22` | `24` (`$`) |
| Literal backtick | `22 5c 5c 60 22` | `22 5c 60 22` | `60` (backtick) |
| Literal quoted semicolon | `22 3b 22` (`";"`) | same | `3b` (`;`) |
| General escaped space | `61 5c 73 62` (`a\\sb`) | `61 20 62` (`a b`) | two argv items `61` and `62` |
| Literal percent | `25 25` (`%%`) | same | `25` (`%`) after field expansion |
| Ambiguous shorter quote spelling | `22 5c 5c 22 22` | **not produced: source refusal** | refusal |
| Unknown/dangling escape | `22 5c 5c 61 22` or terminal `5c` | **not produced: source refusal** | refusal |

The shorter `22 5c 5c 22 22` spelling cannot decompose as `5c 5c` plus a
closing quote because `5c 5c` is not an inside-state production. Thus the four
accepted quote escapes shown above are the only sources which produce
post-pass `5c 22`, `5c 5c`, `5c 24` or `5c 60`. A decoded LF, CR or tab refuses
before tokenization.

The argv tokenizer then splits on one or more unquoted ASCII spaces. Double
quotes are the only quoting form and must enclose a whole argument. They are
removed. An unmatched quote, a quote in a partial token, a post-pass backslash
other than one of the four quote escapes inside quotes, empty argv or empty
executable refuses. `22 22` is one empty non-executable argument and is not
dropped. Single quotes never quote.

Reserved characters are space, tab, newline, double quote (`22`), single quote,
backslash, `>`, `<`, `~`, `|`, `&`, `;`, `$`, `*`, `?`, `#`, `(`, `)` and
backtick. Outside a whole double-quoted argument they refuse. Inside one,
double quote, backtick, dollar and backslash require their post-pass quote
escape; the other reserved characters are literal quoted content. Field-code
expansion occurs once after quote removal. Expanded content is never reparsed
for quotes, separators, field codes or shell syntax.

The complete M1 field-code matrix is:

| Form | Placement | Result |
|---|---|---|
| `%%` | anywhere in one unquoted token | one literal `%` in that token |
| `%f`, `%F`, `%u`, `%U` | at most one total, standalone unquoted token | zero arguments because this request has no file/URL payload |
| `%c` | at most once, standalone unquoted token | one argv item containing the held locale-selected `Name` |
| `%k` | anywhere | refusal; M1 exposes no mutable pathname as snapshot authority |
| `%i` | at most once, standalone unquoted token | `--icon` plus the held unlocalized non-empty `Icon`, or zero arguments when `Icon` is absent/empty |
| `%d`, `%D`, `%n`, `%N`, `%v`, `%m` | anywhere | refusal; M1 does not silently remove deprecated codes |
| any other `%` followed by any byte, including non-alpha/non-`%`, or a lone `%` | anywhere | refusal |

Every field code other than `%%` must be a complete standalone unquoted token.
Any field code inside quotes is refused. `%F`, `%U` and `%i` therefore cannot be
embedded, and M1 deliberately applies the same unambiguous rule to the other
codes. Expansion may produce multiple argv items only for `%i`; the file/URL
list forms produce none in this no-payload request. The post-expansion argv must
remain non-empty and contain no NUL.

Helm never invokes `/bin/sh -c`, `sh -c`, a user's shell, `eval`, or a shell
parser. It performs no variable/command substitution, word splitting after
parsing, tilde expansion, globbing, redirection, pipeline, backgrounding or
alias/function lookup. `$`, backtick, `*`, `?`, `;`, `|`, `&`, `<`, `>` and
parentheses which survive the required desktop quoting are literal bytes in one
argv element.

The executable token contains no `=`. It is either a simple ASCII name matching
`[A-Za-z0-9._+-]+` with no slash, or an absolute ASCII path at most 4096 bytes:
one leading `/`, non-empty components separated by one `/`, no trailing `/`,
and no `.` or `..` component. Any other relative or absolute spelling refuses.
For a simple name Helm uses the immutable session base `PATH` captured in
section 6. Every PATH component is non-empty, absolute UTF-8 with the same
normalized-component rule and at most 4096 bytes; there are at most 64
components and 64 KiB total.

Executable lookup permits ordinary filesystem symlinks, including the final
component, because installed `/usr/bin` and Nix-profile executables commonly
use them. It resolves the candidate once with Linux `openat2` anchored at `/`,
`RESOLVE_NO_MAGICLINKS`, and `O_RDONLY|O_CLOEXEC`. The resulting readable
descriptor is also the pinned executable handle; execute-only files which
cannot be opened read-only are an intentional M1 refusal.
Symlink loops, procfs-style magic links, resolution failure, non-regular final
objects, read failure or failed effective-credential execute permission refuse.
The symlink pathname is never used again: `fstat` captures device/inode/mode and
bounded `pread` reads the ELF identification/header from this same final open
file description. That descriptor is the executable identity transferred to
the supervisor; there is no `/proc/self/fd` reopen or second handle.
To keep the post-Exec witness discriminating, preflight also opens and `fstat`s
the launcher's kernel `/proc/self/exe` target with `O_PATH|O_CLOEXEC` and
refuses when its device/inode equals the target descriptor: before `execveat`
the fork child is still that launcher/supervisor image, so an identical target
could not be distinguished by an `/proc/<pid>/exe` identity transition. This
read-only evidence handle is closed during preflight and is never an execution
handle.

M1 admits only a Linux ELF image whose first four bytes are exactly
`7f 45 4c 46`, whose ELF identification names the host class, endianness and
current version, and whose file is at least the complete ELF header size.
`#!` scripts and every non-ELF/short image are refused during preflight. A
malformed or unsupported ELF detail which the kernel discovers only at exec is
a post-authorization exec failure, never permission to retry by pathname or
shell. The application child calls
`execveat(executable_fd, "", argv, envp, AT_EMPTY_PATH)` with the executable
descriptor carrying `FD_CLOEXEC`; successful ELF exec therefore leaks no plan or
executable descriptor into the image. It does not perform a second PATH,
symlink or pathname lookup. This is the complete M1 script/symlink policy.

`TryExec` absent means no additional check. A present empty value is malformed
and refuses. Otherwise its source crosses section 4's general-unescape pass,
must result in one ASCII desktop-entry string with no quote, field code,
backslash or whitespace, and uses the same simple-name/absolute-path grammar.
It is resolved once with the same symlink, regular-file and executable checks,
including the readable-ELF M1 rule; it is closed after preflight, is not
executed, and need not equal `argv[0]`.

`Path` absent/empty means the application child's fixed `/` working directory.
A non-empty `Path` crosses section 4's general-unescape pass and must result in
one absolute normalized ASCII path with no remaining backslash, using the same
4096-byte component grammar. Preflight opens it with
`O_RDONLY|O_DIRECTORY|O_CLOEXEC` without following symlinks, requires a directory
searchable by the effective credentials, and retains its descriptor; the gated
application child later uses that descriptor rather than reopening the path. A
relative or unavailable directory refuses.

`Terminal` absent/`false` is supported. `Terminal=true` is refused. Terminal
wrapping must never be shell concatenation; any future supported wrapper needs
an accepted exact #135 consumer profile amendment.

### 5. Generation-bound profile catalogue

Desktop preflight produces only immutable desktop facts and held executable/cwd
descriptors. It does not read `current` or a mutable consumer profile. After the
Draft SPEC 0012 lifetime owner exists, selection follows SPEC 0011 and accepts a
consumer binding only from the held generation.

A launchable generation contains an output named
`launch-profiles/catalogue-v1` whose bytes equal the raw `launch-profile`
preimage bound by SPEC 0011: its SHA-256 must equal both the manifest's
`launch-profile-sha256` and the output digest. Its canonical UTF-8 grammar is:

```text
helm-launch-profile-catalogue-v1\n
binding <desktop-file-id> <profile-id> <normalized-relative-profile-path> <lowercase-64-hex>\n
... one or more binding records, strictly sorted by desktop-file-id
```

The file is at most 64 KiB and has at most 1024 bindings, one LF per line and no
CR, blank, comment, duplicate or extra record. `desktop-file-id` uses section
1's grammar and is unique. `profile-id` is 1–64 lowercase ASCII bytes matching
`[a-z0-9][a-z0-9._-]*`. The profile path obeys SPEC 0011's normalized output
grammar, is below `launch-profiles/`, is not the catalogue, and is unique for
each distinct profile id. The digest is the exact profile output's manifest
digest. Repeated use of one profile id must repeat the same path and digest.

The held desktop id must have exactly one binding. Helm opens that profile only
below the retained generation descriptor, verifies it is a manifest-listed
regular output with the catalogued digest, and passes its held bytes plus the
held generation root and desktop snapshot to the #135 evaluator for that
profile id. A profile is at most 64 KiB, contains at most 1024 #135 records and
no individual record or value exceeds 16 KiB. Every #135 grammar must be a
single-pass bounded parser whose work is linear in these held bytes and whose
only filesystem inputs are already-opened manifest outputs. The current
built-in bytes
`helm-theme-launch-profile-v1\nnone\n` are not this catalogue and are truthfully
nonlaunchable. A missing binding/output, unknown profile id, malformed
catalogue/profile or any manifest/catalogue/profile digest disagreement refuses
without executing the target and follows the lifecycle failure path without a
lease leak.

#135 defines every profile record's exact bytes and resulting consumer argv and
environment. Its evaluator is pure over held inputs. It may return only an
immutable `ConsumerBinding` containing final argv items, child environment
overlay and generation-relative descriptor/path bindings. It cannot replace the
held executable, introduce a shell, add file/URL inputs, read `current`, read a
mutable active path or user config, or name a path outside the held generation.
Every argv change and environment name is authorized by that profile's accepted
#135 contract; unknown output is refused.

Only descriptor-relative copying plus manifest/digest validation of the bounded
catalogue/profile occurs while SPEC 0011's shared generation lock is held. Helm
first creates/fsyncs the process lease protecting that held generation, then
releases `activation.lock` and `lifecycle.lock`. Catalogue parsing, profile
parsing/evaluation, environment merge and plan encoding occur outside both
locks over copied bytes and held descriptors. Failure reacquires locks only for
Draft SPEC 0012's ordered terminal cleanup. No profile-controlled computation
serializes unrelated lifecycle work.

### 6. Child-only environment and prepared plan

After ADR 0011/SPEC 0005's real session-entry dual-import boundary scheduled by
#60, and before any concurrent server thread/request admission, the launcher
process freezes its original `environ`
array once as `BaseEnvironment`. It never calls `setenv`, `unsetenv`, `putenv`,
Rust process-global environment mutation or global manager/bus publication for
a profile launch. Each raw base entry is split at its first `=`. A name is
non-empty and contains neither `=` nor NUL; a value contains no NUL. Raw
non-UTF-8 base names/values are preserved. Duplicate names, more than 1024
entries, an entry longer than 16 KiB, or more than 64 KiB of base strings makes
verified launch admission unavailable. Entries are sorted by unsigned name
bytes. The one `PATH` value must additionally satisfy section 4; absent or
malformed PATH admits only absolute Exec/TryExec.

The accepted #135 binding supplies at most 256 overlay entries/64 KiB. Overlay
names are ASCII `[A-Z_][A-Z0-9_]*`, values are raw bytes without NUL, and names
are unique. It may not set `PATH`, `XDG_CONFIG_HOME`, `HOME`, `XDG_RUNTIME_DIR`,
`DBUS_SESSION_BUS_ADDRESS`, `WAYLAND_DISPLAY`, `DISPLAY`, `LD_PRELOAD` or
`LD_LIBRARY_PATH`; a later accepted amendment must change this denylist.
Denylisted values already present in `BaseEnvironment` remain inherited
byte-for-byte: the denylist prevents per-launch replacement, not inheritance.
The canonical merge replaces one byte-equal base name or inserts a new name,
then sorts the complete result by unsigned name bytes. It never emits
duplicates. Final envp has at most 1280 entries, 16 KiB per `name=value` string
and 96 KiB total.

Final argv has 1–256 entries, no NUL, at most 16 KiB per item and 64 KiB total.
At process start Helm reads positive Linux `_SC_ARG_MAX`; for each plan it
computes:

```text
exec-footprint = sum(argv byte lengths + one NUL each)
               + sum(env name + '=' + value + one NUL each)
               + (argc + envc + 2) * sizeof(char*)
```

It refuses before durable exec authorization unless
`exec-footprint + 32768 <= _SC_ARG_MAX`. The complete encoded plan in section 7
is at most 128 KiB. These limits cover base plus overlay and make predictable
`E2BIG` a pre-authorization refusal. A later kernel error is still an exec
failure, never retried.

No environment merge or plan creation occurs under either lifecycle or
generation lock. Helm does not invoke `systemctl --user import-environment`,
`dbus-update-activation-environment`, D-Bus `UpdateActivationEnvironment`, or
an equivalent for a profile launch. Those session-entry operations are owned
by ADR 0011/SPEC 0005, scheduled by #60 and ordered after #132's claim; they
are not a per-launch theming mechanism.

Release authority is represented by distinct consuming types:

- `PreparedSelection` owns immutable desktop/profile evidence, held
  executable/cwd/generation descriptors, final argv/envp, the SPEC 0011 process
  lease, its sole release capability and a `ClosedExecGate` which can only be
  closed, never sent. It has no gate-send/child-create capability. Dropping it closes
  that endpoint, performs pre-transfer terminal cleanup, and may release only
  that process lease.
- `TransferredExecAuthorization` is returned only by consuming
  `PreparedSelection` during atomic lifecycle-lease replacement. It owns the
  unique gate-send capability and lifecycle lease identity but no process-lease
  release capability. Dropping it closes the gate and cannot unlink either
  lease.
- `StartingExecPermit` exists only in the supervisor after a valid gate message
  and durable `starting` transition. It can create the application child exactly
  once and is neither serializable nor clonable.

All three types contain/use the spelling `catalogue-sha256`; no semantic field
is named `catalog`. After `PreparedSelection` is built Helm never rereads the
desktop file, `current`, a mutable active path, process-global environment,
user consumer configuration, catalogue or profile.

### 7. Lifecycle handoff and one exec

Typed desktop launch has a fail-closed platform gate before it opens admission
or performs desktop preflight. The shared SPEC 0007 control listener and every
non-typed-launch request remain available; this gate neither changes the
session claim to `admission-frozen` nor tears down helpers or raw `Spawn`.
While the gate is closed, a new `LaunchDesktop` returns
`DesktopLaunchRefused/pidfd-capability-unavailable` without a launch mapping,
while `GetDesktopLaunch` continues to report existing mappings.

On every service start/restart, under `lifecycle.lock`, Helm first validates
Draft SPEC 0012's `pidfd-health` and `pidfd-probe` records. A prior `failed`
health record with an affected launch cannot be superseded until that exact
ownership is terminal, empty and its lifecycle lease released. Any probe
record is reconciled only by its recorded systemd/direct containment identity;
reuse or uncertainty keeps typed admission closed. Only after all prior failed
ownership and probe containment are resolved does Helm generate a new service
incarnation, probe id and 256-bit failure token, publish/fsync
`pidfd-health/probing`, then publish/fsync the matching
`pidfd-probe/reserved` record before creating the inert probe supervisor. The
supervisor remains behind its PDEATHSIG barrier until that record is durably
updated to `contained` with its exact ownership identity.

Packaging declares Linux 5.4 or newer, and initialization still exercises the actual
seccomp/kernel surface in that disposable isolated probe process. The probe
normalizes/blocks `SIGCHLD`, creates a pipe-blocked disposable child with
`clone3(CLONE_PIDFD)`, first calls
`waitid(P_PIDFD, pidfd, ..., WEXITED|WNOHANG|WNOWAIT)` and requires success with
no exited child, then calls `pidfd_send_signal(pidfd, SIGKILL, NULL, 0)`, polls
the pidfd for at most one second, and reaps/validates `CLD_KILLED/SIGKILL` with
`waitid(P_PIDFD, ...)`. The child sets `PR_SET_PDEATHSIG=SIGKILL`, rechecks its
probe parent and otherwise waits on the pipe. The probe uses no numeric-PID
signal/wait fallback. If clone, either waitid operation, pidfd signalling,
polling or validation returns `ENOSYS`, `EINVAL`, `EPERM`, `EACCES`, timeout or
any other unexpected result, failure uses one serialized two-file order under
`lifecycle.lock`; it is not a multi-file atomic rename. `pidfd-health` is the
sole failure commit authority. First the supervisor authenticates and
replaces/fsyncs `pidfd-health/probing` with `failed/startup-probe`; only after
that root fsync does it replace/fsync `pidfd-probe/contained` with `failed`.
It then closes its barrier and exits. The external startup
reconciler owns only the recorded disposable process tree and must use its
exact systemd invocation/cgroup or direct group identity to terminate it and
prove total emptiness. A failed final `waitid(P_PIDFD)` therefore does
not claim that the probe reaped its own child or retry that child by numeric
PID; service death leaves both durable records for restart, and reuse or
failure of the emptiness proof leaves typed desktop admission closed. It
reports `FATAL UNSUPPORTED-PIDFD-LIFECYCLE` through the ordinary diagnostic
surface and structured typed refusal, but creates no typed-launch generation
selection, lease, lifecycle record or application child.

On probe success, the supervisor is normally reaped, exits, and the external
startup reconciler proves the recorded containment totally empty before
unlinking/fsyncing `pidfd-probe`. Only then may the ordered startup protocol
perform the separate CAS/fsync of `pidfd-health` from `probing` to
`healthy`. That exact healthy record
is the sole authority to open typed desktop admission. A live coordinator never
clears `failed`; only a later full restart, after affected ownership and probe
containment reconcile, may create a new probing incarnation and reach healthy.

Restart classifies every durable health/probe pair under `lifecycle.lock`:

| `pidfd-health` | `pidfd-probe` | Sole recovery authority |
|---|---|---|
| absent | absent | Initial state; no owner exists and a new service may no-replace-publish sequence-1 probing. |
| absent | any record | Not producer-reachable; fail typed admission closed and preserve the probe evidence. |
| `probing` | matching `reserved` or `contained` | No failure or success committed. Reconcile the barrier/containment to exact emptiness, remove/fsync the probe record, then run a wholly new probing incarnation; never promote the old probe to healthy. |
| `failed/startup-probe` | matching `contained` | Health committed before the probe-state update. Treat containment as failed, terminate/prove it empty, remove/fsync it, and retain failed health until a new full restart probe. |
| `failed/startup-probe` | matching `failed` | Normal committed failure; perform the same exact containment cleanup, then a later new full probe. |
| `probing` | absent | Crash either before reserved-record publication or after successful containment removal but before healthy. Because owner creation was forbidden before the reserved final, no unrecorded owner is possible in the first case; nevertheless never infer success. After the recorded coordinator is stale, replace with a new probing incarnation and run the full probe. |
| `failed/startup-probe` | absent | Cleanup committed and completed; retain failed until a later new full probe. |
| `failed/<actual-operation-phase>` | absent | Treat the named launch as the first commit witness, scan the complete bounded launch inventory, and reconcile every nonterminal typed launch present under the failed service state to terminal total emptiness and lease release. Any live/uncertain ownership blocks replacement; only then may a later restart replace failed with a new probing incarnation. |
| `healthy` | absent | The only admission-open pair. A restart still replaces the stale incarnation with probing and reruns the full probe before opening its own admission. |
| any other health state | `failed` beside probing, any probe beside healthy/actual-operation failed, a mismatched probe/incarnation, or any other pair | Not producer-reachable: fail typed admission closed and preserve all evidence for external repair. |

A crash after either rename but before its activation-root fsync recovers as
whichever old/new final is actually durable; its temporary is discarded under
Draft SPEC 0012 and that resulting row alone governs. No recovery guesses that
both file changes committed.

The passed capability is a service-incarnation invariant. `EINTR` is retried
for actual pidfd operations and `ESRCH` from `pidfd_send_signal` is resolved
only by polling and `waitid(P_PIDFD)` on that same pidfd. At an unexpected
actual-launch initial poll, timeout signal, result wait or final reap failure,
the supervisor first acquires `lifecycle.lock` and uses its inherited raw
service failure token in one idempotent admission-freeze handshake. If the
matching-incarnation health is `healthy`, it is the winner: it CASes to
`failed`, naming its exact launch, phase and supervisor identity, validates
them against the lifecycle record/lease, and root-fsyncs the transition. If
matching-incarnation health is already valid `failed`, it is a loser: without
overwriting the first affected-launch fields, it validates the committed
record's service/boot/coordinator incarnation and raw-token digest against its
inherited token. That durable same-incarnation failure is sufficient freeze
authority for every later supervisor. Until either the winner's CAS/root-fsync
or the loser's validation succeeds, the supervisor retains both
`lifecycle.lock` and lifetime-owner authority and does not exit. A different
incarnation, malformed record, token mismatch, `probing` or absent health is
not a loser success and remains fail-closed under that retained authority.
This winner-or-loser handshake prevents a second failure from deadlocking on
an impossible `failed -> failed` update.
The supervisor then preserves the last legal
durable tuple: `starting/none/yes/pending` before positive proof, or
`application-running/none/yes/positive` after it. It does not write a
nonterminal `lost`, `owner-drained` or `terminal` record. It closes every gate,
plan, acknowledgement and child-creation capability, retains the lifecycle
lease/record, relinquishes subreaper ownership and exits without an
`owner-drained` witness. It never signals, waits, releases or retries the child
by numeric PID.

Before every preparation reservation and every later publication, transfer,
authorization, arm and token boundary, the coordinator holds its admission
mutex plus `lifecycle.lock` and requires `pidfd-health/healthy` with its exact
current service incarnation. The health CAS and a reservation are therefore
totally ordered. A reservation that committed immediately before failure must
fail its next health check and cancel gate-closed; no request can authorize or
arm after the failed record is durable. Coordinator death during publication
does not weaken the freeze because the supervisor owns the lock, record CAS and
root fsync independently, and restart checks the record before admission.

On restart with actual-operation `failed` health, Helm scans the complete
bounded launch inventory before any failed-to-probing replacement. It
reconciles every nonterminal typed launch present under that failed service
state—not only `pidfd-health.affected-launch`—through its exact owner/lease row
to terminal total emptiness and lease release. The named affected launch is
the first commit witness, not an exhaustive affected set. Any later launch
which is live, reused, malformed or membership-uncertain blocks new probing and
preserves its tuple/lease. Thus cleaning the winner while a loser remains
unreconciled can never reopen typed admission.

Only after that supervisor identity is proven stale may the external
reconciler act. In systemd mode it uses the exact recorded invocation/cgroup;
in direct mode it uses the exact recorded process group and tracked-descendant
evidence. It applies the existing bounded whole-ownership TERM/KILL and total-
emptiness proof, then and only then writes the legal
`terminal/lost/exec-open yes/exec-evidence lost` bypass and releases the lease.
Any live member, reused identity or containment uncertainty preserves the last
nonterminal tuple and lease. Expected pidfd observations retain the narrower
normal route and its exact positive/negative result; only unexpected operation
failure takes this authority handoff. Thus a seccomp/policy change after
initialization degrades safely without fabricating state or stranding an
unleased application.

Before creating the lifetime supervisor Helm creates three distinct Linux
`AF_UNIX|SOCK_SEQPACKET|SOCK_CLOEXEC` socketpairs: a plan channel and a gate
channel plus a gate-receipt acknowledgement channel. It generates independent
random 128-bit plan and preparation nonces and a 256-bit gate token. The child
side inherits only its three endpoints, expected nonce/token and supervisor
state; every application descriptor is closed there until transfer.
The private endpoint plus credentials/nonce authenticates plan delivery. Gate
bytes can never be parsed as a plan.

Channel setup completes before the lifetime-supervisor fork and before any
sender can write. `SO_PASSCRED=1` is set on both supervisor receive endpoints
and on the coordinator's gate-receipt acknowledgement receive endpoint. The plan sender's
kernel-reported `SO_SNDBUF` must be at least `131072 + 32` bytes (Helm requests
a larger value if necessary) so Linux admits every 128-KiB seqpacket; failure to
set or verify that capacity refuses before owner creation. The plan receiver
allocates a 131072-byte data buffer and a zeroed ancillary buffer whose capacity
is exactly `CMSG_SPACE(3 * sizeof(int)) + CMSG_SPACE(sizeof(struct ucred))`;
the arm, gate and acknowledgement receptions use zeroed ancillary buffers of
exactly `CMSG_SPACE(sizeof(struct ucred))`. Every receive
uses `recvmsg(..., MSG_CMSG_CLOEXEC)`. An unavailable/rejected flag refuses the
platform. After plan receipt, `F_GETFD` must report `FD_CLOEXEC` on all three
received descriptors before metadata validation; absence on any descriptor is
a terminal protocol refusal. No received descriptor exists without the atomic
close-on-exec flag. `F_GETFL` must report `O_RDONLY` access mode for the
executable, and `fstat` must confirm cwd/generation are directories; the
supervisor repeats the bounded ELF-header `pread` on the received executable
and matches the preflight identity before accepting it.

Each channel has explicit unarmed/armed states. Before publishing a preparation
reservation or creating any channel, the lifecycle coordinator samples
`CLOCK_MONOTONIC` once and computes the absolute, overflow-checked
`preparation-deadline = sample + 10 seconds`. That exact absolute timestamp is
copied into the launcher task and inherited unchanged by the supervisor; no
participant resamples an epoch origin. `now >= preparation-deadline` is expired,
so expiry wins equality with an arm or state transition. The
`PreparationEpoch` ends only at valid authenticated gate-token receipt or
terminal cancellation. It bounds every plan/gate state through that boundary,
including
`AwaitPlanUnarmed`, `AwaitPlanArmed`, selection/evaluation, process-lease,
record publication, adoption, `AwaitGateUnarmed` and `AwaitGateArmed`; each
two-second armed deadline is an additional tighter per-packet limit. Expiry
closes all three channels, kills
and reaps the still-gate-closed supervisor where its exact identity is live,
and invokes the state/lease cleanup below; it never sends a gate. This overall
bound does not start or consume either armed deadline. Before every arm,
acknowledgement or token send/receive, preparation receipt/removal and every
`preparing`/adoption/transfer/`authorized` publication, the actor checks the
shared absolute deadline; an expired actor can only cancel.
The supervisor also exits gate-closed at that epoch even when the launcher
process remains live but its request worker is wedged.

The sole per-UID lifecycle coordinator owns the in-memory handle for each
durable Draft SPEC 0012 `preparations/<preparation-id>` record. Under its mutex
and `lifecycle.lock`, it first validates the bounded preparation inventory,
counts only `coordinator-owned` records against the 128-active bound, and
applies Draft SPEC 0012's separate 512-entry/2-MiB final-plus-temporary
namespace accounting. Only when the active count is below 128 and two
entry/8192-byte operation credits remain, and the exact current-incarnation
`pidfd-health` is `healthy`, does it publish/fsync a new `coordinator-owned`
record before channel or supervisor
creation. That record binds the absolute deadline, session/request/desktop,
coordinator incarnation, already-generated launch/lease ids and gate-token
digest. Concurrent callers therefore cannot observe the same vacancy, and a
reservation refusal creates no channel, supervisor, lease or launch record.
After supervisor creation the coordinator atomically updates/fsyncs the record
with that exact PID/start-time before creating the process lease.

A valid gate-arm receipt is a durable, acknowledged ownership handoff, not an
in-memory decrement. The supervisor checks `now < preparation-deadline`, the
exact arm credentials and gate-token digest, and the related durable
`authorized/exec-open yes` launch and matching lifecycle lease. Under
`lifecycle.lock` it compare-and-swaps the preparation record from
`coordinator-owned` to `supervisor-received`, increments its sequence and
fsyncs the preparations directory. Expiry/cancellation uses the same lock and
state predicate, so exactly one side wins; equality is expiry. Only after that
fsync may the supervisor send this exact credentialled acknowledgement on the
third channel:

```text
helm-gate-arm-accepted-v1\n
<preparation-id>\n
<gate-token-sha256>\n
```

The coordinator performs one bounded `recvmsg(..., MSG_CMSG_CLOEXEC)`, requires
the exact packet length/content, one matching kernel `SCM_CREDENTIALS`, no
rights, truncation, extra control data or bytes, and reopens under
`lifecycle.lock` the exact `supervisor-received` record to verify its reserved
ids and supervisor identity and requires `now < preparation-deadline`. It then
unlinks/fsyncs that preparation record and retires the counted reservation
exactly once; this durable receipt retirement is not the end of the
`PreparationEpoch`. Only after durable removal and a second
`now < preparation-deadline` check may it send the raw gate token. Thus
coordinator death before acknowledgement leaves
a counted `coordinator-owned` record, while death after the supervisor's CAS
leaves a non-counting `supervisor-received` cleanup receipt and never an
early-released slot. A missing, malformed or lost acknowledgement sends no
token. If the absolute deadline expires after removal but before token send,
the coordinator closes the gate and the absent-preparation `authorized` launch
uses Draft SPEC 0012's exact gate-closed `terminal/failed` recovery row.

The gate's two-second armed deadline begins at valid gate-arm receipt and
bounds the durable CAS, acknowledgement and token, rather than restarting for
either substep. At startup/restart, typed admission stays frozen while the
coordinator, under `lifecycle.lock`, validates every bounded preparation
record, its reserved identities and any related launch/lease/owner evidence.
It reconstructs the active count and namespace credits from
`coordinator-owned` records, namespace-accounted `supervisor-received`
receipts and every temporary/unsafe entry; safely retires valid received
receipts; and cancels expired records through Draft SPEC 0012's exact recovery
row. Malformed, uncertain, over-128-active, over-512-entry or over-2-MiB
inventory fails closed. The independent reconciler repeats this scan
at least once per second, including while no request worker is live. Only after
a complete valid startup scan may admission reopen. A later response from a
wedged task must revalidate the durable record, supervisor and deadline under
the coordinator mutex and `lifecycle.lock`; it cannot publish, transfer or arm
after expiry/reconciliation.

Only after the complete plan is encoded does the launcher send one
credentialled, descriptor-free arm packet
`helm-plan-arm-v1\n<plan-nonce>\n`. Valid arm receipt transitions to
`AwaitPlanArmed` and starts a two-second `CLOCK_MONOTONIC` deadline at that
receive boundary; the plan packet must arrive before it. Similarly, only after
the durable `authorized` transition does the launcher send
`helm-gate-arm-v1\n<gate-token-sha256>\n`; valid receipt enters
`AwaitGateArmed` and starts its independent two-second monotonic deadline. The
durable receipt, acknowledgement and token receive must complete before both
that armed deadline and the unchanged absolute preparation deadline. The
supervisor checks both immediately before accepting the token; a token received
at or after either deadline is refused and the gate remains closed. The arm digest is the lowercase 64-hex SHA-256
of the raw 32-byte token; the token packet contains exactly those raw bytes.
Pre-arm preparation consumes neither two-second armed deadline but remains
inside the ten-second `PreparationEpoch`; gate-arm receipt starts only the
tighter gate deadline and does not end the absolute epoch.
EOF before arm, EOF/timeout after arm, duplicate/out-of-order arm or data, or a
launcher crash closes all three channels and follows the state-appropriate cleanup.
Every arm/token receive requires exactly one matching kernel
`SCM_CREDENTIALS`, no `SCM_RIGHTS`, no truncation and its exact packet length;
the launcher identity check is the same PID/UID/GID/boot/start-time check as the
plan. A credential/control mismatch refuses without advancing the channel state.

Every packet send on any of the three channels uses
`sendmsg(..., MSG_NOSIGNAL)`; a kernel
without this safely scoped flag is unsupported. Peer close therefore returns a
send error to the sending actor rather than delivering `SIGPIPE` to the session
process. After selection/evaluation, the launcher sends exactly one plan packet
with one such `sendmsg` and `SCM_RIGHTS` descriptors in fixed order: executable, cwd, held
generation root. `SO_PASSCRED` is enabled; the supervisor requires one
`SCM_CREDENTIALS` whose kernel-supplied PID/UID/GID match the recorded launcher,
then verifies that live PID's boot id and `/proc/<pid>/stat` start time against
the pre-fork record before accepting the packet. The packet is at most 128 KiB
and has this envelope followed by exactly
`body-length` bytes:

```text
helm-exec-plan-v1\n
launch <launch-id>\n
nonce <lowercase-32-hex>\n
body-length <canonical-positive-decimal>\n
body-sha256 <lowercase-64-hex>\n
fd-count 3\n
\n
<body bytes>
```

The body has this exact order. `<n>:<bytes>` means canonical decimal byte
length with no leading zero except `0`, colon, exactly that many raw bytes, then
one framing LF. Length framing permits LF/non-UTF-8 environment values.

```text
helm-exec-plan-body-v1\n
session <session-id>\n
request <request-id>\n
desktop <n>:<desktop-file-id>\n
desktop-sha256 <lowercase-64-hex>\n
generation <generation-id>\n
manifest-sha256 <lowercase-64-hex>\n
catalogue-sha256 <lowercase-64-hex>\n
profile <n>:<profile-id>\n
profile-sha256 <lowercase-64-hex>\n
fd executable <device-u64> <inode-u64> <mode-u32>\n
fd cwd <device-u64> <inode-u64> <mode-u32>\n
fd generation <device-u64> <inode-u64> <mode-u32>\n
argc <canonical-positive-decimal>\n
arg <n>:<raw-argument-bytes>\n
... exactly argc arg records
envc <canonical-unsigned-decimal>\n
env <name-n>:<raw-name><value-n>:<raw-value>\n
... exactly envc env records, sorted by unsigned name bytes
```

The supervisor uses one 128-KiB plan `recvmsg` with `MSG_CMSG_CLOEXEC`.
`MSG_TRUNC`, `MSG_CTRUNC`, missing
or extra credentials/descriptors, wrong order/metadata/identity/nonce/launch
id, noncanonical lengths, digest mismatch, duplicate packet, extra byte or EOF
before one complete packet refuses. It `fstat`s every fd and matches body
identity. The launcher closes its plan endpoint immediately after the
all-or-error seqpacket send; the supervisor requires EOF after that one packet.
Failure closes every received fd and gate, starts no application and follows
pre-transfer cleanup.

The required cross-spec order is:

0. Complete section 1's read-only idempotency lookup and, only for a new key,
   sections 1–4 desktop preflight. Refusal creates no owner or launch-local
   generation/lifecycle/environment/bus side effect.
1. Durably reserve one preparation, create all three channels and exactly one
   inert lifetime supervisor, and fsync its exact identity into the reservation.
2. Under `lifecycle.lock` then shared `activation.lock`, resolve/validate
   `current`, copy/validate bounded catalogue/profile bytes, retain the opened
   generation and create/fsync the process lease for that supervisor. Release
   both locks.
3. Outside both locks, evaluate section 5, merge section 6, encode/validate the
   complete plan, form `PreparedSelection`, and deliver the authenticated packet
   plus three descriptors. Failure keeps the gate closed and cleans the process
   lease.
4. Under `lifecycle.lock`, rescan and atomically persist the unique
   session/request/launch mapping in `preparing`; a concurrent loser cleans its
   inert plan/owner/process lease. Adopt the winner into its verified
   scope/group, consume `PreparedSelection` to atomically replace/fsync the same
   lease, and produce `TransferredExecAuthorization`.
5. Durably record `authorized/exec-open yes`, send the gate arm packet, require
   the supervisor's durable-receipt acknowledgement, durably retire that
   receipt, and only then send exactly the 32-byte gate token as the only data
   packet on the distinct gate channel and close it. Wrong length/value,
   duplicate, truncation, lost acknowledgement, armed
   timeout or EOF starts no child. `authorized` is not application liveness.
6. The supervisor receives the token, durably transitions the exact record to
   `starting`, consumes `StartingExecPermit`, creates `pipe2(O_CLOEXEC)` and
   creates exactly one application child with the atomic `clone3(CLONE_PIDFD)`
   protocol below. The read end stays with the supervisor;
   the write end stays with the child until Exec or explicit failure. The
   supervisor remains alive as lifetime owner/subreaper and never replaces
   itself with the application. It is single-threaded. Before child creation it
   installs `SIGCHLD` disposition `SIG_DFL` with no `SA_NOCLDWAIT`, blocks
   `SIGCHLD`, and has no handler or other thread which can reap a child. It then
   uses one fork-like `clone3` call with exactly `CLONE_PIDFD`,
   `exit_signal=SIGCHLD`, and a parent pidfd result slot initialized to `-1`;
   `CLONE_THREAD`, `CLONE_PARENT`, `CLONE_DETACHED` and all VM/files/signal-
   handler sharing flags are absent. The kernel returns the numeric child PID
   and its pidfd atomically. `ENOSYS`, `EINVAL`, a missing pidfd, or any clone
   error is `failed` and has no `fork`/`pidfd_open` fallback. The supervisor
   thereafter signals only with `pidfd_send_signal` and observes/reaps only with
   `waitid(P_PIDFD, ...)`; it never kills or waits by numeric PID. The child
   resets every catchable signal disposition to `SIG_DFL` and installs an empty
   signal mask before descriptor/application setup. The parent closes its
   writer and the child closes its reader, so EOF has one possible writer.
7. Only the application child applies `fchdir(cwd_fd)`, closes cwd and
   generation descriptors, closes all inherited channels/lifecycle state and every
   unrelated descriptor, and calls the one ELF `execveat` with the explicit
   argv/envp. Only the `FD_CLOEXEC` executable descriptor and error-pipe writer
   remain at that call; both close atomically on success. On failure the child
   writes exactly eight bytes—`45 52 52 01`, then Linux errno as unsigned
   big-endian `u32` in `1..=4095`—with one blocking `PIPE_BUF`-atomic write.
   The child has no signal handler, retries `EINTR` before transfer, and only an
   exact eight-byte return permits `_exit(127)`. Any other non-`EPIPE` error retains the writer while the child waits
   to be killed by the supervisor's bounded timeout. If the supervisor/read end
   has already died, the child's reset default `SIGPIPE` disposition (or an
   `EPIPE` return before delivery) terminates it or makes it `_exit(127)`; that
   owner-loss path is `lost` and no live supervisor exists to misclassify its
   EOF. Thus a reporting failure cannot become a success witness. The supervisor
   starts a separate two-second monotonic
   error-pipe deadline when `clone3` returns. The supervisor polls the error pipe
   and pidfd together. Exactly one valid eight-byte frame followed by EOF is
   Exec failure. EOF after 1–7 bytes, more than eight bytes, bad
   magic/version/errno, timeout, read error or owner crash is `lost`/uncertain,
   never success. Zero-byte EOF is only a trigger for the following positive
   post-Exec proof, not success by itself: the pidfd must be non-readable in a
   zero-time poll; the supervisor opens the kernel `/proc/<child-pid>/exe`
   target with `O_PATH|O_CLOEXEC`, `fstat`s that open description and requires
   its device/inode to equal the pinned executable; and a second zero-time
   pidfd poll must still be non-readable, after which the procfs evidence
   description is closed. The child is the supervisor's
   unreaped `CLONE_PIDFD` child; the pidfd is the kill/wait authority and the
   numeric PID is used only to name that child's procfs directory before reap,
   so PID reuse cannot satisfy the check. Failure to open/read/identify the procfs
   target, target identity drift, or pidfd readability before, during or after
   the check is conservatively `lost`, even if Exec may actually have occurred.
   Only this EOF-plus-live-pidfd-plus-exact-executable proof permits durable
   `application-running`. A SIGKILL, OOM kill or other default-action death
   before `execveat` closes the pipe but also makes the proof fail and can never
   be reported as running. The supervisor closes its executable/cwd/generation
   copies after this proof or failure classification, but retains the pidfd as
   the sole child signal/wait authority through final
   `waitid(P_PIDFD, ...)` and closes it only after reap. Failure/exit never
   retries or executes a replacement
   and follows Draft SPEC 0012's mode-specific drain and lease order.

After the application result and absence of every other attributed member are
known, the supervisor fsyncs `owner-drained`, exits, and leaves the lifecycle
lease intact. An external reconciler alone proves the now-total cgroup/direct
group empty, records terminal and releases the lease. The supervisor's own
membership is never counted as group-empty evidence. Draft SPEC 0012's
state/result matrix is normative for this amendment: any normal exit code,
including nonzero, maps to `exited`; post-proof signal death and exact
pre-/Exec failure map to `failed`; ambiguous evidence maps to `lost`. An
external reconciler may bypass `owner-drained` only at the states/results that
matrix lists and never fabricates a live-supervisor result.

There is no state in which a live application is unleased. Generation N+1
becoming current changes none of the desktop bytes, executable, argv, profile,
environment or generation-relative paths prepared from N.

This section is a candidate boundary amendment, not acceptance of SPEC 0012.
Implementation is blocked until SPEC 0012 incorporates this protocol,
supervisor topology, consuming types and corrected A13, then is separately
Accepted.

### 8. Public typed request, picker and raw-argv truth

Acceptance of this spec requires a coordinated SPEC 0006/#117 interface
amendment adding this distinct request/reply relation:

```text
Response::Hello { version, session, session_id }

Request::LaunchDesktop(DesktopExecRequest {
  session_id, request_id, desktop_file_id
})

Request::GetDesktopLaunch { session_id, request_id }

Response::DesktopLaunchAccepted {
  session_id, request_id, launch_id, lifecycle_sequence,
  desktop_file_id, generation_id, profile_id,
  launch_mode: "fresh-exec",
  exec_evidence: "pending" | "positive" | "negative" | "lost",
  state: "authorized" | "starting" | "application-running" |
         "owner-drained" | "terminal",
  result: "none" | "exited" | "failed" | "lost"
}

Response::DesktopLaunchRefused {
  session_id, request_id, launch_id: none | <launch-id>,
  reason: "preflight-refused" | "session-mismatch" |
          "request-conflict" | "pre-authorization-failed" |
          "idempotency-capacity" | "pidfd-capability-unavailable" |
          "unknown-request" | "uncertain-state"
}

Response::DesktopLaunchCurrent {
  session_id, request_id, launch_id, lifecycle_sequence,
  desktop_file_id, generation_id, profile_id,
  state: "preparing" | "adopted" | "authorized" | "starting" |
         "application-running" | "owner-drained" | "terminal",
  disposition: "pending" | "accepted" | "refused",
  launch_mode: "fresh-exec",
  exec_evidence: "pending" | "positive" | "negative" | "lost",
  result: "none" | "exited" | "failed" | "lost",
  refusal_reason: "none" | "pre-authorization-failed"
}
```

The request uses section 1's id grammars. Under `lifecycle.lock`, Helm scans the
bounded registry before owner creation and again before publishing `preparing`.
An absent `(session_id, request_id)` may publish exactly one launch mapping; a
concurrent loser closes its inert owner/plan and releases only its process
lease. The same tuple plus same desktop id is an idempotent retry and never
reruns preflight, selection, gate send or child creation. The same tuple with a different
desktop id refuses. A non-current/closed session id refuses and never starts a
new-session launch.

Every identity and `lifecycle_sequence` in an accepted/current reply comes from
the validated durable record. `GetDesktopLaunch` for a mapping always returns
`DesktopLaunchCurrent`; an idempotent `LaunchDesktop` retry which finds an
existing mapping returns that same current DTO rather than waiting or rerunning
work. `launch_mode` names the selected admission contract and is always
`fresh-exec`; it is never execution-success evidence. `exec_evidence` is copied
from the durable record and becomes `positive` only after section 7's complete
post-Exec proof. Its legal combinations are total and exact:

| Durable evidence | `disposition` | `exec_evidence` | `result` | `refusal_reason` |
|---|---|---|---|---|
| `preparing` or `adopted` | `pending` | `pending` | `none` | `none` |
| `authorized` or `starting` | `accepted` | `pending` | `none` | `none` |
| `application-running` | `accepted` | `positive` | `none` | `none` |
| `owner-drained` or `terminal`, `exec-open yes` | `accepted` | exact durable `positive`, `negative` or `lost` | exact non-`none` durable result | `none` |
| `terminal/failed`, `exec-open no` | `refused` | `negative` | `failed` | `pre-authorization-failed` |

No other current DTO combination encodes. A malformed/identity-uncertain
mapping returns the non-authoritative `DesktopLaunchRefused/uncertain-state`
and preserves all evidence; it does not invent fields. An absent mapping returns
`DesktopLaunchRefused/unknown-request`.

The point query is scoped only by the authenticated connection's UID plus the
exact `session_id`/`request_id`; it exposes no scope, unit, process-group,
owner PID or ownership-kind field. Systemd-scope and direct-process-group
ownership remain internal lifecycle evidence. Therefore this same total DTO is
truthful in direct mode: its launch/desktop/generation/profile identities and
execution evidence come from the durable record, not from a public “scope”
abstraction.

Initial `DesktopLaunchAccepted` is returned only after durable
`authorized`; its state may already be later when read. Its `launch_mode`
means one fresh-Exec plan was authorized, while `exec_evidence: pending` means
child creation or Exec is not yet proven. Only section 7's positive
post-Exec proof permits `application-running`. A failure before durable mapping
returns `DesktopLaunchRefused`; after mapping, a pre-authorization terminal is
the exact refused `DesktopLaunchCurrent` row above. A durable terminal result
after authorization remains an accepted launch outcome with its real result.
A retained terminal record's `exec-open no|yes` decides refused-versus-accepted
recovery; uncertainty never guesses.

A preflight refusal creates no durable mapping, preserving the zero-side-effect
rule. A received refusal retires that request id at the client; a lost refusal
may be retried safely because no execution was authorized. `GetDesktopLaunch`
for an absent mapping returns `unknown-request`. Once a mapping exists, its key
is retained and all retries are stable through session close.

`pidfd-capability-unavailable` is valid only for a new typed launch with no
mapping while the durable health record is absent, `probing`, `failed`, from a
different service incarnation, or blocked on probe/affected-ownership
reconciliation. It disables only this typed relation: the ordinary SPEC 0007
listener, point queries, non-launch requests and raw `Spawn` remain available.

Disconnect after mutation but before reply is recovered by retrying the same
request id or `GetDesktopLaunch`; both return current durable state and never
create another owner. Terminal typed records remain through session close, so
reply ambiguity cannot outlive their idempotency evidence. M1 defines no
desktop-launch event or subscription. The complete public recovery surface is
the idempotent request plus `GetDesktopLaunch` point query. #117 may later
define a separate subscription topology, queue/coalescing rule and transport
tests compatible with SPEC 0007; until then no `Event` variant, delivery or
ordering promise exists.

Each request and reply is one SPEC 0007 frame at most 65,536 bytes
including LF; these fixed records are far below that bound and do not contain
argv/env/plan bytes. The connection retains at most one ordinary request/reply
as SPEC 0007 requires. Full-history pagination or replay beyond current
session-scoped query remains #117 work; it is not needed for idempotent recovery.

Helm, not fuzzel, discovers the bounded supported desktop catalogue and issues
opaque picker rows mapped in memory to `DesktopFileId`. Fuzzel may display and
return one row; it may not resolve a desktop file, return `Exec`, or execute its
application mode on the verified path. Helm rejects a returned row not in that
exact picker snapshot and then re-resolves the id through sections 1–4, so a
stale picker row does not become pathname authority.

Existing SPEC 0006 `run ARGV... -> Request::Spawn(Vec<String>) -> Ok|Error`
remains a separate direct/unverified admission class. `Ok` means only that raw
argv was accepted for asynchronous handling. It cannot yield
`DesktopLaunchAccepted`, `DesktopLaunchCurrent`, a generation/profile identity
or any verified/themed claim. Spells, arbitrary commands and shell commands
are separate future admission classes, never aliases for desktop `Exec`. The
currently implemented enum is truthful but insufficient for this Draft
contract; implementation is blocked until the SPEC 0006/#117 interface
amendment is Accepted and written red-first.

## Acceptance criteria

Tests are written red-first only after this spec and its required lifecycle
dependency are Accepted. Every refusal fixture instruments the complete
forbidden side-effect set: generation-store open/selection, lease directory,
lifecycle/session record, environment publication, owner/gate/scope/group,
target exec, application-directed bus traffic, signal and restart.

| # | Given / When / Then | Test |
|---|---|---|
| E1 | Given a supported entry, a canonical base containing raw non-UTF-8 and inherited denylisted values, and an admitted replacement/insertion overlay, when launched, then a real sentinel observes the exact sorted final envp, argv, cwd and generation-N bindings with no parent/systemd/D-Bus mutation or shell. Table variants with duplicate/invalid/oversized base or overlay, per-item/count/total overflow, or insufficient `_SC_ARG_MAX` headroom refuse before durable exec authorization. | |
| E2 | Given every hexadecimal row in section 4, including accepted triple-backslash quote and refused shorter `22 5c 5c 22 22`/unknown/dangling overlaps, plus a PATH trap named `sh`, when the contextual source lexer, tokenizer and field expansion run, then the fixture separately asserts exact post-general-unescape/final bytes or lexical refusal, proves the shorter inside-quote form cannot decompose through outside-state `5c 5c`, performs no second expansion and creates no shell/trap file. | |
| E3 | Given every field/quote/escape/TryExec/Path/Terminal case and executable fixtures for readable ELF, execute-only ELF, script, short/non-ELF, ordinary final symlink, magic link, loop and target identical to the supervisor image, when preflight runs, then readable ELF is opened once as the pinned `O_RDONLY` description and read by `pread`, execute-only and identical-image target intentionally refuse, the read-only `/proc/self/exe` evidence handle is never an execution handle, no `/proc/self/fd`/second target handle appears, and every case produces exact argv/cwd/descriptor or pre-side-effect refusal. | |
| E4 | Given a new `DBusActivatable=true` request and a tempting valid `Exec`, with no bus and with a private bus, when admitted, then after at most the read-only idempotency scan both refuse before Exec/TryExec/profile/generation or any launch-local lifecycle mutation, execute/create nothing, and send no bus message or owner query. | |
| E5 | Given a `DBusActivatable=true` entry and an independently instrumented candidate bus owner, when admitted, then Helm refuses solely from the held key, never derives or queries an application name, never addresses the owner, and its PID/name/state remain byte-identical with no signal, restart, activation or retheme. | |
| E6 | Given deterministic hooks where a D-Bus owner appears/disappears and where two plain-Exec requests overlap, when admission continues, then every D-Bus entry remains unconditionally refused and each valid plain-Exec request may start its own fresh process without any owner probe or race-free/singleton claim. | |
| E7 | Given table-driven roots covering XDG defaults/explicit UTF-8 absolute paths, precedence/collision/Hidden masking, unsafe components, lookup/line/depth limits, duplicate groups/keys/localized variants, supported/unsupported Version, empty TryExec, locale precedence/fallback/malformed locale, OnlyShowIn/NotShowIn and replacement during/after capture, when lookup parses one request, then each case returns the specified immutable snapshot or preflight refusal; replacement cannot mix bytes and a read counter proves no reread. | |
| E8 | Given generation N is selected/leased and N+1 becomes current before exec, when the sentinel starts, then every profile/env/config path and byte names N, and GC retains N until Draft SPEC 0012 proves lifecycle emptiness and releases it. | |
| E9 | Given absent/malformed current, bad manifest/seal/output digest, legacy `none` bytes, missing/unknown/mismatched catalogue/profile, every profile/count/size/work bound, and a deliberately slow profile, when launch runs concurrently with unrelated lifecycle work, then evaluation happens outside both locks, executes nothing on refusal, and leaks no process lease or lifecycle record. | |
| E10 | Given isolated startup probes with separately failed clone, initial/final P_PIDFD wait, pidfd signal, poll and validation; service death at every health/probe rename/fsync and owner/barrier boundary in systemd/direct modes; every health/probe pair; stale/live/reused creator/owner/unit/group/cgroup evidence; concurrent reservation versus actual healthy-to-failed CAS; two and many already-running supervisors failing before/during/after the first CAS; coordinator death before/during/after winner fsync and loser validation; the first named affected launch cleaned while later failing launches remain live, stale or uncertain; plus the plan/FD/credential/deadline/limit/error-pipe/pidfd/proc-exe cases above, when deterministic recovery runs, then health is the sole failure commit authority and each crash exposes one conservative pair. The first supervisor commits; every valid same-incarnation loser authenticates that durable freeze without overwriting the first witness, preserves its own tuple/lease, closes capabilities and exits without owner-drained; invalid losers retain lock/owner authority fail-closed. Reserved-before-owner plus PDEATH/barrier leaves no unrecorded runnable probe, and no recovery infers two-file commit or promotes old probing. New probing is forbidden until a complete bounded inventory scan reconciles every nonterminal typed launch under failed health to terminal emptiness/lease release; cleaning only the named winner while any later loser remains live/uncertain cannot reopen admission. Failed capability keeps the ordinary listener/non-launch/raw requests available, racing preparation cancels, only external total emptiness writes terminal/lost, all sends use `MSG_NOSIGNAL`, and only full post-Exec proof records positive/application-running. | |
| E11 | Given two concurrent valid launches and a current switch, when both execute and one app child creates descendants/exits, then each has one launch id, supervisor, transferred lease, immutable snapshot/profile/generation; each supervisor survives its child Exec as subreaper until attributed membership is proven empty; neither mixes bytes or creates a replacement application child. | |
| E12 | Given typed systemd-scope and direct-process-group launches with disconnect before reply at `preparing`, `adopted`, every accepted later state and refused/accepted terminal evidence; same-key retries/point queries, same-key/different-desktop, stale-session retry, every legal external bypass, direct fuzzel mode, forged picker, raw Spawn and spells, when invoked, then one connection-UID/session/request id maps to at most one launch/child creation, `GetDesktopLaunch` alone returns the total exact durable sequence/state/disposition/`launch_mode: fresh-exec`/execution-evidence/result/refusal DTO within one 65,536-byte frame without waiting, exposes no scope/owner/group abstraction, never treats admission mode as positive proof, encodes no desktop-launch event variant, and raw/direct success remains identity-free `Ok`. | |

## Budgets and limits

| Item | Limit |
|---|---|
| Desktop roots | 64; combined captured path value 64 KiB |
| Desktop lookup | 4096 entries and 64 directory levels per request |
| Desktop snapshot | 1 MiB, 4096 lines, 16 KiB per line |
| Desktop id / Path | 255 bytes / 4096 bytes |
| Launch-profile catalogue | 64 KiB and 1024 bindings |
| Child overlay | 256 variables and 64 KiB total |
| Launch profile | 64 KiB, 1024 records, 16 KiB per record/value |
| Base / final environment | 1024 / 1280 variables; 64 KiB / 96 KiB total; 16 KiB per entry |
| Final argv | 256 items, 64 KiB total, 16 KiB per item |
| Exec footprint | complete argv/env/pointer footprint plus 32 KiB at or below `_SC_ARG_MAX` |
| Plan transfer | one arm plus one plan packet; 128 KiB plan, three descriptors, verified `SO_SNDBUF >= 131104`, exact ancillary capacity, `MSG_NOSIGNAL`, two-second armed deadline |
| Preparation liveness | one absolute deadline sampled before atomic durable reservation, ten seconds through authenticated token receipt; expiry wins equality at ACK/removal/send/receive; independent armed deadlines are tighter and do not end it; 128 active `coordinator-owned` records, 512 total namespace credits and 2 MiB per UID; startup reconstruction plus scan at least once per second |

These are admission bounds, not input/frame budgets. No desktop lookup,
generation validation, lifecycle I/O or process wait may block the render/input
path. No fixed sleep is synchronization.

## Failure modes

| Failure | Guard |
|---|---|
| Alternate escape decomposition or shell interpretation changes argv | Contextual source language, held argv and E2/E3 |
| Pinned ELF cannot be read or an SCM descriptor leaks across Exec | One readable open description, `MSG_CMSG_CLOEXEC` plus `F_GETFD`, and E3/E10 |
| D-Bus entry falls back to Exec or probes an owner | Unconditional key refusal and E4–E6 |
| Desktop replacement changes an admitted launch | One held byte snapshot/descriptors and E7 |
| PATH/cwd/executable changes after preflight | Held executable/cwd descriptors and E2/E7 |
| Current/profile changes mix generations | SPEC 0011 selection plus catalogue binding and E8/E9/E11 |
| Lease transfer drops release authority twice | Consuming handoff plus SPEC 0012 amendment and E10 |
| Authorization/child-create/pre-Exec death is reported as an application running | Atomic `clone3(CLONE_PIDFD)`, exclusive pidfd signal/wait, error-pipe framing and exact `/proc/<pid>/exe` identity; EOF alone is insufficient, E10/E12 |
| Ambiguous reply retry starts a second fresh process | Session/request id mapping, retained terminal evidence, current-state query and E12 |
| Private launch variables leak globally | Explicit child vector/denylist and E1/E4 |
| Picker/raw argv bypass is reported as themed | Typed request/classification and E12 |
| Generic environment variables are mistaken for Qt evidence | Explicit #135 delegation and its real-consumer fixtures |

## Open questions

None inside this candidate's product boundary. Acceptance still requires the
narrow Draft SPEC 0012 and SPEC 0006 amendments and explicit owner acceptance
of this byte grammar. ADR 0011/SPEC 0005 retain session-publication authority;
#60 schedules it and #132 owns the claim/order prerequisite. #133 owns the
restoration/CAS and unsupported-owner seam, but this fresh-Exec candidate
authorizes no global mutation or restoration and keeps those modes refused. #135
may add exact fresh-consumer profiles only by satisfying this contract; it may
not broaden M1 to D-Bus activation, plain-Exec owner probing, shell execution or
global environment mutation.
