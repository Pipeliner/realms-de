# Environment and toolchain facts

Durable facts about building and testing this repo. Session-local noise does not
belong here.

---

## Toolchain

- Rust stable via rustup; `rust-toolchain.toml` pins the channel and requests
  `rustfmt` + `clippy`. MSRV is declared as 1.82 in the workspace manifest.
- `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
  is the pre-commit trio. CI runs exactly this; do not let CI find it first.
- Dev profile uses `opt-level = 1` for the workspace and `2` for dependencies.
  A DE that takes a minute to rebuild does not get iterated on.

## Crate choices, verified available

`smithay-client-toolkit` 0.21, `tiny-skia` 0.12, `cosmic-text` 0.19,
`nucleo` 0.5, `ratatui` 0.30, `sysinfo` 0.38. All pure Rust or dlopen-based —
no system Wayland headers needed to build the clients.

## Test conventions

- Layout and colour maths get **invariant** tests (exact tiling at many sizes,
  hue preserved across the whole contrast range), not example tests. Example
  tests pass while the algorithm is subtly wrong.
- `palette.toml` is `include_str!`-ed into `helm-core`'s tests, so the shipped
  palette is linted by `cargo test`. Editing the palette badly fails the build.

## Sandbox notes (agent sessions)

- `crates.io` is reachable directly; general HTTPS goes through a policy proxy.
  A 403 from the proxy is an org egress policy, not a network fault — report it
  rather than routing around it.
- No Wayland display, no GPU and no `nix` binary in the agent container. Anything
  needing a live session must be exercised in CI or on real hardware, and the
  issue that needs it carries `needs-human`.
