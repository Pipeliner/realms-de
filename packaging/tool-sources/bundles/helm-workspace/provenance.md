# Helm workspace source and closure provenance

This retained bundle is the source authority for Helm workspace Cargo builds
required by SPEC 0024.  It was created by controlled intake, not by a native
package recipe.

## Bound source

- Repository: `https://github.com/pipeliner/realms-de`
- Commit: `368af1e7bc0e61b5c04d14d9233f1690afcf564d`
- Commit timestamp: `2026-09-09T19:30:56Z` (`1788982256`)
- Workspace version: `0.1.0`
- Canonical archive: `source.tar.gz`
- Canonical archive SHA-256:
  `b3abfd5c38aecb1ea7254a5552f3c289759c12affe2ffad9782e63dfbb32db47`

The intake cloned a clean checkout, detached it at the bound commit, confirmed
that `HEAD` equalled that commit and that the checkout had no status output,
then ran one `git archive --format=tar.gz --prefix=helm-workspace/` command at
that commit, excluding `packaging/tool-sources/bundles/` so the authority
cannot recursively contain any independent retained source or dependency
closure. The resulting archive is retained as the canonical source input; the
working checkout is neither an input nor a substitute for the archive.

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
