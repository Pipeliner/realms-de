# Realm workspace source and closure provenance

This retained bundle is the source authority for Realm workspace Cargo builds
required by SPEC 0024.  It was created by controlled intake, not by a native
package recipe.

## Bound source

- Repository: `https://github.com/pipeliner/realms-de`
- Commit: `2cfdc7100f6c87f7185a60880b58af215bd91cfe`
- Commit timestamp: `2026-09-13T07:44:45Z` (`1789285485`)
- Workspace version: `0.1.0`
- Canonical archive: `source.tar.gz`
- Canonical archive SHA-256:
  `a2c10d430fb0050db18b306bb04554301a746d5438738570928db1786bda0530`

The archive was generated directly from the exact bound Git commit with
`git archive --format=tar.gz --prefix=realm-workspace/ <commit> .
':(exclude)packaging/tool-sources/bundles'`, excluding
`packaging/tool-sources/bundles/` so the authority cannot recursively contain
an independent retained source or dependency closure. Git supplied every
archived byte from that immutable commit object; working-tree bytes were not an
input. The resulting archive is retained as the canonical source input.

## Dependency closure

The dependency archive is retained unchanged from the controlled intake at
`0cfb3a408f06dce5f397c8ebbd589aefd4ea1293`. Comparing that commit's lockfile with
the new source shows only two added dependency edges on the workspace's
`realm-session` package: `realm-theme` and the already-retained `tempfile`.
No registry package identity, version, checksum or registry dependency changed.
The copied lockfile matches the new source archive. The retained vendor and
license hashes are unchanged and the bundle checks revalidate their closure.

That original Cargo 1.97.1 vendoring used an otherwise empty, intake-local
`CARGO_HOME` and its source archive's `Cargo.lock`:

```sh
CARGO_HOME=<intake>/cargo-home cargo vendor --locked --versioned-dirs <intake>/vendor
```

The resulting 176 registry crates were archived as `vendor.tar.zst` by one
sorted tar stream with epoch mtime, numeric uid/gid zero, and Zstandard level
3 compression:

```sh
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C <intake>/archive-input -cf - vendor | zstd -3 -q -f -o vendor.tar.zst
```

`licenses.tsv` is derived from the `[package]` `name`, `version`, and
`license` entries in every vendored crate's retained `Cargo.toml`; its fourth
column points to that exact retained metadata file.  `config.toml` selects the
staged `vendor` directory for crates.io source replacement.

The bundle record binds every final artifact by SHA-256.  Native package paths
must consume those retained bytes and must not regenerate the archive, invoke
Git, or fetch dependencies.
