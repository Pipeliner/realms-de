# Realm workspace source and closure provenance

This retained bundle is the source authority for Realm workspace Cargo builds
required by SPEC 0024.  It was created by controlled intake, not by a native
package recipe.

## Bound source

- Repository: `https://github.com/pipeliner/realms-de`
- Commit: `e9ccdb200e051c44dc525c046aa29c7615441215`
- Commit timestamp: `2026-09-10T16:16:44Z` (`1789057004`)
- Workspace version: `0.1.0`
- Canonical archive: `source.tar.gz`
- Canonical archive SHA-256:
  `184f927b0d60f61c42d768b8ac8fbf0b6b525a889b4492cef9f7364aa0fa94c6`

The archive was generated directly from the exact bound Git commit with
`git archive --format=tar.gz --prefix=realm-workspace/`, excluding
`packaging/tool-sources/bundles/` so the authority cannot recursively contain
an independent retained source or dependency closure. Git supplied every
archived byte from that immutable commit object; working-tree bytes were not an
input. The resulting archive is retained as the canonical source input.

## Dependency closure

Cargo vendoring used an otherwise empty, intake-local `CARGO_HOME` and the
archive root's retained `Cargo.lock`:

```sh
CARGO_HOME=<intake>/cargo-home cargo vendor --locked --versioned-dirs <intake>/vendor
```

The resulting 59 registry crates were archived as `vendor.tar.zst` by one
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
