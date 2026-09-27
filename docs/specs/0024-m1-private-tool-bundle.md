# SPEC 0024 — M1 private Yazi and Starship tool bundle

- **Status:** Accepted (2026-08-31; amended 2026-09-13)
- **Milestone:** M1
- **Issue:** [#134](https://github.com/Pipeliner/realms-de/issues/134)
- **Refines:** [SPEC 0023](0023-m1-tool-source-intake.md)
- **Decisions:** [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md),
  [ADR 0010](../adr/0010-packaged-realm-sdd-git-runtime.md)
- **Evidence:** [M1 provenance research](../research/2026-08-30-m1-yazi-starship-provenance.md)

## Purpose

Make the SPEC 0023 A2 source-build route concrete for the native M1 packages.
The route supplies Realm-owned Yazi, `ya`, and Starship executables without
network access at package-build time, without taking ownership of distribution
executables, and without publishing any Realm infrastructure.

## Selected compatibility baseline

The first native source-bundle implementation selects these versions and the
existing native Rust floor:

| Tool | Selected version | Compiler baseline | Reason |
|---|---:|---:|---|
| Starship | `1.23.0` | Rust `1.85` | Its tagged manifest declares that floor and the current Realm Starship template was checked against that release. |
| Yazi / `ya` | `25.4.8` | Rust `1.85` | A locked Cargo check completed at that floor, subject to the configuration migration below. |

The currently retained `v1.26.0` and `v26.8.15` archives remain **intake
evidence**, not the selected native package route. Selecting these older
versions requires a reviewed SPEC 0023 intake update that adds their complete
offline closures and provenance records before a package build may consume
them. This specification neither deletes the current records nor treats a
top-level archive as an offline build input.

## Offline bundle contract

For each selected tool, the retained build input SHALL contain, and the
machine-readable intake record SHALL bind by digest:

1. the upstream tagged source archive and its provenance/notice record;
2. the exact `Cargo.lock` used for the build;
3. the complete `cargo vendor` dependency tree generated from that lockfile,
   retained either as regular Git-tracked files or as a deterministic,
   Git-tracked `.tar.zst` archive of that tree;
4. a Cargo source-replacement configuration which addresses every resolved
   Cargo source through retained input (registry, Git, and any applicable path
   source); and
5. a dependency license and notice report identifying every dependency in the
   resolved closure and the source used for its licensing information.

An archived vendor tree SHALL record its compression algorithm, archive digest,
and deterministic tar metadata (sorted names, epoch mtime, numeric owner/group).
The package build SHALL unpack it into an empty bundle-local directory before
Cargo runs; the linkage test SHALL reject a digest mismatch, path escape,
symlink, missing Cargo checksum, or unpacked tree differing from the archived
tree. Compression changes storage representation only: it must not weaken the
complete-closure, source-replacement, or offline-build requirements.

This rule applies equally to **every Cargo invocation performed by the native
package recipes**, including the Realm workspace build and test commands already
in `debian/rules` and `packaging/fedora/realm.spec`. The native source release
SHALL therefore retain a separately identified Realm-workspace `Cargo.lock`,
vendor tree, source-replacement configuration, and dependency license/notice
report, or the recipe SHALL not invoke Cargo for that workspace. It is not
permitted to prove only the bundled tools offline while allowing the package's
own Cargo commands to rely on a pre-populated cache or live registry.

### Realm workspace source authority

The Realm workspace closure SHALL bind to the immutable package source archive
at `packaging/tool-sources/bundles/realm-workspace/source.tar.gz`. That archive
is the source authority and SHALL be the exact archive unpacked for Realm's
Cargo build by both native package paths; no second source copy or build-time,
unbound `git archive` is permitted. A controlled intake MAY create this one
canonical archive from the record-bound repository commit, but it SHALL retain
the resulting bytes and digest as this source authority outside, and before,
either native package path. Debian/RPM preparation and build phases SHALL NOT
invoke `git`, `git archive`, archive creation, or any fetch operation, even if
the resulting bytes would match the recorded digest. Its tracked
`packaging/tool-sources/bundles/realm-workspace/bundle.toml` record SHALL bind
the archive SHA-256 to the repository commit, commit timestamp, and
provenance/notice record used at intake. The linkage fixture SHALL unpack the
source authority, verify the record's archive digest, and require its root
`Cargo.lock` to match the separately retained, digest-bound Realm-workspace
lockfile byte-for-byte.

The controlled intake SHALL exclude `packaging/tool-sources/bundles/` from the
source archive. Those independent retained authorities are inputs to outer
packaging orchestration, not Realm-workspace Cargo source or build/test inputs;
embedding them would recursively duplicate source and dependency closures. The
linkage validator SHALL reject a workspace source authority containing that
directory, and the provenance record SHALL name the exclusion. Any tracked
workspace source or Cargo build/test input changed after the recorded source
commit SHALL refresh the canonical archive from a new committed snapshot before
native package CI can satisfy this contract; passing tests against the moving
checkout do not make a stale retained archive current.

### CI-only Realm workspace rebinding

Producing or rebinding the Realm workspace source authority is packaging and
SHALL run only in the repository's CI. The rebind workflow accepts one exact,
full repository commit ID; checks out that object with complete history; and
creates the archive only from that immutable Git object with the exclusion
above. A branch name, tag, working-tree byte, local archive, or caller-supplied
provenance field is not an admissible source input.

The workflow SHALL refuse a commit whose `Cargo.lock` differs from the retained
Realm-workspace lockfile. Such a change requires a separate controlled
dependency-closure refresh; this path does not vendor dependencies. For an
unchanged lockfile, CI stages the existing digest-bound closure in runner-local
temporary storage, generates `source.tar.gz`, updates only the bound commit,
commit timestamp, source digest, and provenance digest fields, and runs the
normal bundle-linkage validator over the complete candidate bundle.

Successful CI publishes an artifact containing exactly the candidate
`source.tar.gz`, `bundle.toml`, and `provenance.md`. The workflow has read-only
repository permission and SHALL NOT commit or push the result. A maintainer may
place those exact three CI-produced files in the source change; the ordinary
freshness and linkage checks remain authoritative before merge. An uploaded
candidate alone is not package-build, install, or runtime evidence.

Ordinary CI source-kit verification and Realm package builds SHALL automatically
prepare a validated source binding from their checked-out HEAD before consuming
the retained archive. This includes pull-request merge commits. Preparation
uses the same CI-only producer and unchanged-lockfile restriction, then copies
only the three validated binding files into the job checkout. The vendor
closure remains unchanged. Preparation failure blocks the consumer. Each
consumer retains those three binding files as a CI artifact for provenance.
No follow-up repository commit or manual workflow dispatch is required for
ordinary source, documentation, policy, or workflow edits. The freshness check
continues to compare the entire retained input scope; it is not weakened.
Container jobs SHALL explicitly register their GitHub workspace as a trusted
Git directory before source preparation, so host/container UID differences do
not prevent the same immutable-commit checks from running.

The Realm-workspace source-replacement configuration is a separately retained
build input: native recipes SHALL stage it at the unpacked source root before
Cargo runs. The linkage fixture SHALL verify that configuration, the retained
vendor tree, every resolved dependency, and the dependency license report as
for selected tool bundles. Before each **Realm-workspace** Cargo invocation, the
Debian source package path and Fedora `%prep` path SHALL verify the same
retained archive digest and build only its unpacked source tree. The native
offline-build fixture SHALL substitute a same-name source archive with a
different digest in each path and require refusal before Cargo runs. A moving
checkout or unspecified local RPM `Source0` is not an immutable source
authority and SHALL NOT satisfy this contract.

The canonical source authority intentionally contains no Git worktree metadata.
Both native paths SHALL build the complete staged workspace, but their native
test invocation SHALL run all package-relevant staged workspace members and
exclude only the non-packaged `realm-agent-sdd` member: that member's 47 gate
fixtures validate live Git worktree state and remain mandatory in its existing
source-worktree test lane. Native package preparation SHALL NOT synthesize a
Git repository or weaken the no-Git rule merely to run those worktree fixtures.

The outer Debian and RPM source kits SHALL remain packaging inputs rather than
hidden workspaces at every nesting depth. Outside the canonical retained
`packaging/tool-sources/bundles/realm-workspace/source.tar.gz`, a kit SHALL NOT
contain a `Cargo.toml`, `crates/` or `.cargo/` workspace directory, or another
`source.tar.gz`; the separately retained Realm `Cargo.lock` is permitted only at
its one canonical bundle path. This recursive rule applies inside otherwise
allowed packaging-metadata directories as well as at kit top level.

The installed native-package build guide SHALL come from tracked outer
packaging metadata, not from the identity-bound archive's historical
`docs/INSTALL.md`. The emitted Debian and RPM packages SHALL document
the CI-only package build and verification workflow, without local source-kit
producer commands or packaging-toolchain installation instructions, and SHALL
NOT retain the superseded checkout
`ln -s packaging/debian` or moving `git archive ... HEAD` workflows. Updating
package guidance SHALL NOT mutate or regenerate the canonical source authority.
The CI clean-host prerequisite commands SHALL include the direct native recipe
requirements: Debian `pkg-config` and Fedora `make`. Because the package-native
test phase exercises SPEC 0004's raster/text-cache tests, both package recipes
and their CI prerequisite installs SHALL also supply a real DejaVu fallback
font: Debian/Ubuntu uses `fonts-dejavu-core`, while Fedora uses
`dejavu-sans-fonts` and `dejavu-sans-mono-fonts`. The build must not inherit
this test input accidentally from the CI host's font database.
The CI font guard SHALL check exact font-package tokens in the retained
RPM job's and native-fixture job's prerequisite install commands, not equality
of the entire command line. Additional prerequisites or package reordering
SHALL not fail this font contract; a missing font, similarly named package,
comment-only mention or fonts installed only in a
different job SHALL still fail. This prevents the stale full-line assertions
triggered by adding the required `patch` prerequisite without weakening explicit
font provisioning.

The Debian and Fedora package paths SHALL unpack only these retained inputs and
build using `cargo --frozen --offline --locked`. They MAY consume declared,
target-provided C toolchain/system dependencies, but SHALL NOT acquire an
upstream source, registry package, Git dependency, or release artifact over the
network. The retained source-replacement configuration SHALL remain visible to
Cargo subprocesses launched by package-relevant tests, including generated
trybuild projects outside the staged source tree: native recipes SHALL expose
the staged `.cargo` directory as inherited `CARGO_HOME`, not rely only on
top-level ancestor probing. Compile-fail checks in that package test path SHALL
prove the intended API boundary from the compiler error code and inaccessible
symbol, without requiring diagnostic prose or note layout to be identical
across supported Rust versions. A test fixture SHALL run the actual
`debian/rules` build path and the Fedora RPM build phase with networking
disabled and empty Cargo registry/Git caches, reject a
closure/configuration/lockfile mismatch, and fail when an adversarial
injected-fetch attempt is present in either recipe.

The selected Starship 1.23.0 source invokes `shadow-rs` from its build script.
Before native compilation, the selected-tool stager SHALL apply one
Starship-specific, record-bound patch which extends `shadow-rs`'s upstream
default deny set with `CARGO_TREE`. The bundle record SHALL bind the patch
digest and the exact SHA-256 of `build.rs` before and after application. The
stager SHALL first validate and unpack the original retained source archive
unchanged, refuse a missing, symlinked, digest-mismatched, or path-escaping
patch, refuse a preimage mismatch, apply the patch exactly once, and refuse a
postimage mismatch. Staging repeatedly into the same destination SHALL replace
the old stage with a fully materialized copy from the original archive and
produce the same patched `build.rs`; it SHALL NOT apply the patch cumulatively
or mutate the retained archive. A failed replacement SHALL restore the prior
complete stage. Staging happens before compilation and makes no concurrent
consumer or continuously-present-path guarantee. Native package build metadata
and every CI lane which invokes the stager SHALL declare the `patch` utility
rather than inherit it accidentally from the host.

With `CARGO_TREE` denied, the pinned `shadow-rs` build still makes one
unconditional `cargo -V` metadata query. The native fixture MAY classify only
that exact invocation from the selected Starship source and Cargo
home as build-script metadata. It SHALL execute the supplied real Cargo
unchanged, retain the validated source-replacement configuration in the now
populated Starship Cargo home, and require the query to succeed. This metadata
query is not a recipe build/test invocation and does not weaken the requirement
that the three authoritative builds and one workspace test use
`--frozen --offline --locked`. `cargo tree`, any other nested Cargo command, a
fifth authoritative build/test within either full package case, or Cargo from
another source/home remains a failure. The separately recorded same-recipe
Yazi reproducibility case SHALL execute exactly one additional offline Yazi
build from its fresh selected source/home, with the same denied Git probes.

Pinned `shadow-rs` also attempts local VCS metadata commands even though the
retained source archive contains no Git repository. During the selected
Starship stage, the fixture SHALL put a denying sentinel ahead of Git, require
the exact read-only metadata attempts made by pinned `shadow-rs`, and prove
that no real Git executable ran. The allowlisted attempts are limited to
`status`, `rev-parse`, `log`, `show`, `tag`, `describe`, and `symbolic-ref`
queries. Any acquisition or mutation command, including `fetch`, `clone`,
`pull`, `checkout`, `reset`, `clean`, `init`, `add`, `commit`, or `tag` with a
mutation argument, remains a failure. Network access remains disabled around
the entire package path. This compositional fixture exception records and
denies an upstream metadata attempt; it does not permit either native recipe to
invoke Git or weaken the source archive's deterministic authority.

The selected Yazi build additionally attempts exactly `git --version` from
its `yazi-boot`, `yazi-dds`, and `yazi-cli` source directories. These three
probes SHALL likewise be classified separately and denied without executing
Git. Other arguments or directories remain forbidden.

The fixture's disposable build tree SHALL be on a Linux filesystem that
supports `O_TMPFILE` with file `fsync`, atomic `renameat2` publication/exchange,
and directory `fsync`, as required by the retained lifecycle tests. CI SHALL
select that tree explicitly and preflight those operations before invoking
either native package driver; it SHALL NOT skip or weaken the lifecycle tests
when the selected filesystem lacks any capability.

The fixture SHALL invoke its supplied real Cargo and rustc executables, not a
stand-in. It SHALL bind Cargo to that supplied rustc and clear inherited
compiler-wrapper selection. Transparent instrumentation may record a Cargo
invocation only when it execs the supplied real Cargo unchanged. When CI
elevates that fixture, it SHALL first stage a complete Rust toolchain in a
directory readable, traversable, and executable by the elevated process, and
preflight-execute that staged Cargo and rustc. Staging may copy the real
toolchain solely to make it accessible; it SHALL NOT replace either executable
with a shim. The CI step that invokes the network-isolated native fixture SHALL
have a bounded job-step timeout and SHALL fail closed when the fixture exceeds
it; an indefinitely hung isolation check is not verification evidence.

Native fixture diagnostics SHALL survive its scratch-directory cleanup. CI
supplies a separate evidence directory for each case's driver output and command
sentinel log, and attempts upload even on failure or timeout. Only these text
logs are uploaded, not extracted package trees or source bundles. Each case
announces its start in the live job log. Missing logs after a forced runner loss
remain missing evidence, not a passing fixture. The timeout stays bounded;
retaining diagnostics does not waive it or prove why the build exceeded it.
The combined Debian/RPM cold-build fixture receives a 120-minute step budget
inside a 135-minute job budget. This covers two real release builds and their
tests; the earlier 15-minute cap expired while compiling the first workspace
(CI run 36317915230), before either package driver could finish. This budget is
an execution limit, not acceptance evidence: both drivers must still complete
all assertions, and a timeout remains a failure.
The evidence directory SHALL be under the fixture's dedicated `/tmp` parent,
outside its disposable scratch child, rather than beneath runner-owned home
directories. Before invoking native package drivers, CI SHALL prove file creation and append
from the same root-mapped network namespace used by the driver. Host-root write
access alone is insufficient: capabilities in a child user namespace do not
grant access through host-owned private ancestor directories. A failed probe
SHALL report ancestor permissions and stop before the expensive package work.

All fixture support files executed or read inside that namespace SHALL likewise
be regular copies beneath the accessible fixture scratch directory before
either package driver starts. In particular, command sentinels SHALL NOT
symlink back into runner-owned checkout ancestors, and the same-recipe Yazi
helper SHALL execute from the staged support copy. Each denying command keeps
its command basename so metadata classification is unchanged. A namespace
preflight SHALL read/parse every staged helper, execute every denying command
and require its expected denial log/status, and execute the supplied Cargo and
rustc version checks. An inaccessible support path SHALL stop before package
work, never silently fall through to a real network/VCS executable. A lightweight
regression SHALL remove access to the source ancestor after staging and still
execute the staged sentinels and helper without producing packages.

The Debian recipe's production resolver SHALL continue to select its complete
versioned Cargo/rustc pair below `/usr/lib/rust-1.[89][0-9]/bin`.  The native
offline fixture MAY set an explicit resolver-root input that contains the same
versioned layout.  Rules SHALL combine that root with the resolver's logical
selected path before invoking Cargo or rustc, so the fixture executes its
staged supplied pair rather than falling back to inherited `PATH`.  The fixture
root shall provide the supplied real rustc and either the supplied real Cargo
or transparent Cargo instrumentation which immediately execs that supplied
Cargo unchanged.  That input exists only to make the real nested Debhelper
invocation reproducible in an isolated fixture; it SHALL be consumed by the
resolver itself, not used to override its selected binary directory or bypass
the completeness check.
Fixture helper functions SHALL preserve the repository-root authority while
constructing per-case toolchain directories, so repeated cases and the
reproducibility helper continue resolving the same checked-out scripts.

Starship version verification SHALL require a successful command exit and the exact selected version on the
first output line and retain the complete output as evidence. Its additional
build-metadata lines are not part of the version identity; they SHALL NOT make
the selected version fail comparison. Empty output, a different version, or a
version embedded after another first line SHALL fail.

The Yazi build SHALL set deterministic source-date and VCS metadata. The intake
record SHALL bind the selected tag to its upstream commit SHA and commit
timestamp; the package recipe SHALL set the exact version/metadata inputs
accepted by the selected Yazi `vergen` build from those retained values. It
SHALL not accept `vergen`'s fallback metadata as release evidence or assume that
`SOURCE_DATE_EPOCH` alone controls it. The reproducibility fixture SHALL build
in two different directories/times and either compare the normalized declared
artifacts or explicitly record and justify every remaining non-identical field.
The declared comparison artifacts are the pre-packaging `yazi` and `ya`
executables from two clean builds using the same Debian Yazi recipe and supplied
toolchain. The second build uses a fresh source kit and target directory. The
fixture retains UTC execution times, paths and SHA-256 hashes and fails on any
byte difference; no normalization is currently declared.
Direct invocation of the shared Debian target SHALL export the same effective
dpkg build-flag defaults as Debhelper: CFLAGS, CPPFLAGS, CXXFLAGS, LDFLAGS and
RUSTFLAGS, while preserving explicit caller overrides. Run 36331107468 showed
that restoring only CFLAGS left the other four empty in the repeat, including
the missing Rust frame-pointer option. Such unequal inputs are not a valid
same-recipe comparison; byte equality remains required after correcting them.
The shared Debian Yazi recipe SHALL append Rust source-path remapping from the
source-kit root to `/usr/src/realm-0.1.0`, covering staged sources and vendored
dependencies. Run 36337157087 retained different absolute vendor paths in both
executables despite equal Rust flags: C/C++ prefix maps do not remap Rust
source locations. Preserve caller Rust flags and Cargo's encoded-flags
precedence (including an explicitly empty encoded value), appending the map
to the effective flag channel. This is compile-time path mapping, not binary
normalization; the strict byte comparison remains unchanged.
The CI diagnostic artifact SHALL include the explicit `yazi-reproducibility.txt`
report as well as driver `.out` and sentinel `.log` files. On mismatch the helper
SHALL print at most the last 40 report lines into the driver log, preserving
the compared artifact hashes and first byte differences even if upload fails.
Transparent Cargo instrumentation SHALL record effective `CFLAGS`, `CPPFLAGS`,
`CXXFLAGS`, `LDFLAGS`, `RUSTFLAGS`, and `CARGO_ENCODED_RUSTFLAGS` for both builds
without changing them. CI run 36324975517 completed both builds but lost the
comparison report and did not record all flags; this proves a mismatch, not its
cause. Do not retain binary artifacts or weaken byte equality to diagnose it.
On a remaining mismatch after equivalent build flags, retain bounded textual
ELF headers, section tables and notes for each compared file, plus embedded
strings containing either build root. Missing diagnostic tools or malformed
ELF input must be reported without replacing the original mismatch outcome.
Include textual section-content hashes and selected rustc/C compiler/linker
versions. The Cargo invocation recorder also retains only the allowlisted
SOURCE_DATE_EPOCH and VERGEN git SHA/commit-date/build-date values actually
passed to each build; no arbitrary environment dump is permitted.
The byte offset alone is not evidence of a particular ELF field or root cause.

Debian-versus-RPM
installed-byte equality is not this comparison: their C flags and packaging
strip operations differ. Both native installed-version checks remain required.

For the selected `25.4.8` bundle, both native recipes SHALL set
`SOURCE_DATE_EPOCH=1744112829`,
`VERGEN_GIT_SHA=99ea3b74c4260a724b43af812df0f68ef59395b7`,
`VERGEN_GIT_COMMIT_DATE=2025-04-08`, and
`VERGEN_BUILD_DATE=2025-04-08`. These values are the Unix epoch, commit, and
UTC date bound by that bundle's intake record. The native Yazi build SHALL
append `-std=gnu17` to the package builder's inherited `CFLAGS`. The
retained `onig_sys 69.8.1` source uses pre-C23 empty-parameter callback
declarations; selecting GNU C17 preserves their intended unspecified-argument
meaning on GCC 16 and newer without suppressing incompatible-type diagnostics.
This compatibility selection applies only to the selected Yazi build, not the
Realm workspace or Starship builds.

After retained-bundle verification and unpacking, Fedora preparation SHALL
remove executable permission bits from staged regular Rust source (`*.rs`)
files before compilation and debug-source collection. Source bytes and retained
archives remain unchanged; executable scripts and binaries keep their modes.
Rust inner attributes are not script shebangs. RPM's normal shebang processing
and debug-source generation remain enabled. A fixture SHALL verify those mode
and content boundaries, and the native CI RPM build verifies integration.

The retained Yazi runtime fixture SHALL set both the process working directory
and `PWD` to its controlled directory. Yazi 25.4.8 prefers absolute `PWD` over
the operating-system directory; an inherited package-build `PWD` must not make
the fixture inspect the package source tree instead of its sample file.

The Yazi build command SHALL select the `yazi-fm` and `yazi-cli` packages,
which produce `yazi` and `ya`;
the Starship build command SHALL select the `starship` binary. Both use their
selected bundle's staged source, vendor tree, source-replacement configuration,
and a bundle-local target directory with `--release --frozen --offline
--locked`. Default features are retained; a feature-set change requires a
specification amendment because it changes the supported executables.

The outer native source kits SHALL carry exactly the Realm-workspace,
`yazi-25.4.8`, and `starship-1.23.0` bundle authorities plus the narrow staging
helpers needed to validate and materialize those inputs. They SHALL NOT contain
a recursive copy of any bundle, a second source snapshot, or another hidden
workspace. Before Cargo runs, the selected-tool stager SHALL validate the
record-bound source archive, lockfile, vendor archive, source replacement, and
license report, then materialize only that selected bundle into an empty
package-local stage. The target mapping SHALL identify `retained:yazi@25.4.8`
and `retained:starship@1.23.0` for Debian and Fedora; Nix continues to use its
locked nixpkgs inputs.

## Executable ownership and session scope

Native packages SHALL install only these Realm-owned executables:

```text
/usr/lib/realm/bin/yazi
/usr/lib/realm/bin/ya
/usr/lib/realm/bin/starship
```

They SHALL NOT install or replace `/usr/bin/yazi`, `/usr/bin/ya`, or
`/usr/bin/starship`. Distribution-provided tools remain independently owned and
may coexist.

For the direct-launch path (without a systemd user manager),
`packaging/session/realm-session` SHALL prepend `/usr/lib/realm/bin` to the Realm
session process environment and its descendants. For the normal systemd-user
path, the Realm-owned service units SHALL receive an explicit private PATH by a
Realm-owned unit-level environment mechanism; `realm-wm`, `realm-bar`, and the
applications they launch therefore inherit it without changing the user
manager's global environment. Neither route SHALL add the private path to
`SESSION_ENV_VARS`, `systemctl --user import-environment`, or
`dbus-update-activation-environment`. `REALM_IMPORT_PATH` retains its existing,
explicit opt-in semantics, but its imported caller PATH SHALL be captured before
the private prefix and exclude `/usr/lib/realm/bin`. It is not the mechanism
which makes the private tools available. Tests SHALL prove private-tool lookup
and PATH isolation for both direct and systemd-user launch paths, including the
`REALM_IMPORT_PATH=1` case.

## Yazi v25.4 configuration migration

The Realm Yazi template has been migrated to the selected `25.4.8` schema. The
static `packaging/tool-sources/test-tool-configs.sh` guard already proves the
required names below and rejects the known legacy names. That source-level
guard is necessary but does not replace the real retained-runtime fixture.

The rendered template SHALL use Yazi v25.4's names as follows:

| Existing Realm field | v25.4 rendered field | Mapping |
|---|---|---|
| `[mgr]` | `[manager]` | Rename the section. |
| `status.mode_normal` | `mode.normal_main`, `mode.normal_alt` | Copy the existing style to both normal variants. |
| `status.mode_select` | `mode.select_main`, `mode.select_alt` | Copy the existing style to both selection variants. |
| `status.mode_unset` | `mode.unset_main`, `mode.unset_alt` | Copy the existing style to both unset variants. |
| `status.permissions_t` | `status.perm_type` | Rename. |
| `status.permissions_r` | `status.perm_read` | Rename. |
| `status.permissions_w` | `status.perm_write` | Rename. |
| `status.permissions_x` | `status.perm_exec` | Rename. |
| `status.permissions_s` | omitted | The selected schema has no corresponding field; omission is intentional. |
| absent field | `status.perm_sep` | Render a separator style using the former `permissions_s` palette role. |

No unsupported renamed field may remain in a rendered v25.4 configuration. A
Hermetic test SHALL render the Realm template at
`YAZI_CONFIG_HOME/theme.toml` and run the selected retained Yazi build in a
controlled terminal-capable environment so its real configuration-loading path
is exercised. A version-pinned strict schema guard SHALL reject the legacy
section and field names before that runtime step: a permissive TOML/Serde parser
is not evidence that an obsolete field had the intended effect. The fixture
shall assert that the canonical fields are consumed, not merely that the TOML
parses.

The Starship validation SHALL render `configs/templates/starship.toml` and run
the selected private `starship prompt` with `STARSHIP_CONFIG`, fixed HOME/cwd,
terminal settings, shell/keymap, status, and command-duration inputs. It SHALL
assert no configuration diagnostic on stderr and a known rendered feature from
the Realm template, rather than only a nonempty default prompt.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| B1 | Given a selected tool or Realm-workspace source bundle, when its intake linkage is validated, then archive, lockfile, every resolved Cargo source, vendor tree, source-replacement config, digest records, and dependency license report agree exactly. | `packaging/tool-sources/test-bundle-linkage.sh`; `packaging/tool-sources/check-bundle-linkage.py` |
| B2 | Given retained-only Debian and Fedora source kits and their actual package build paths with networking disabled and empty Cargo caches, when source-kit recursion, emitted package documentation, selected bundles, the complete Realm workspace build, and all package-relevant staged workspace tests run (excluding only non-packaged `realm-agent-sdd`), then no hidden workspace is accepted, the installed guide names only the retained-kit workflow, all authoritative Cargo build/test invocations use `--frozen --offline --locked`, deterministic source/VCS metadata where applicable, the exact isolated Starship version query and denied upstream Git metadata probes are classified separately, and no recipe fetch path exists. | `packaging/tool-sources/test-native-source-kits.sh`; `packaging/tool-sources/test-native-builds.sh` |
| B3 | Given a native package install and direct or systemd-user Realm session launch, when executable and PATH ownership are inspected, then only `/usr/lib/realm/bin/*` owns the three Realm tools, Realm-launched applications resolve them, and neither user manager nor DBus activation receives the private PATH, including with `REALM_IMPORT_PATH=1`. | `packaging/tool-sources/test-native-builds.sh`; `packaging/session/test-private-tool-path.sh` |
| B4 | Given a rendered Realm Yazi theme at `YAZI_CONFIG_HOME`, when the selected v25.4 runtime loads it, then a strict schema guard has rejected legacy fields and canonical fields are consumed; given a controlled Starship invocation, the rendered configuration has no diagnostics and renders a known Realm feature. | `packaging/tool-sources/test-tool-configs.sh`; selected-runtime assertions in `packaging/tool-sources/test-native-builds.sh` |
| B5 | Given a selected dependency closure, when license evidence is inspected, then every resolved dependency has a linked license/notice record. | `packaging/tool-sources/test-bundle-linkage.sh`; `packaging/tool-sources/check-bundle-linkage.py` |
| B6 | Given an exact committed workspace revision with an unchanged retained lockfile, when its source authority needs rebinding, then read-only repository CI alone creates and validates the candidate archive/records and retains exactly those three files as an artifact without committing or pushing them; a mutable ref, changed lockfile, local packaging command, or unvalidated candidate is rejected. | CI rebind workflow projection and transformation fixtures; bundle-linkage validator in the rebind job. |

## Boundaries and follow-on work

This specification completes the design required for SPEC 0023 A2. Passing the
native-build, installed-ownership, session-PATH, and retained-runtime fixtures
establishes the selected tools' native package availability, but does not by
itself close issue #134: immutable generation update and rollback behavior from
SPEC 0023 A4 remains follow-on work. It also does not claim full user
configuration integration: the actual generated templates are in
`configs/templates/`, while the configuration directories described by ADR
0007 are not currently present. That integration gap requires its own accepted
specification before it becomes a supported capability.

No public package repository, binary distribution, signing service, mirror,
container registry, backend, or network service is introduced.
