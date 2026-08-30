# M1 Yazi and Starship provenance: research record

**Verified:** 2026-08-30  
**Status:** non-normative research record  
**Scope:** package-source facts for [#134](https://github.com/Pipeliner/realms-de/issues/134).
This record does not select a source, add a repository, vendor a binary, or
authorize package publication.

## Verified facts

| Subject | Primary-source evidence | Consequence |
|---|---|---|
| Ubuntu 24.04 archive | Starship's [official installation guide](https://starship.rs/guide/) lists Ubuntu archive installation only from 25.04. The project Debian control file separately records a dated Noble observation that neither Yazi nor Starship was in the archive. | Current Starship guidance provides no Noble archive route; package availability must be revalidated when the source policy is selected. |
| Yazi on Debian/Ubuntu | Yazi's [official installation guide](https://yazi-rs.github.io/docs/installation/) calls its Debian/Ubuntu repository official and configures APT with a downloaded keyring plus `signed-by` for stable amd64 and arm64 packages. It also documents official release binaries and source builds. | An upstream repository route exists for Yazi, but a supported policy must verify the key fingerprint, expiry/rotation, repository scope, and its `file` prerequisite before adopting it. |
| Starship | Starship's [official guide](https://starship.rs/guide/) documents generic release installation and `cargo install starship --locked`; its [releases](https://github.com/starship/starship/releases) publish architecture-specific assets with SHA-256 data. | Starship can be pinned from upstream source or release assets, but neither is a Noble archive dependency and the project would own update, rollback and support policy. |
| Fedora | Yazi's official installation guide documents an **unofficial** COPR path; Starship's official guide documents COPR for Fedora. | Treating either COPR as a required supported source needs an explicit third-party trust decision. |
| NixOS | Both upstream guides document Nix/nixpkgs installation paths; the repository's locked Nix build already uses those packages. | NixOS can retain its existing locked package path independently of a Debian/Fedora decision. |
| Yazi capabilities | Yazi's installation guide requires `file` and lists additional preview/search tools as optional extensions. | A source policy must distinguish the Yazi executable from the dependencies required for the promised Helm experience. |
| Redistribution licenses | Yazi's [license](https://github.com/sxyazi/yazi/blob/main/LICENSE) is MIT; the [Starship repository](https://github.com/starship/starship) declares ISC. | Both licenses permit redistribution, subject to preserving their required notices; package review must still inspect the selected release's bundled notices and dependencies. |
| Release verification | Starship's [v1.26.0 release](https://github.com/starship/starship/releases/tag/v1.26.0) shows a GitHub-verified signed tag and architecture-specific SHA-256 release assets. Yazi's [v26.5.6 release](https://github.com/sxyazi/yazi/releases/tag/v26.5.6) publishes Linux package/binary assets with SHA-256 data. | A pinned digest gives byte reproducibility after review. It does not itself authenticate maintainer identity, prove that an asset will remain available, or replace an independent signature/key policy. |
| Debian build policy | [Debian Policy §4.9](https://www.debian.org/doc/debian-policy/ch-source.html) says required `debian/rules` targets must not attempt network access to other hosts, except for the specified non-free / non-autobuild exception and loopback services started by the build. | A Debian-archive-compatible Helm package build cannot fetch Yazi or Starship from a live upstream URL. Its reviewed source or artifact must be available before the build begins. |
| Fedora source handling | Fedora's [packaging guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/) state that source code must not be downloaded from external sources during a build, only from the Fedora lookaside cache and/or Fedora Git. Its [source-control policy](https://fedoraproject.org/wiki/Package_Source_Control) describes the lookaside cache as retained, hash-addressed upstream archives. | A Fedora-guideline-compatible Helm package build likewise needs a prior, hash-verified source-intake and retention path; a live release download in the spec build is not sufficient. |

## Decision boundary

The remaining choices are normative product/distribution commitments, not facts
an agent may infer:

1. support upstream-managed repositories/assets for the two tools;
2. build and redistribute locked upstream sources as part of Helm packaging;
3. narrow the M1 distribution guarantee until target archives provide the
   tools; or
4. choose another explicitly governed source per target.

Any supported cross-target claim needs an accepted policy defining version
floors, hashes or signing verification, licenses/redistribution obligations,
update and rollback ownership, package metadata, and how a previously launched
immutable generation remains valid.  The existing issue set leaves those
choices unresolved; this record does not impose an implementation ordering.

A release URL and a checked-in SHA-256 are insufficient as the entire supply
chain policy. The policy must state which release/tag identity is reviewed,
how a signing key or provenance attestation is independently trusted, what
happens if an upstream asset disappears or is replaced, and where required
license notices are preserved. GitHub's [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
are an opt-in repository feature, not a property Helm may assume for either
upstream project.

The policy must also distinguish **intake** from **build**: a reviewed actor
may acquire an upstream source or artifact at a defined intake point, verify
its identity and digest, preserve the required notices, and retain the exact
input in a governed store. The package build then consumes that fixed input
without contacting upstream. Choosing the store, its retention owner, and
whether Helm may operate it is a product/infrastructure decision still outside
this research record. The no-public-infrastructure standing constraint rules
out silently creating a Helm-hosted service as an implementation shortcut.

**Scope of these constraints.** Debian Policy constrains required
`debian/rules` targets, with its stated non-free/non-autobuild exception;
Fedora's rule constrains packages in the Fedora build system. They do not
prohibit a separately governed intake step before package construction, nor do
they automatically govern arbitrary third-party `.deb`/RPM builds. For any
supported policy-compatible package path, however, Helm should make upstream
acquisition a reviewed, hash-verified intake operation and have the later
package build consume retained fixed inputs without a live upstream fetch.

## Observations

- [fact] Yazi documents an official Debian/Ubuntu APT repository configured with a downloaded keyring and `signed-by` #m1 #packaging
- [fact] Starship documents Ubuntu archive installation only from Ubuntu 25.04 onward #m1 #packaging
- [risk] Upstream repositories and release artifacts move package-provenance and update obligations into the Helm support contract #security #packaging
- [risk] A release checksum detects a changed artifact but does not independently establish publisher identity or retention #security #provenance
- [constraint] Debian-archive and Fedora-guideline-compatible package builds need fixed source inputs and cannot make a live upstream fetch their source of truth #packaging #provenance
- [decision-required] The package source policy must define reviewed intake, fixed-input retention, and ownership separately from package builds #m1 #packaging
- [decision-required] A supported cross-target Yazi and Starship claim needs an accepted source and lifecycle policy #mvp

## Relations

- informs [[ADR 0007 — Reuse yazi, btop and zsh+starship rather than rewrite them]]
- blocks [[Issue 134 — Decide supported M1 package sources and catalog migration for Yazi and Starship]]
- blocks [[Issue 135 — Complete exact M1 activation assets and supported-consumer probes]]
