# Realm workspace source and closure provenance

This retained bundle is the source authority for Realm workspace Cargo builds
required by SPEC 0024.  It was created by controlled intake, not by a native
package recipe.

## Bound source

- Repository: `https://github.com/pipeliner/realms-de`
- Commit: `ea9f7e7cc010006902f796828d2855cfafb15b92`
- Commit timestamp: `2026-09-27T13:20:30Z` (`1790515230`)
- Workspace version: `0.1.0`
- Canonical archive: `source.tar.gz`
- Canonical archive SHA-256:
  `a1194aa9704651dd2760160169f4adf59cc09fb73bf28aedb6ce4ae00ec562e7`

The archive was generated directly from the exact bound Git commit with
`git archive --format=tar.gz --prefix=realm-workspace/ <commit> .
':(exclude)packaging/tool-sources/bundles'`, excluding
`packaging/tool-sources/bundles/` so the authority cannot recursively contain
an independent retained source or dependency closure. Git supplied every
archived byte from that immutable commit object; working-tree bytes were not an
input. The resulting archive is retained as the canonical source input.

## Dependency closure

Cargo 1.97.1 vendoring used an otherwise empty, intake-local `CARGO_HOME` and
the archive root's retained `Cargo.lock`:

```sh
CARGO_HOME=<intake>/cargo-home cargo vendor --locked --versioned-dirs <intake>/vendor
```

The resulting 221 registry crates were archived as `vendor.tar.zst` by one
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

## CI workspace dependency-edge refresh

Commit `ea9f7e7cc010006902f796828d2855cfafb15b92` refreshed workspace dependency lists only. External package records and retained vendor, configuration and license-report bytes are unchanged and digest-verified.
- Prior lock SHA-256: `835ee92fdbaaa335c36d99e795e590011e8613dc161b9aded60e50549263657a`
- Refreshed lock SHA-256: `07a6aab2f12652213126b339939818a7a96090ce328298b6c5a080ee2ac3f5ce`
