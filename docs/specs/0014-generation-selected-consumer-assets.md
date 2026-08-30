# SPEC 0014 — M1 generation-selected consumer assets

- **Status:** Draft
- **Milestone:** M1
- **Issue:** [#135](https://github.com/Pipeliner/realms-de/issues/135)
- **Depends on:** Accepted [SPEC 0011](0011-theme-activation-generations.md)
- **Decision:** Accepted [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md)
- **Merge dependency:** #133's Accepted fresh-Exec ADR must be present before
  this document can be accepted
- **Coordination only:** #132 and #133's proposed SPEC 0012/SPEC 0013 lifecycle
  and fresh-Exec contracts remain Draft and are not authority here

## Purpose and scope

Bind Helm-issued Foot, Yazi and btop launches, and a future human-selected
Zsh/Starship launch, to one immutable sealed theme generation without modifying
or silently importing ordinary user configuration. This specification defines
the #135 consumer asset and profile boundary. It does not cover every M1 theme
surface: GTK and direct fuzzel application execution are outside this contract,
and no fuzzel profile or verified-launch claim is defined here. Qt remains
human-gated below.

This Draft does not choose exact Yazi or btop payload bytes, Zsh startup
semantics, Qt integration, executable/package evidence, or user-shim trust. No
consumer in this Draft is implementation-ready until its listed human gates are
resolved and the resulting amendment is accepted.

## Published catalogue and individual profiles

A launchable generation candidate contains a manifest-listed regular output at
`launch-profiles/catalogue-v1`. Its complete bytes are the raw
`launch-profile` preimage whose SHA-256 is SPEC 0011's
`launch-profile-sha256`; that SHA-256 must also equal the catalogue output's
manifest digest. This preserves the bytes a launcher must evaluate instead of
leaving only an unrecoverable digest.

The catalogue is at most 64 KiB and contains at most 1024 bindings. It uses the
candidate grammar coordinated with Draft SPEC 0013:

```text
helm-launch-profile-catalogue-v1\n
binding <desktop-file-id> <profile-id> <normalized-relative-profile-path> <lowercase-64-hex>\n
... one or more binding records, strictly sorted by desktop-file-id
```

The document is canonical UTF-8 with exactly one LF per line and no CR, NUL,
blank, comment, duplicate, or extra record. `desktop-file-id` is valid UTF-8,
1–255 bytes, ends in the exact ASCII suffix `.desktop`, and otherwise contains
only ASCII letters, digits, dot, underscore and hyphen. The bytes before that
suffix are non-empty. A slash, NUL, control byte, `.` or `..`, an empty
basename, invalid UTF-8, a missing suffix, an overlong id or any other byte is
refused. Desktop-file ids are unique and binding records are sorted by their
unsigned UTF-8 bytes.

`profile-id` is 1–64 lowercase ASCII bytes matching
`[a-z0-9][a-z0-9._-]*`. The profile path satisfies SPEC 0011's normalized ASCII
output grammar, is below `launch-profiles/`, is not the catalogue path, and
identifies one manifest-listed regular output. Its digest is exactly that
profile output's manifest digest. Reuse of one profile id must repeat the same
path and digest; distinct profile ids must have distinct paths and distinct
digests. The selected desktop id has exactly one binding; a missing or
duplicate binding refuses.

The individual profile output is distinct from the catalogue. Its held bytes,
profile id, catalogue digest, manifest output record, SPEC 0011 manifest and
seal must all agree before evaluation. Catalogue/profile bytes are copied from
already-opened manifest outputs below the held generation. Neither document is
read through `current` or a mutable active path after selection. The legacy
`helm-theme-launch-profile-v1\nnone\n` input publishes no catalogue and
authorizes no generation-selected consumer.

This catalogue/output relation is a candidate reconciliation for SPEC 0011,
not a silent amendment to that Accepted specification. Acceptance of this
Draft requires the corresponding SPEC 0011 wording to be accepted in the same
change.

## Canonical individual profile

An individual profile is at most 64 KiB, has at most 1024 physical records, and
uses this canonical UTF-8 grammar:

```text
helm-consumer-launch-profile-v1\n
consumer <foot|yazi|btop|starship>\n
interaction generation-local-replacement\n
executable-evidence <evidence-id>\n
parser-evidence <evidence-id>\n
file-count <decimal>\n
file <binding-id> <normalized-generation-relative-output-path>\n
... exactly file-count file records, strictly sorted by binding-id
directory-count <decimal>\n
directory <binding-id> <normalized-generation-relative-directory-path>\n
... exactly directory-count directory records, strictly sorted by binding-id
argument-count <decimal>\n
argument inherited-argv0\n
argument literal <n>:<bytes>\n
argument file <binding-id> <prefix-n>:<prefix-bytes>\n
argument directory <binding-id> <prefix-n>:<prefix-bytes>\n
... exactly argument-count argument records, in final argv order
environment-count <decimal>\n
environment file <NAME> <binding-id>\n
environment directory <NAME> <binding-id>\n
... exactly environment-count environment records, sorted by NAME
```

Every structural record ends with exactly one LF. CR, NUL, blank lines,
comments, unknown records, duplicate fields, count mismatches and trailing
bytes are forbidden. Decimal fields are unsigned ASCII without sign or leading
zeroes, except zero is exactly `0`; they must fit the stated bound before
conversion or allocation. `<n>:<bytes>` has a canonical decimal byte length
and exactly that many UTF-8 bytes, with no NUL, CR or LF. A record or framed
value is at most 16 KiB. Length/count addition and expansion use checked
arithmetic; overflow or a limit excess refuses. Parsing is single-pass and
linear in the held bytes.

The combined file/directory reference count is at most 256. Argument and
environment counts are each at most 256. After expansion, an argv item or
environment name/value is at most 16 KiB, profile-contributed argv bytes total
at most 64 KiB, and overlay name/value bytes total at most 64 KiB. Prefix,
separator, root and relative-path bytes all count. These bounds are checked
before allocation or concatenation; the owning fresh-Exec contract may impose
stricter final argv/environment/`ARG_MAX` bounds.

`binding-id` and `evidence-id` match
`[a-z0-9][a-z0-9._-]{0,63}`. File/directory binding ids share one namespace and
are unique. Environment names match `[A-Z_][A-Z0-9_]*` and are unique. Profile
overlays remain subject to the owning fresh-Exec contract's count, item,
aggregate, inherited-environment and denylist limits. This grammar cannot unset
an inherited variable or set a literal environment value.

### Typed generation references

A `file` record names exactly one manifest-listed regular output. It is opened
descriptor-relatively with no symlink following and its bytes/digest are
validated under SPEC 0011.

A `directory` record is not an output record. It names a real no-follow
directory below the held generation and must be a non-empty proper prefix of at
least one file record in the same profile. The complete set of manifest outputs
strictly below that directory must equal the profile's file records strictly
below it: a missing descendant, an extra unbound descendant, an empty prefix,
or a file/directory prefix collision refuses. Every path component is opened
and retained descriptor-relatively; pathname replacement cannot redirect
evaluation outside the sealed tree.

Every file binding must be consumed by at least one argument, environment, or
declared directory closure required by the exhaustive consumer schema below.
Every directory binding must be consumed by at least one argument or
environment record. Conversely, every argument/environment binding must exist
with the named type. Unused, hidden, or indirectly named profile assets are
forbidden.

For an `argument file` or `argument directory`, the one resulting argv item is
the declared prefix followed by the held selected-generation path, one `/`,
and the bound relative path. An `environment file` or `environment directory`
produces the same path without a prefix. `argument inherited-argv0` copies only
the immutable desktop preflight's first argv item; it does not resolve or
replace the held executable. No profile can introduce a shell, executable,
file/URL payload, dynamic operand, extra environment name, or path outside the
held generation.

The selected generation's absolute filesystem spelling must be valid UTF-8,
contain no NUL, be absolute and normalized, and be at most 4096 bytes. A
non-UTF-8 generation/configuration root refuses
before profile evaluation can yield a binding. Relative profile paths remain
normalized ASCII matching SPEC 0011. Both argv and environment expansion use
the single captured absolute spelling associated with the held generation
descriptor; neither re-resolves `current`.

## Exhaustive consumer registry

The table is a closed M1 allowlist, not an example. A profile has exactly one
consumer and must have exactly the listed file bindings, directory bindings,
argument records and environment records in the stated order. Unknown
consumers, extra/missing records, a different binding id/path, duplicate
consumer selection, a changed literal/prefix/name, or a desktop base argv other
than the row's exact single item refuses. No row accepts payload operands.

This exhaustive registry is versioned declarative data in ADR 0007's permitted
generated/config seam, not hardcoded reused-tool policy in evaluator, drawing,
control or other logic code. Acceptance requires the ADR 0007 reconciliation
to name that generated/config location and extend its seam guard to the
registry. The table below fixes the semantic rows but does not invent the
registry asset's exact bytes or authorize implementation before that
reconciliation.

| Consumer / profile id | Exact files and directories | Exact final argv / overlay shape |
|---|---|---|
| `foot` | file `config=foot/foot.ini`; no directory | desktop base argv exactly `foot`; profile arguments exactly `inherited-argv0`, then file `config` with prefix `--config=`; no environment record |
| `yazi` | files `config=yazi/yazi.toml`, `keymap=yazi/keymap.toml`, `theme=yazi/theme.toml`; directory `config-dir=yazi` whose exact manifest closure is those three files | desktop base argv exactly `yazi`; profile argument exactly `inherited-argv0`; environment exactly `YAZI_CONFIG_HOME` from directory `config-dir` |
| `btop` | files `config=btop/btop.conf`, `theme=btop/themes/helm.theme`; directory `themes-dir=btop/themes` whose exact manifest closure is `theme` | desktop base argv exactly `btop`; arguments exactly `inherited-argv0`, literal `--config`, file `config` with empty prefix, literal `--themes-dir`, directory `themes-dir` with empty prefix; no environment record |
| `starship` | file `config=starship/starship.toml`; no directory | overlay-only candidate: zero arguments; environment exactly `STARSHIP_CONFIG` from file `config`; it cannot independently authorize a process and awaits the Zsh decision |

The profile id equals the lowercase consumer id in this candidate. A catalogue
may bind multiple desktop ids to the same profile id/path/digest, but one
desktop id cannot select multiple consumers and an individual profile cannot
contain multiple consumer records. Foot inherited/default config is not an
identity witness. Yazi's directory is admitted only with all three exact
files. btop's `btop.conf` must select the generation-local Helm theme, but its
exact bytes remain human-gated. `STARSHIP_CONFIG` does not establish that an
unresolved Zsh startup policy invokes Starship.

Qt5 and Qt6 have no grammar row and are unconditionally refused as unknown
consumers until separate exact rows and evidence are accepted. Qt6ct output,
an environment assertion, or Qt6 evidence reused for Qt5 cannot satisfy that
future gate. Fuzzel likewise has no row in this #135 contract.

## Evidence and truthful outcome

`executable-evidence` and `parser-evidence` are references, not self-attesting
claims. Syntactic validity or a producer-chosen label proves nothing. Before
any row can be accepted, a human-approved immutable registry must bind its
evidence ids to: the consumer; approved held executable locator and immutable
identity; supported package/version floor; exact parser/validator and expected
result; and the profile/asset grammar it covers. #134 retains package
provenance ownership. PATH lookup, a basename, or a profile digest alone cannot
prove executable identity. PATH shadowing, executable substitution, an unknown
id, stale version, wrong consumer or wrong parser evidence refuses.

No evidence registry or ids are approved by this Draft. Consequently the four
rows describe the only candidate shapes but none currently yields a verified
generation-selected launch. Their executable locators, identities, package
floors and parser evidence remain human gates; implementation must not turn
the table or an opaque label into a verified claim.

Pre-exec admission may validate only immutable registry/package/executable
identity and parser compatibility evidence that can exist before the child.
Black-box open/read observations necessarily occur after a probe starts. They
are required acceptance/packaging evidence, not fictional pre-spawn proof and
not a runtime state transition defined here. Any future production
post-spawn-verification state, failure action, or lease consequence belongs in
the accepted lifecycle/fresh-Exec contract.

Evaluation returns either one immutable candidate consumer binding or refusal.
On refusal the target is not executed and no process/lifecycle lease or record
survives or leaks. This is an outcome requirement only: Draft SPEC 0012/0013
own any temporary lease creation, ordering, transfer and cleanup needed to
protect a held generation from GC. This specification does not require refusal
to precede all temporary lifecycle state and does not run profile-controlled
parsing under a global lock.

Generation-local replacement applies only to a future admitted Helm-issued
launch. It performs no merge, import, append, overwrite or deletion of user
configuration. It neither reads corresponding ordinary user config as an
input nor globally mutates `XDG_CONFIG_HOME`, the launcher, session, systemd
manager, D-Bus activation or portal environment. Manual/direct launch, raw
`Spawn(argv)`, fuzzel application mode and unsupported shims are unverified and
receive no #135 generation-selected identity.

## Performance

The complete warm-cache `theme apply`, including the final accepted consumer
asset set, remains governed by SPEC 0002's existing **< 150 ms** benchmark
contract. This document adds no separate benchmark command, machine, sample or
percentile rule. Consumer parser/startup measurements are informational only;
there is no launch-latency or parser-probe timing SLO without measurement and a
later accepted decision.

## Fixture matrix

Tests are written red-first only after this Draft and its required dependencies
are accepted. Human-gated real-consumer rows cannot pass until their decisions
and evidence registry entries are accepted.

| # | Fixture | Required assertion |
|---|---|---|
| C1 | Catalogue/profile integrity | Missing catalogue/profile bytes, a catalogue not manifest-listed at `launch-profiles/catalogue-v1`, disagreement among raw preimage digest, catalogue output digest, profile path/digest and profile output digest, duplicate desktop binding, reuse of one profile id with changed path/digest, distinct profile ids sharing a path or digest, unknown profile id, or legacy `none` bytes refuses with no target execution and no surviving lease. Empty, 256-byte, invalid-UTF-8, missing-suffix, `.desktop` empty-basename, slash, NUL, control-byte, `.`/`..` and otherwise-disallowed desktop ids refuse; valid boundary ids sort by unsigned UTF-8 bytes. |
| C2 | Grammar bounds | Zero required catalogue bindings, count mismatch, integer/length overflow, overlong/truncated value, 64-KiB/1024-record/16-KiB boundary violation, sort violation, duplicate binding/environment id, unknown record/kind/consumer, trailing bytes, and non-linear parser work refuse; exact-limit fixtures parse without unchecked arithmetic. |
| C3 | Exhaustive registry | For every row, extra/missing file, directory, argument or environment records; duplicate consumer selection; wrong profile id; changed executable spelling; dynamic operand; file/URL payload; literal environment; forbidden extra overlay; or executable substitution/PATH shadow refuses. Every Qt and fuzzel profile refuses because no row exists. |
| C4 | Typed reference containment | A regular-file reference to a directory, directory reference to a file, empty/non-prefix directory, missing or extra directory descendant, prefix collision, symlink/special component, digest mismatch, or directory/file replacement race refuses or remains pinned to the held no-follow descriptors without external access. |
| C5 | N versus N+1 pinning | For each supported fake consumer, publish N+1 after selecting N and prove only profile-declared configuration paths/bytes come from N, corresponding N+1 and ordinary user-config sentinels are not read, and no user file changes. Executable, libraries, locale and unrelated runtime reads are outside this oracle. |
| C6 | Yazi (human-gated) | The approved parser accepts all three exact files; a probe observes exact `YAZI_CONFIG_HOME`, reads only those N configuration files, and negative variants that ignore the overlay or read default/user config fail. Directory closure hostile cases are also exercised. |
| C7 | btop (human-gated) | The approved parser accepts exact `btop.conf` and theme; a probe observes exact `--config`/`--themes-dir`, proves the config selects the N theme, and negative variants reading default/user config fail. |
| C8 | Foot (human-gated) | A probe observes exact two-item argv and opens the config below N; inherited/default config, wrong executable identity/version, unknown/stale parser evidence and PATH shadow cannot satisfy it. |
| C9 | Zsh/Starship and Qt (human-gated) | After policy acceptance, Zsh proves the selected startup semantics and `STARSHIP_CONFIG` use N without unchosen rc/extension reads. Separate real Qt5/Qt6 probes are required for each supported major; Qt6ct-only, environment-only, or cross-major evidence fails. Until then all such launch claims refuse. |
| C10 | Root/environment policy | Valid UTF-8 held roots expand byte-exactly into both argv and overlays; a non-UTF-8 root, unsafe/oversized path, overlay collision, forbidden inherited-environment replacement or global mutation attempt refuses with no target execution and no surviving lease. |
| C11 | Lifecycle outcome | Malformed catalogue/profile, parser/evidence rejection and exec-preflight failure execute no target and leave no surviving process/lifecycle lease or record, while the lifecycle fixture may prove its own temporary lease order and cleanup. The `DBusActivatable=true` zero-launch-side-effect witness remains owned by #133. |
| C12 | User config and shims | Sentinel user configs are neither read as profile inputs nor changed before/during/after candidate launches. Direct/manual launch, raw `Spawn(argv)`, fuzzel application mode and unknown shim remain unverified. Any supported shim row waits on its human-approved ownership/attestation model. |
| C13 | Performance | The final accepted asset set is added to SPEC 0002's authoritative warm-cache apply benchmark and remains below its existing 150-ms limit; consumer parser/startup measurements remain informational. |

## Needs a human

This Draft deliberately makes no default choice for any item below:

- **Executable and parser evidence for every row:** exact executable locator
  and immutable identity, package/version floors, evidence-id registry,
  parser/validator command and expected result, package-provenance handoff to
  #134, and whether post-spawn consumption has any production state at all.
- **Yazi:** exact `keymap.toml`, `yazi.toml` preview/thumbnail settings, aliases
  and commands.
- **btop:** exact `btop.conf` bytes, layouts, meters, update interval,
  process/sensor options, and whether a future constrained versioned extension
  exists. Arbitrary merge remains unsafe and unsupported.
- **Zsh:** whether Helm uses `zsh -d -f`, a controlled `ZDOTDIR`, or a defined
  extension/user-rc policy, including which user startup semantics remain.
- **Qt:** exact Qt5 and Qt6 configuration/plugin mechanisms, package/version
  floors, fresh-process consumption evidence, and whether Qt5 is supported now
  or explicitly deferred.
- **Shims:** whether any user-controlled wrapper/shim is supported and, if so,
  its exact ownership, descriptor preservation and attestation model.

Until these decisions are made, the corresponding profile bytes and generated
asset payloads are not implementation-ready. This specification does not
invent config contents, keybindings, layouts, shell semantics, Qt support,
executable trust or shim trust.

## Boundaries

This Draft adds no code or generated assets and does not authorize
implementation. Acceptance requires the human-gated choices for each selected
surface, failing fixtures, the #133 fresh-Exec ADR merge dependency, and the
corresponding accepted SPEC 0011/SPEC 0002/ADR 0007 reconciliations. It does not
define public desktop-launch or lifecycle APIs, change immutable-generation
lock/lease ordering, permit live reload, mutate or import user configuration,
promise direct-launch theme identity, cover fuzzel as a #135 consumer, or treat
Draft SPEC 0012/0013 as accepted authority.
