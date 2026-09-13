# ADR 0022 — Ubuntu Noble builds a private pinned River closure

- **Status:** Accepted (2026-09-13)
- **Deciders:** realm maintainer, repository owner
- **Supersedes / Superseded by:** Supersedes ADR 0013 Decision 4 only for
  Ubuntu; Fedora remains governed by ADR 0015 and Nix by ADR 0010/0013

## Context

ADR 0013 selected River 0.4 and assumed that native packaging could vendor a
pinned River build. Ubuntu 24.04 Noble has no River or Zig package and its
wlroots 0.17 cannot build River 0.4.8. Checking River 0.4.8 and wlroots 0.20.2
also found seven libraries below their required versions in Noble: Wayland,
wayland-protocols, libdrm, pixman, libxkbcommon, libdisplay-info and wlroots
itself. Building only River would therefore leave the Ubuntu target unusable.

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
   omitted from this first Ubuntu closure.
4. Private shared libraries and River receive relative runpaths and are checked
   by ELF resolution before River's version command is executed. They do not
   replace Noble's system libraries.
5. The first increment is a CI-only source-build probe. It proves the complete
   feature-bearing closure builds and executes its version path on Noble. It
   does not publish packages, integrate the closure into the `.deb`, claim a
   graphical login, or claim a DRM device was exercised.
6. No PPA, COPR, public repository, mirror, binary hosting service or raised
   Ubuntu minimum is introduced. Native build proof remains CI-only.

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

- Realm owns rebuilds for River and seven private library sources on Ubuntu.
- The probe adds a Zig toolchain and a non-trivial native graphics build to CI.
- A successful container build still leaves Debian integration and graphical
  login unproven.

### Neutral

- Noble continues supplying Mesa, libinput, libseat, Xwayland and the remaining
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
features, verify relative runpaths and private ELF resolution, and execute
`river -version` as `river 0.4.8`. A missing network namespace is a failure, not
a skip.

## Needs a human

None. The repository owner selected the private CI-built closure on issue #98
and explicitly retained Ubuntu 24.04 without authorizing hosting.
