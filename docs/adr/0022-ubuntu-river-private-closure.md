# ADR 0022 — Ubuntu Noble builds a private pinned River closure

- **Status:** Accepted (2026-09-13)
- **Deciders:** realm maintainer, repository owner
- **Supersedes / Superseded by:** Supersedes ADR 0013 Decision 4 only for
  Ubuntu; Fedora remains governed by ADR 0015 and Nix by ADR 0010/0013

## Context

ADR 0013 selected River 0.4 and assumed that native packaging could vendor a
pinned River build. Ubuntu 24.04 Noble has no River or Zig package and its
wlroots 0.17 cannot build River 0.4.8. Checking River 0.4.8 and wlroots 0.20.2
also found eight libraries below their required versions in Noble: Wayland,
wayland-protocols, libdrm, libinput, pixman, libxkbcommon, libdisplay-info and
wlroots itself. Building only River would therefore leave the Ubuntu target
unusable.

Fedora 44 now has an official River 0.4.8 package and is outside this decision.
Nix already obtains the tested closure from the locked nixpkgs input.

## Decision

1. Ubuntu 24.04 remains an M3 target. Its River route is a private closure built
   from the exact sources and flags in SPEC 0026, not a Noble River package.
2. Source acquisition is an explicit, digest-checked CI phase. Compilation runs
   afterward in a network namespace with no usable network and with Meson
   fallbacks disabled; River's Zig dependencies are prefetched and supplied
   through Zig's offline `--system` interface.
3. The closure retains real DRM, libinput, GBM/GLES2, session and Xwayland
   support. A headless-only build does not satisfy this decision. Optional
   Vulkan, nested X11, libliftoff, colour-management and xcb-errors support are
   omitted from this first Ubuntu closure. The private libinput build retains
   both mtdev and libwacom device support; it is not permitted to make River
   compile by dropping supported input-device classes.
4. Private shared libraries and River receive relative runpaths and are checked
   by ELF resolution before River's version command is executed. They do not
   replace Noble's system libraries.
5. The first increment is a CI-only source-build probe. It proves the complete
   feature-bearing closure builds and executes its version path on Noble.
   SPEC 0028's next increment packages that same closure as a separate amd64
   `realm-river` Debian package below `/usr/lib/realm`; it does not turn the
   Realm Rust source kit into a graphics source package.
6. No PPA, COPR, public repository, mirror, binary hosting service or raised
   Ubuntu minimum is introduced. Native build proof remains CI-only.
7. The selected libinput quirks remain private to `/usr/lib/realm`. Noble's
   observed byte-identical libinput 1.25/1.31 rule templates retain the same
   property names and callout references, but do not prove the two callout
   implementations behaviorally equivalent. Noble's global rules and callouts
   remain distro-owned; Realm neither replaces global host input data nor
   drops mtdev or libwacom device support to avoid that ownership boundary.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Private pinned source closure | Keeps Ubuntu 24.04 and the exact protocol generation Realm implements without creating distribution infrastructure | **Chosen.** It is the only currently authorized route that can make the target self-contained |
| Require or operate a PPA | Lets Realm depend on ordinary distro packages and centralizes compositor updates | No repository, signing authority or hosting decision exists, and creating one is outside the MVP slice |
| Raise the Ubuntu minimum | Avoids most private graphics dependencies once a later Ubuntu ships them | Contradicts the accepted Ubuntu 24.04 target and drops LTS users instead of solving the dependency |
| Build a headless-only River | Smaller dependency surface and enough for a version command | Does not build the session Realm ships: it omits DRM/input rendering and cannot support graphical login |
| Redistribute an upstream binary | Avoids carrying Zig in CI | River does not publish a self-contained Noble closure, and this would introduce a separate binary-provenance decision |

## Consequences

### Good

- Noble receives the exact River/wlroots protocol generation already proven by
  the NixOS VM without changing the target or creating package infrastructure.
- Digest-pinned acquisition and network-disabled compilation separate source
  provenance from build execution.
- Feature and ELF checks prevent a successful headless build or accidental link
  against Noble's obsolete graphics libraries from masquerading as closure
  proof.

### Bad

- Realm owns rebuilds for River and eight private library sources on Ubuntu.
- The probe adds a Zig toolchain and a non-trivial native graphics build to CI.
- Even after SPEC 0028 proves Debian build and clean installation, graphical
  login and DRM/input hardware remain unproven.

### Neutral

- Noble continues supplying Mesa, libseat, Xwayland and the remaining
  system runtime. Private libraries are limited to the version gaps.
- This decision neither changes Fedora's official-package route nor Nix's
  locked closure.

## Reversal

Low once Ubuntu provides a tested River 0.4-compatible package: remove the
private closure job and change Debian dependency resolution to that package.
Reconsider when a supported Ubuntu release supplies the complete compatible
closure, or when Realm adopts its own compositor. Moving the build to a public
repository is a separate hosting/signing decision, not an implicit next step.

## Guard

SPEC 0026's manifest fixtures reject missing, duplicate or malformed pins. Its
Ubuntu 24.04 CI job must acquire and hash the selected sources, compile the
closure inside a fresh network namespace, assert the six required wlroots
features, inspect River and every private shared object for complete transitive
ELF resolution with `LD_LIBRARY_PATH` unset, verify relative runpaths, and
execute `river -version` with exact stdout `0.4.8 +xwayland`. A missing network
namespace is a failure, not a skip.

## Needs a human

None. The repository owner selected the private CI-built closure on issue #98
and explicitly retained Ubuntu 24.04 without authorizing hosting.
