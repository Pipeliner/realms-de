# SPEC 0028 — Ubuntu private River Debian package

- **Status:** Accepted (2026-09-13)
- **Milestone:** M3
- **Issue:** [#98](https://github.com/Pipeliner/realms-de/issues/98)
- **Decisions:** [ADR 0022](../adr/0022-ubuntu-river-private-closure.md),
  [ADR 0011](../adr/0011-session-integration-contract.md)
- **Depends on:** [SPEC 0026](0026-ubuntu-river-private-closure.md)
- **Supersedes / Superseded by:** Implements SPEC 0026's deliberately deferred
  Debian integration only

## Purpose

Turn the CI-proven Ubuntu Noble River 0.4.8 closure into the native package
that an installed Realm session can actually select. The result is a separate
`realm-river` package so Realm's Rust package remains independently buildable
and Fedora's distribution-owned River route is unchanged.

## Scope

**In:** an amd64 retained-only `realm-river` Debian source kit; network-disabled
package compilation from SPEC 0026's acquired inputs; the private runtime
payload below `/usr/lib/realm`; Realm package dependency resolution; and a
clean Noble minbase installation/version/ELF/preflight proof.

**Out:** an apt repository or published binary; Fedora or Nix changes; another
architecture or Zig toolchain; Debian archive policy submission; graphical
login; real DRM/input devices; changing River/wlroots features; and packaging
development headers, static libraries, build tools or diagnostic utilities.

## Package and path contract

1. The source and binary package are named `realm-river`, version `0.4.8-1`,
   and architecture `amd64`. This is the only architecture supported by the
   selected `zig-x86_64-linux-0.16.0` input. No claim is made for another
   architecture.
2. `realm-river` provides versioned virtual package `river (= 0.4.8)`. The
   `realm` package continues to accept `realm-river (>= 0.4.8) | river (>=
   0.4.0)`: Noble resolves the first alternative, while Fedora and Nix retain
   their existing target-specific River ownership.
3. The package installs River only as `/usr/lib/realm/bin/river`; it neither
   installs nor replaces `/usr/bin/river`. The Realm session already prepends
   `/usr/lib/realm/bin` to its child-only `PATH`, so its unset
   `REALM_COMPOSITOR` resolves the private binary. The path remains absent from
   the systemd user manager and D-Bus activation environments.
4. Private shared libraries live in `/usr/lib/realm/lib` and retain SPEC
   0026's relative runpaths. Selected libinput quirks live in
   `/usr/lib/realm/share/libinput`, the exact path compiled into the private
   libinput library. The binary package also contains the selected River
   executable and every private shared object required by its complete
   transitive ELF graph.
5. The binary package excludes `include/`, pkg-config records,
   `wayland-scanner`, libinput diagnostic tools, static libraries and the
   private build tree. It does not install libinput rules or callouts into
   `/lib/udev` or `/usr/lib/udev`, and therefore does not replace Noble's
   distro-owned input database.

## libinput host-data boundary

Noble's `libinput-bin` 1.25.0-1ubuntu2 owns the active udev callouts, the
`80-libinput-device-groups.rules` and `90-libinput-fuzz-override.rules` rules,
and its system quirks. The two selected 1.31.3 rule templates are byte-identical
to the 1.25.0 templates, which proves only that their property names and
callout references match. It does not prove complete behavioral compatibility
between the 1.25 callout implementations and the 1.31 library. The
`realm-river` package depends on `libinput-bin (>= 1.25.0)` and leaves those
global files under distro ownership. Its private libinput library is compiled
to read the selected 1.31.3 quirks from `/usr/lib/realm/share/libinput`; it does
not read or replace Noble's `/usr/share/libinput` quirks. mtdev and libwacom
support remain enabled exactly as in SPEC 0026. Actual input-device behavior
remains unproven until the later hardware obligation.

## Retained source-kit contract (L4)

1. Networked acquisition remains the explicit SPEC 0026 command. A new source
   kit producer accepts only that command's completed cache and a new output
   path. It validates the manifest, every archive digest, the exact seven Zig
   package names/content-hash directories, the selected Zig executable and an
   exact cache inventory before copying anything.
2. The source kit contains only `debian/`, `flake.lock`, the required
   `packaging/river/` build/check scripts and manifest, and a normalized
   closure-input directory containing the digest-checked archives, selected Zig
   tool and seven Zig package trees. It contains no Realm workspace checkout,
   Cargo authority, network cache, compiler output or nested source kit.
3. Source-kit construction invokes no network client. Missing, extra,
   symlinked or non-regular input is fatal. Output publication is one
   no-replace rename after complete validation; a partial destination is never
   accepted as a kit.
4. The retained source kit's package-build entrypoint captures the parent
   network namespace, requires non-interactive `sudo`, and uses it only to
   invoke `unshare --net` before `dpkg-buildpackage`. This is the same
   privileged network-namespace boundary proven by SPEC 0026 on the Noble CI
   runner; it does not depend on an unprivileged user namespace. The package
   rules require the captured parent identity and the closure builder verifies
   that its current namespace differs and has no usable interface before
   compilation. Invoking the rules without the entrypoint, unavailable
   non-interactive privilege, or failed namespace creation/verification is
   fatal, with no connected retry. Every Meson setup keeps
   `--wrap-mode=nofallback`; River keeps Zig `--system`.

## Staged build and runtime projection (L4)

1. C dependencies are configured with logical prefix `/usr/lib/realm` and
   installed beneath Debhelper's `DESTDIR`. `PKG_CONFIG_SYSROOT_DIR` and the
   staged tool path feed later builds without changing the logical prefix.
   This preserves libinput's compiled quirks path rather than embedding a CI
   temporary directory.
2. River is installed into the same staged logical tree. Before packaging, the
   staged ELF objects must pass SPEC 0026's feature, relative-runpath, complete
   recursive resolution and exact `0.4.8 +xwayland` checks with
   `LD_LIBRARY_PATH` unset. Those checks address objects through the staging
   root; they do not claim the logical-prefix runtime data is readable before
   installation into a root containing `/usr/lib/realm`.
3. Debhelper projects only the package-and-path contract's runtime files and
   derives the remaining Noble shared-library dependencies. It must not emit a
   dependency on one of the private library families carried by the same
   package.
4. The ordinary `realm` Debian package and this `realm-river` package are built
   separately. Neither source kit is nested inside the other, and changing the
   compositor package does not weaken the Realm workspace bundle's existing
   freshness and no-fetch checks.

## Verification obligations (L5)

| # | Given / When / Then | Evidence |
|---|---|---|
| D1 | Given the canonical acquired cache and missing/extra/symlink/cache-mutation fixtures, when the source-kit producer runs, then only the exact retained input becomes a complete kit and no network sentinel is invoked. | `packaging/river/test-noble-river-source-kit.sh` |
| D2 | Given the Debian metadata and a staged fixture tree, when package ownership is projected, then the exact runtime payload is below `/usr/lib/realm`, no `/usr/bin/river`, development file, global udev rule/callout or build input enters the binary package, and metadata names amd64 `realm-river` 0.4.8-1 with versioned River provision. | `packaging/river/test-noble-river-package.sh` |
| D3 | Given the package build, when compilation runs, then the namespace/network checks, exact wlroots features, private libinput 1.31.3, staged all-object recursive ELF resolution and exact version probe pass before `dpkg-buildpackage` succeeds; no pre-install claim is made for logical-prefix data lookup. | `.github/workflows/distro.yml` — `ubuntu-river-debian` build log |
| D4 | Given a fresh Noble minbase root, when `realm-river` and `realm` are installed together with archive dependencies, then dpkg reports both configured, `realm`'s River dependency is satisfied by `realm-river`, selected libinput quirks exist at the compiled `/usr/lib/realm/share/libinput` path, `/usr/lib/realm/bin/river -version` prints exactly `0.4.8 +xwayland`, resolution stays private with `LD_LIBRARY_PATH` unset, and Realm's existing private-PATH fixture plus installed `realm-session --check` find that default compositor. | `.github/workflows/distro.yml` — `ubuntu-river-debian`; `packaging/session/test-private-tool-path.sh` |
| D5 | Given the successful job, when support claims are inspected, then they say only that the two native packages build and clean-install on amd64 Noble; graphical login, DRM/input hardware and package publication remain unverified. | SPEC 0028 boundary and `docs/INSTALL.md` |

## Failure modes

- A missing or changed source input fails before source-kit publication.
- Package compilation cannot create a fresh network namespace or finds a usable
  interface: fail without compiling or retrying online.
- A private dependency resolves to the host, the staged libinput quirks path is
  not `/usr/lib/realm/share/libinput`, or a required feature is disabled: fail
  before package creation.
- The binary payload contains a global udev file, `/usr/bin/river`, build input,
  development file or unowned library dependency: fail package projection.
- Either package fails to configure in the clean Noble root or the installed
  default path/version/ELF checks fail: do not claim native integration.

## Open questions

None for this increment. Graphical verification remains a later VM/hardware
obligation, and producing an arm64 package requires a separately selected Zig
input and CI proof rather than renaming this amd64 result.
