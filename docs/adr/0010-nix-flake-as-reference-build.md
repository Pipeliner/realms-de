# ADR 0010 — The Nix flake is the reference build; deb and rpm are generated

- **Status:** Accepted (2026-08-26) — provisional; see Reversal
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** —

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

1. **The flake is the definition.** `packaging/nix/` contains the flake with
   `packages.*`, `nixosModules.helm` and `homeManagerModules.helm`. It is the
   build that must always work, and it is what a maintainer runs to reproduce a
   user's report.
2. **The NixOS VM test is the acceptance test for the session.** It boots a
   VM, logs into helm, and asserts the bar appears and `helm ctl doctor` passes.
   This is the only test that exercises the session contract (ADR 0011) end to
   end, and it is why NixOS is the reference rather than merely a target.
3. **Distro packages are generated from the same metadata, not maintained in
   parallel.** `.deb` via `cargo-deb`, `.rpm` via `cargo-generate-rpm`, both
   driven by `[package.metadata.*]` in the workspace manifests. Dependency
   lists, binary paths, unit files and the desktop entry are declared once.
4. **CI builds and installs all three.** A job per distro, installing the
   generated package into a clean container and running `helm ctl doctor`.
5. **The MSRV is pinned by the oldest supported target.** `rust-version` in the
   workspace manifest is currently 1.82 and moves only with a stated reason.
6. **Nothing distro-specific may live outside `packaging/`.** No `#[cfg]` on a
   distro, no path that assumes a filesystem layout.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Maintain three packagings by hand** | Each uses its ecosystem's idioms properly: a real `debian/rules`, a real `.spec` with `%post` scriptlets, packaging that a Debian or Fedora maintainer would recognise and could adopt. Generated packaging is always slightly wrong by native standards | Three copies of the dependency list drift, and the drift is invisible until a user on the least-used distro cannot log in. That is precisely the pitfall on the register. Generation makes divergence structurally impossible for the parts that matter |
| **Ship only a tarball or an AppImage** | One artifact, no distro work at all, works everywhere in principle | A desktop environment is not a single application. It must install a session desktop entry where the display manager will find it, install systemd user units, and declare a dependency on a portal backend and on a compositor. None of that is expressible in a tarball, and AppImage's sandbox assumptions are wrong for something that *is* the session. It would also make `helm ctl doctor` the only line of defence against a broken install |
| **Make a traditional distro the reference and treat Nix as a port** | Larger user base; more contributors already know `debhelper`; the reference build would match what most users run | We would lose the VM test, which is the single most valuable test in the project. Reproducing a user's exact environment would also stop being a one-line operation. And the generation direction has to point somewhere: pointing it away from the most precisely specified build makes the generated end less reliable, not more |
| **Distribute via Flatpak** | Handles dependencies and works across distros | Flatpak is for applications. It cannot install a session, cannot own systemd user units, and its sandbox is on the wrong side of the boundary for a compositor and a session daemon. It is also, per ADR 0005, one of the theming pipeline's documented limits |
| **Nix flake plus a distro-agnostic install script** | Simple; no packaging tooling to learn | An install script is an unversioned, unremovable package manager. Upgrades and uninstalls become the user's problem |

## Consequences

### Good

- One reproducible definition of what helm *is*, including its runtime
  dependencies, which is exactly the knowledge that usually lives only in a
  maintainer's head.
- The VM test catches the session contract failures (portals, environment
  import, unit ordering) before a user does. Nothing else can.
- Adding a fourth target means adding a generator, not a maintainer.
- Contributors can reproduce a bug with one command.
- Dependency version floors are declared once and checked on all three distros.

### Bad

- Generated `.deb` and `.rpm` packages are not idiomatic. They will not satisfy
  Debian policy or Fedora packaging guidelines without work, which means helm
  cannot enter the official repositories of either distro as-is. That is a real
  cost and it is the strongest argument for the hand-maintained alternative.
- `cargo-deb` and `cargo-generate-rpm` do not cover everything. Post-install
  scriptlets, alternatives registration and SELinux contexts need hand-written
  fragments, and those fragments are the parts that will drift.
- Nix expertise becomes load-bearing for the project. A contributor who cannot
  read the flake cannot fully review a packaging change.
- The VM test is slow and needs KVM on the runner, which constrains CI choices.
- Fedora's SELinux policy is asserted to need no custom labels
  (`docs/ARCHITECTURE.md` §5). That is an assumption until the Fedora CI job
  runs with enforcing mode on.

### Neutral

- The three-target list is a choice about where to spend effort, not a technical
  limit. Arch, openSUSE and others are buildable from the same metadata by
  anyone who wants to add a generator.

## Reversal

Medium. `packaging/` is self-contained, so switching the generation direction
means rewriting the generators, not the crates. Estimated one to two weeks per
distro to move to hand-maintained native packaging, plus ongoing maintenance
forever, which is the actual cost.

A partial reversal is more likely and is cheap: keep generation as the source of
truth and hand-write a native `.spec` or `debian/` directory *derived* from it
for repository submission, accepting one-way drift at release time only.

The signal to reconsider is concrete: a distro maintainer willing to package
helm properly. At that point idiomatic packaging becomes worth its cost, because
someone else is paying it.

## Guard

- *Planned (M3):* the NixOS VM test — boots the session, asserts the bar
  surface appears and `helm ctl doctor` exits zero.
- *Planned (M3):* three container jobs, one per distro, installing the generated
  package into a clean image and running `helm ctl doctor`.
- *Planned (M3):* a metadata consistency test asserting that the dependency
  lists in the flake, the deb metadata and the rpm metadata are generated from
  the same source and agree. This is the guard for the decision itself.
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
