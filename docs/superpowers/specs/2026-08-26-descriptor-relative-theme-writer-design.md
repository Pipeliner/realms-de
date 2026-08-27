# Descriptor-relative theme writer design

## Context

Issue #110 found that the theme writer validated pathnames and then returned to
path-based directory creation, staging, cleanup, and rename. A same-user
process can replace a checked parent directory with a symlink between those
operations. `O_EXCL | O_NOFOLLOW` protects one temporary pathname, not the
parent lookup or final rename.

This design implements the accepted output-containment amendment in
[[SPEC 0002 — Theme pipeline]]. It is limited to the `apply` writer. Palette
loading remains covered by A13; cross-file all-or-nothing publication remains
#22's concern.

## Decision

`apply_with` opens the configuration root once with `openat(CWD, root,
RDONLY | DIRECTORY | NOFOLLOW | CLOEXEC)`. For every validated target it opens
each parent relative to that descriptor with `DIRECTORY | NOFOLLOW`; a missing
parent is created with `mkdirat` against the held parent descriptor and then
opened with the same flags.

The final component is then used only relative to the held parent descriptor:
comparison uses `openat(..., RDONLY | NOFOLLOW)`, staging uses
`CREATE | EXCL | NOFOLLOW | CLOEXEC`, commit uses `renameat`, cleanup uses
`unlinkat`, and durability uses `fsync` on both the temporary file and parent
directory. A staged record owns the parent `OwnedFd` plus temporary and final
basenames until commit or cleanup.

Temporary files use `.<final>.helm-tmp.<pid>.<monotonic-sequence>`. A bounded
retry on `AlreadyExists` makes a stale predictable temporary name harmless
instead of an availability denial. No cleanup reconstructs an absolute path.

## Security property

Once a directory descriptor is held, every filesystem mutation for that output
is relative to it. Replacing the original pathname with a symlink can prevent a
visible update, but cannot redirect a read, write, rename, or unlink to the
symlink destination. Individual `renameat` operations stay atomic; multi-file
all-or-nothing publication is explicitly retained for #22.

## Tests

The staging-link test asserts an attacker-planted predictable link remains
untouched while apply stages through another name. A deterministic replacement
test opens a parent descriptor, renames that path away, replaces it with a
victim symlink, and asserts staging/commit never touches the victim.

## Relations

- implements [[SPEC 0002 — Theme pipeline]]
- addresses [[GitHub issue 110]]
- complements [[GitHub issue 22]]
