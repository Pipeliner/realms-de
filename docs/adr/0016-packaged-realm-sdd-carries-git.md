# ADR 0016 — The Nix-installed `realm-sdd` carries Git in its own runtime closure

- **Status:** Accepted (2026-08-29)
- **Deciders:** realm maintainers, repo owner
- **Supersedes / Superseded by:** Supplements [ADR 0014](0014-local-agent-sdd-pilot-governance.md); it does not change #144's check-time dependency decision

## Context

`realm-sdd` is the local, read-only validator for the metadata-only agent-SDD
pilot. Its successful assessment path queries the current worktree through Git:
it resolves commits, reads trees, identifies the record-carrier parent, and
checks ancestry and cleanliness. The Nix `realm` package installs that binary,
but previously wrapped only `realm-session`. Adding `pkgs.git` for #144 made the
workspace's Nix **test** sandbox pass; it did not make Git available when an
installed `realm-sdd` is run.

An installed validator whose result changes because a host happened to expose
`git` on `PATH` is an untruthful package interface. The unified `realm` output
will gain a Git reference because it contains `realm-sdd`; Git is nevertheless
not a desktop-session dependency, and `realm-session` must not receive it in
its own wrapper search path or in `support.wrapperRuntime`.

## Decision

1. The Nix `realm` package SHALL wrap only `$out/bin/realm-sdd` with the pinned
   `pkgs.git` executable path. Git is therefore available to the installed
   validator independently of the caller's inherited `PATH`.
2. This promise is limited to the Nix-installed binary. It does not bundle a
   repository, create commits, access the network, or make a non-Nix source
   build work without Git. A real local Git worktree and its objects remain
   required by [SPEC 0008](../specs/0008-agent-sdd-pilot-governance.md).
3. `realm-session` and `support.wrapperRuntime` SHALL NOT gain Git because of
   this decision. The package output closure gains Git only because it also
   contains `realm-sdd`. #144 remains the separate check-time declaration.
4. An executable package-level regression SHALL construct a valid local Git
   record-carrier fixture, invoke the installed wrapper with no usable inherited
   `PATH`, require the canonical successful `realm-sdd gate` result, and assert
   that the generated `realm-session` wrapper does not include Git's store bin
   directory.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Declare host Git as a prerequisite | Smaller Nix closure and conventional for source tools | The installed Nix package would depend on incidental `PATH`; the validator would currently report misleading object/freshness failures rather than a stated dependency boundary |
| Add Git to the session wrapper runtime | One shared wrapper declaration | Git is unrelated to session startup and would add it to the desktop wrapper search path for an internal validator |
| Treat Git as a build-only input | Already fixed #144's sandbox failure | It does not make the installed binary's substantive operations work |

## Consequences

### Good

- The Nix package has one truthful, tested runtime contract for `realm-sdd`.
- The `realm-session` wrapper search path stays limited to desktop dependencies.
- The regression tests the installed binary instead of inferring its closure
  from derivation inputs or wrapper text.

### Bad

- The Nix output gains a Git runtime reference for `realm-sdd`.
- A package-level fixture has to create a minimal disposable Git worktree.

### Neutral

- The validator remains read-only and metadata-only; Git availability does not
  promote records, change GitHub state, or add automatic capture.

## Reversal

Low. Remove the `realm-sdd` wrapper and its package regression, then amend
[SPEC 0010](../specs/0010-packaged-realm-sdd-git-runtime.md) with a replacement
supported interface. Reconsider only if `realm-sdd` stops querying Git or a
separate documented host-tool contract gains a safe diagnostic and equivalent
coverage.

## Guard

*Implemented (#146):* `checks.<system>.realm-sdd-git-runtime` creates a valid
record-carrier fixture and runs the installed `realm-sdd` under an empty caller
`PATH`. It exits zero with canonical pass JSON; removing the wrapper or falling
back to host `PATH` makes the guard fail. The existing CI runs this check in
both its full-KVM and no-KVM fallback paths.

## Needs a human

None. The repository already defines a local Git-backed pilot, and isolating
Git to that installed validator is the smallest evidence-backed package
boundary.
