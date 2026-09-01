# Helm workspace source and closure provenance

This retained bundle is the source authority for Helm workspace Cargo builds
required by SPEC 0024.  It was created by controlled intake, not by a native
package recipe.

## Bound source

- Repository: `https://github.com/pipeliner/realms-de`
- Commit: `50b5847e85d300fb0dd223afcb65af9c69a19ad8`
- Commit timestamp: `2026-08-31T20:15:32Z` (`1788207332`)
- Workspace version: `0.1.0`
- Canonical archive: `source.tar.gz`
- Canonical archive SHA-256:
  `3672d2e416eb8c8de8e8490703f426cfacaa08570cfd47f3ef5fa81c831325ef`

The intake cloned a clean checkout, detached it at the bound commit, confirmed
that `HEAD` equalled that commit and that the checkout had no status output,
then ran one `git archive --format=tar.gz --prefix=helm-workspace/` command at
that commit.  The resulting archive is retained as the canonical source input;
the working checkout is neither an input nor a substitute for the archive.

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
