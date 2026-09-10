# Task 5 report: package, document, and verify `realmctl`

## Delivery

- Nix now installs the built `target/release/realmctl` as `$out/bin/realmctl`.
- Debian's existing `packaging/debian/rules` loop already conditionally installs
  every built binary, including `realmctl`; no nonexistent `realm.install` file
  was added. Fedora's existing generated binary list does the same. Their
  comments and package descriptions now state that fact accurately.
- The install guide now lists `realmctl theme apply`, `theme lint`, and `theme
  diff` as installed, while preserving the accurate pre-alpha warning that the
  desktop itself still lacks `realm-wm` and `realm-bar`. README contained no
  stale claim that realmctl was absent and needs no change.
- SPEC 0002 and SPEC 0006 now name the exact present CLI integration or unit
  tests for the delivered lint, diff, and apply rows.

## Test-first evidence

1. Before the Nix change, `rg -n 'realmctl' packaging/nix/package.nix` returned
   no result, proving that lane had no installation record.
2. `cargo build -p realm-ctl --release --locked` then produced executable
   `target/release/realmctl`.
3. Inspection of the existing Debian and Fedora loops confirmed each already
   consumes that release path and installs `realmctl` when it is executable.
4. The only new installation statement is therefore Nix's explicit
   `install -Dm755 target/release/realmctl $out/bin/realmctl`.

## Final verification

- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` —
  passed.
- `cargo test --workspace --all-features --locked` — passed: 47
  realm-agent-sdd integration tests, 54 realm-core tests, 2 realmctl unit tests,
  10 realmctl integration tests, and 118 realm-theme tests; doc-tests passed.
- `cargo build -p realm-ctl --release --locked` — passed and
  `target/release/realmctl` is executable.
- `bash packaging/fedora/check-projections.sh` — passed: Fedora 44 projections
  match SPEC 0009.
- Text checks confirm Nix's explicit `realmctl` installation record and the
  existing Debian/Fedora conditional installation loops.
- A real `nix build .#realm --no-link` was attempted with the Nix daemon, but
  evaluation cannot run on this host: installed Nix is 2.16.1 while the pinned
  nixpkgs requires Nix 2.18 or newer. This is an environment toolchain block,
  not a package evaluation or build failure.
- `git diff --check` — passed.
