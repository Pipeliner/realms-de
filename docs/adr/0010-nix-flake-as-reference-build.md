# ADR 0010 — The Nix flake is the reference build; native deb and rpm packaging is tracked

- **Status:** Accepted (ratified 2026-08-28); see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** Fedora 41-or-later baseline clauses and the
  Fedora part of Decision 7 are superseded by
  [ADR 0015](0015-fedora-44-pre-alpha-baseline.md); all other clauses remain active

## Context

Standing order S6 and `docs/ARCHITECTURE.md` §5 name three first-class targets:
NixOS/Nix, Ubuntu 24.04 LTS and later, Fedora 41 and later. All three are tested
in CI. Anything else is best-effort.

A desktop environment is a bad fit for the usual "just cargo install it" story.
It is not one binary. It is several binaries, a session desktop entry, systemd
user units, a portal configuration, a set of generated theme files, config for
four reused tools (ADR 0007), a font dependency, and a hard runtime dependency
on a compositor and a portal backend. Get any of that wrong and the user gets a
black screen or a file dialog that hangs for 25 seconds (ADR 0011).

`docs/PITFALLS.md` names the failure directly: "works on the author's distro
only". The usual cause is three packagings maintained by hand, diverging until
two of them are quietly broken.

There is also a verification problem specific to desktops. Unit tests do not
tell you whether a session starts. Only booting one does.

## Decision

1. **The root `flake.nix` is the Nix reference build definition.** It imports
   `packaging/nix/` for the derivation, checks, and modules; `flake.lock` is
   intentionally not committed yet, so the current Nix build is evaluable but
   not reproducible until a reviewed lock file is committed. It is the build a
   maintainer runs to reproduce a user's report once locked.
2. **The NixOS VM test is the acceptance test for the session.** It boots a
   VM, logs into helm, and asserts the bar appears and `helm ctl doctor` passes.
   This is the only test that exercises the session contract (ADR 0011) end to
   end, and it is why NixOS is the reference rather than merely a target.
3. **Native distro packaging is tracked explicitly.** Debian uses
   `packaging/debian/`; Fedora uses `packaging/fedora/helm.spec`; shared session
   assets live under `packaging/session/` and `packaging/systemd/`. The optional
   `cargo-deb` fragment is not installed into `Cargo.toml`, and no generated
   metadata source of truth exists today. Dependency lists, binary paths, units,
   and desktop entries therefore require a consistency check rather than a
   false claim of generation.
4. **CI builds and installs all three once each package path is buildable.** A
   job per distro installs its native package into a clean container and runs
   `helmctl doctor`.
5. **The MSRV is pinned by the oldest supported target.** `rust-version` in the
   workspace manifest is currently 1.85 and moves only with a stated reason.
6. **Nothing distro-specific may live outside `packaging/`.** No `#[cfg]` on a
   distro, no path that assumes a filesystem layout.
7. **A pinned river 0.4.x is vendored**, per
   [ADR 0013](0013-river-window-management-backend.md). The target distros ship
   0.3.x or river-classic, neither of which speaks
   `river-window-management-v1`. Nix currently constrains river through its
   unlocked `nixpkgs` input; Debian and Fedora record alternative/runtime
   requirements but do not yet build or bundle a verified `helm-river` package.
   Treat that as an unsatisfied packaging obligation, not as a shared pin. It
   must be resolved and tested before a distributable session is claimed.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Use unreviewed, ad-hoc distro packaging** | It avoids maintaining packaging definitions in the repository | It leaves package contents and dependencies unverifiable until a user fails to start a session. Tracked native definitions plus a consistency check make drift visible without pretending a generator exists |
| **Ship only a tarball or an AppImage** | One artifact, no distro work at all, works everywhere in principle | A desktop environment is not a single application. It must install a session desktop entry where the display manager will find it, install systemd user units, and declare a dependency on a portal backend and on a compositor. None of that is expressible in a tarball, and AppImage's sandbox assumptions are wrong for something that *is* the session. It would also make `helm ctl doctor` the only line of defence against a broken install |
| **Make a traditional distro the reference and treat Nix as a port** | Larger user base; more contributors already know `debhelper`; the reference build would match what most users run | We would lose the VM test, which is the single most valuable test in the project. Reproducing a user's exact environment would also stop being a one-line operation. Native package definitions remain supported targets, but do not replace the reference session test |
| **Distribute via Flatpak** | Handles dependencies and works across distros | Flatpak is for applications. It cannot install a session, cannot own systemd user units, and its sandbox is on the wrong side of the boundary for a compositor and a session daemon. It is also, per ADR 0005, one of the theming pipeline's documented limits |
| **Nix flake plus a distro-agnostic install script** | Simple; no packaging tooling to learn | An install script is an unversioned, unremovable package manager. Upgrades and uninstalls become the user's problem |

## Consequences

### Good

- One reviewable Nix definition of what helm *should* be, including runtime
  dependencies. It becomes reproducible only when `flake.lock` is committed.
- The VM test catches the session contract failures (portals, environment
  import, unit ordering) before a user does. Nothing else can.
- Adding a fourth target means adding a native package definition and extending
  the consistency checks.
- Contributors can reproduce a bug with one command.
- The explicit package definitions make each distro's policy-visible choices
  reviewable; the consistency check identifies overlapping values that drift.

### Bad

- Native Debian and Fedora definitions duplicate some shared facts. Until the
  planned consistency check exists, a changed dependency, binary path, unit or
  desktop entry can drift between them.
- The optional `cargo-deb` fragment is not active metadata, and Fedora's native
  `.spec` is the supported rpm path. Neither `cargo-deb` nor
  `cargo-generate-rpm` is a current shared source of truth.
- Nix expertise becomes load-bearing for the project. A contributor who cannot
  read the flake cannot fully review a packaging change.
- Vendoring river (point 7) means a pinned Zig toolchain in the deb and rpm
  builds, and it makes helm responsible for shipping compositor security fixes
  promptly. This is a cost ADR 0013 accepted knowingly.
- The VM test is slow and needs KVM on the runner, which constrains CI choices.
- Fedora's SELinux policy is asserted to need no custom labels
  (`docs/ARCHITECTURE.md` §5). That is an assumption until the Fedora CI job
  runs with enforcing mode on.

### Neutral

- The three-target list is a choice about where to spend effort, not a technical
  limit. Arch, openSUSE and others require their own tracked package definition
  and an extension of the consistency check if they become supported.

## Reversal

Medium. `packaging/` is self-contained, so changing the supported native
package formats does not require changing crates. It does require changing and
re-validating every affected package definition and its overlap checks.

A possible future change is to introduce a reviewed shared metadata model for
the facts that truly are common. That would be a new decision: it must define
the source of truth, native escape hatches, generation/review flow, and a guard
that proves generated output matches each supported package format. It is not
the current model.

The signal to reconsider is concrete: a distro maintainer willing to package
helm properly. At that point idiomatic packaging becomes worth its cost, because
someone else is paying it.

## Guard

- *Planned (M3):* the NixOS VM test — boots the session, asserts the bar
  surface appears and `helm ctl doctor` exits zero.
- *Planned (M3):* three container jobs, one per distro, building and installing
  its tracked native package into a clean image and running `helm ctl doctor`.
- *Planned (M3):* a metadata consistency test asserting that the dependency
  lists, binary paths, session assets, and portal policy in the root flake,
  Debian control files, and Fedora spec agree where the targets overlap. This
  is the guard for the decision itself; it does not claim a nonexistent shared
  generator.
- *Planned (M3):* an MSRV job pinned to the `rust-version` floor, so raising it
  is a deliberate change rather than an accident.
- *Planned (M3):* a Fedora job with SELinux enforcing, which is what turns the
  "SELinux-clean" claim from an assumption into a tested fact.

## Needs a human

**Where do the distro packages actually live?** Building a `.deb` and an `.rpm`
in CI is not distribution. The options:

1. **GitHub Releases only.** Users download and `dpkg -i` or `rpm -i`. Zero
   infrastructure, no signing key to manage, no upgrade path.
2. **A self-hosted apt repository and a Fedora Copr.** Real `apt upgrade` and
   `dnf upgrade`. Requires a signing key, somewhere to host it, and a person
   responsible for both. `docs/ARCHITECTURE.md` §5 already mentions "a
   PPA-shaped repo layout", so this is the assumed direction, but it has not
   been decided.
3. **Submit to the official Debian and Fedora repositories.** Best outcome for
   users, and the only one that gets helm into a default `apt install`. Requires
   idiomatic packaging (see Consequences) and a sponsor, and it is months of
   work by someone with standing in those communities.

**Recommendation: option 1 for M3, option 2 once there are users to upgrade.**
Option 3 needs a person, not a decision.

The unresolved sub-questions a human must answer: who holds the package signing
key and where; whether we can use GitHub Pages for an apt repository or need
hosting; and whether Copr's build limits fit our CI cadence.

Tracked as a `needs-human` issue (standing order S3).
