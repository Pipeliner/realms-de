# SPEC 0024 — M1 private Yazi and Starship tool bundle

- **Status:** Accepted (2026-08-31; amended 2026-09-01)
- **Milestone:** M1
- **Issue:** [#134](https://github.com/Pipeliner/realms-de/issues/134)
- **Refines:** [SPEC 0023](0023-m1-tool-source-intake.md)
- **Decisions:** [ADR 0007](../adr/0007-reuse-yazi-btop-starship.md),
  [ADR 0010](../adr/0010-packaged-helm-sdd-git-runtime.md)
- **Evidence:** [M1 provenance research](../research/2026-08-30-m1-yazi-starship-provenance.md)

## Purpose

Make the SPEC 0023 A2 source-build route concrete for the native M1 packages.
The route supplies Helm-owned Yazi, `ya`, and Starship executables without
network access at package-build time, without taking ownership of distribution
executables, and without publishing any Helm infrastructure.

## Selected compatibility baseline

The first native source-bundle implementation selects these versions and the
existing native Rust floor:

| Tool | Selected version | Compiler baseline | Reason |
|---|---:|---:|---|
| Starship | `1.23.0` | Rust `1.85` | Its tagged manifest declares that floor and the current Helm Starship template was checked against that release. |
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
package recipes**, including the Helm workspace build and test commands already
in `debian/rules` and `packaging/fedora/helm.spec`. The native source release
SHALL therefore retain a separately identified Helm-workspace `Cargo.lock`,
vendor tree, source-replacement configuration, and dependency license/notice
report, or the recipe SHALL not invoke Cargo for that workspace. It is not
permitted to prove only the bundled tools offline while allowing the package's
own Cargo commands to rely on a pre-populated cache or live registry.

### Helm workspace source authority

The Helm workspace closure SHALL bind to the immutable package source archive
at `packaging/tool-sources/bundles/helm-workspace/source.tar.gz`. That archive
is the source authority and SHALL be the exact archive unpacked for Helm's
Cargo build by both native package paths; no second source copy or build-time,
unbound `git archive` is permitted. A controlled intake MAY create this one
canonical archive from the record-bound repository commit, but it SHALL retain
the resulting bytes and digest as this source authority outside, and before,
either native package path. Debian/RPM preparation and build phases SHALL NOT
invoke `git`, `git archive`, archive creation, or any fetch operation, even if
the resulting bytes would match the recorded digest. Its tracked
`packaging/tool-sources/bundles/helm-workspace/bundle.toml` record SHALL bind
the archive SHA-256 to the repository commit, commit timestamp, and
provenance/notice record used at intake. The linkage fixture SHALL unpack the
source authority, verify the record's archive digest, and require its root
`Cargo.lock` to match the separately retained, digest-bound Helm-workspace
lockfile byte-for-byte.

The Helm-workspace source-replacement configuration is a separately retained
build input: native recipes SHALL stage it at the unpacked source root before
Cargo runs. The linkage fixture SHALL verify that configuration, the retained
vendor tree, every resolved dependency, and the dependency license report as
for selected tool bundles. Before each **Helm-workspace** Cargo invocation, the
Debian source package path and Fedora `%prep` path SHALL verify the same
retained archive digest and build only its unpacked source tree. The native
offline-build fixture SHALL substitute a same-name source archive with a
different digest in each path and require refusal before Cargo runs. A moving
checkout or unspecified local RPM `Source0` is not an immutable source
authority and SHALL NOT satisfy this contract.

The canonical source authority intentionally contains no Git worktree metadata.
Both native paths SHALL build the complete staged workspace, but their native
test invocation SHALL run all package-relevant staged workspace members and
exclude only the non-packaged `helm-agent-sdd` member: that member's 47 gate
fixtures validate live Git worktree state and remain mandatory in its existing
source-worktree test lane. Native package preparation SHALL NOT synthesize a
Git repository or weaken the no-Git rule merely to run those worktree fixtures.

The outer Debian and RPM source kits SHALL remain packaging inputs rather than
hidden workspaces at every nesting depth. Outside the canonical retained
`packaging/tool-sources/bundles/helm-workspace/source.tar.gz`, a kit SHALL NOT
contain a `Cargo.toml`, `crates/` or `.cargo/` workspace directory, or another
`source.tar.gz`; the separately retained Helm `Cargo.lock` is permitted only at
its one canonical bundle path. This recursive rule applies inside otherwise
allowed packaging-metadata directories as well as at kit top level.

The installed native-package build guide SHALL come from tracked outer
packaging metadata, not from the identity-bound archive's historical
`docs/INSTALL.md`. The emitted Debian and RPM packages SHALL document
`build-native-source-kits.sh` and SHALL NOT retain the superseded checkout
`ln -s packaging/debian` or moving `git archive ... HEAD` workflows. Updating
package guidance SHALL NOT mutate or regenerate the canonical source authority.

The Debian and Fedora package paths SHALL unpack only these retained inputs and
build using `cargo --frozen --offline --locked`. They MAY consume declared,
target-provided C toolchain/system dependencies, but SHALL NOT acquire an
upstream source, registry package, Git dependency, or release artifact over the
network. A test fixture SHALL run the actual `debian/rules` build path and the
Fedora RPM build phase with networking disabled and empty Cargo registry/Git
caches, reject a closure/configuration/lockfile mismatch, and fail when an
adversarial injected-fetch attempt is present in either recipe.

The Yazi build SHALL set deterministic source-date and VCS metadata. The intake
record SHALL bind the selected tag to its upstream commit SHA and commit
timestamp; the package recipe SHALL set the exact version/metadata inputs
accepted by the selected Yazi `vergen` build from those retained values. It
SHALL not accept `vergen`'s fallback metadata as release evidence or assume that
`SOURCE_DATE_EPOCH` alone controls it. The reproducibility fixture SHALL build
in two different directories/times and either compare the normalized declared
artifacts or explicitly record and justify every remaining non-identical field.

## Executable ownership and session scope

Native packages SHALL install only these Helm-owned executables:

```text
/usr/lib/helm/bin/yazi
/usr/lib/helm/bin/ya
/usr/lib/helm/bin/starship
```

They SHALL NOT install or replace `/usr/bin/yazi`, `/usr/bin/ya`, or
`/usr/bin/starship`. Distribution-provided tools remain independently owned and
may coexist.

For the direct-launch path (without a systemd user manager),
`packaging/session/helm-session` SHALL prepend `/usr/lib/helm/bin` to the Helm
session process environment and its descendants. For the normal systemd-user
path, the Helm-owned service units SHALL receive an explicit private PATH by a
Helm-owned unit-level environment mechanism; `helm-wm`, `helm-bar`, and the
applications they launch therefore inherit it without changing the user
manager's global environment. Neither route SHALL add the private path to
`SESSION_ENV_VARS`, `systemctl --user import-environment`, or
`dbus-update-activation-environment`. `HELM_IMPORT_PATH` retains its existing,
explicit opt-in semantics, but its imported caller PATH SHALL be captured before
the private prefix and exclude `/usr/lib/helm/bin`. It is not the mechanism
which makes the private tools available. Tests SHALL prove private-tool lookup
and PATH isolation for both direct and systemd-user launch paths, including the
`HELM_IMPORT_PATH=1` case.

## Yazi v25.4 configuration migration

The current Helm Yazi template targets a different schema and is not valid
configuration evidence for Yazi `25.4.8`. Its migration is part of selecting
that version, rather than a later cosmetic change.

The rendered template SHALL use Yazi v25.4's names as follows:

| Existing Helm field | v25.4 rendered field | Mapping |
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
Hermetic test SHALL render the Helm template at
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
the Helm template, rather than only a nonempty default prompt.

## Acceptance criteria

| # | Given / When / Then | Test |
|---|---|---|
| B1 | Given a selected tool or Helm-workspace source bundle, when its intake linkage is validated, then archive, lockfile, every resolved Cargo source, vendor tree, source-replacement config, digest records, and dependency license report agree exactly. | To be implemented: source-bundle linkage fixture. |
| B2 | Given retained-only Debian and Fedora source kits and their actual package build paths with networking disabled and empty Cargo caches, when source-kit recursion, emitted package documentation, selected bundles, the complete Helm workspace build, and all package-relevant staged workspace tests run (excluding only non-packaged `helm-agent-sdd`), then no hidden workspace is accepted, the installed guide names only the retained-kit workflow, all Cargo invocations use `--frozen --offline --locked`, deterministic source/VCS metadata where applicable, and no recipe fetch path exists. | `packaging/tool-sources/test-native-source-kits.sh`; `packaging/tool-sources/test-native-builds.sh` (Helm-workspace portion; selected Yazi/Starship bundle integration remains follow-on work) |
| B3 | Given a native package install and direct or systemd-user Helm session launch, when executable and PATH ownership are inspected, then only `/usr/lib/helm/bin/*` owns the three Helm tools, Helm-launched applications resolve them, and neither user manager nor DBus activation receives the private PATH, including with `HELM_IMPORT_PATH=1`. | To be implemented: package/session ownership fixture. |
| B4 | Given a rendered Helm Yazi theme at `YAZI_CONFIG_HOME`, when the selected v25.4 runtime loads it, then a strict schema guard has rejected legacy fields and canonical fields are consumed; given a controlled Starship invocation, the rendered configuration has no diagnostics and renders a known Helm feature. | To be implemented: rendered-config runtime fixture. |
| B5 | Given a selected dependency closure, when license evidence is inspected, then every resolved dependency has a linked license/notice record. | To be implemented: dependency-license fixture. |

## Boundaries and follow-on work

This specification completes the design required for SPEC 0023 A2 only. It
does not claim A2 is implemented, does not establish target availability (A3),
and does not establish immutable generation update or rollback behavior (A4).
It also does not claim full user configuration integration: the actual generated
templates are in `configs/templates/`, while the configuration directories
described by ADR 0007 are not currently present. That integration gap requires
its own accepted specification before it becomes a supported capability.

No public package repository, binary distribution, signing service, mirror,
container registry, backend, or network service is introduced.
