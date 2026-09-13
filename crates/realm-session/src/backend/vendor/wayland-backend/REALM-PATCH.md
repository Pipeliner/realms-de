# Realm bounded-client adaptation

This directory vendors the complete crates.io source for
`wayland-backend` 0.3.17 (MIT), whose published crate checksum is
`38a91b4eaddff87b1cd1074985e3713da4af2c49742d1b356b2c01670a67a078`.
The retained `.cargo_vcs_info.json` identifies upstream
`smithay/wayland-rs` commit `72f7fe0d9e99cb720ed57ade0c3077aec533b6ab`,
path `wayland-backend`. The unmodified upstream license is retained as
`LICENSE.txt`.

Realm changes only the pure-Rust client path. `client_api.rs`,
`rs/client_impl/mod.rs`, and `rs/socket.rs` add the exclusive bounded-read,
single-message dispatch, and single-send flush primitives specified by Realm
ADR 0021. `Cargo.toml` adds an empty workspace declaration so the retained
crate can be inspected independently beneath Realm's workspace, plus a
`realm_test` feature used only to expose the structural no-retry regression to
Realm's integration test. No wire format, signature, object-map, or callback
implementation is replaced: the adaptation continues to use the upstream
parser and protocol state.

The workspace pins this directory through `[patch.crates-io]`; the generated
River bindings continue to use exactly `wayland-client` 0.31.15. Do not update
or regenerate this fork without re-recording the source version, published
checksum, upstream revision, license, focused real-socket tests, and the patch
review in this file.
