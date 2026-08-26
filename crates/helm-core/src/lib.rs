//! Shared contracts for the helm desktop environment.
//!
//! Everything in this crate is deliberately free of I/O, timers and toolkit
//! dependencies. The compositor, the bar, the launcher and the CLI all agree on
//! the types defined here, which keeps the interesting logic — the ledger, the
//! layout projection and the palette derivation — unit-testable without a
//! Wayland socket.
//!
//! Three ideas carry the whole design:
//!
//! 1. **The ledger is the truth.** [`ledger::Ledger`] is an ordered list of
//!    windows per orbit. Nothing else stores window order.
//! 2. **Layouts are pure projections.** [`layout::project`] is
//!    `fn(&Ledger, Layout, Workarea) -> Vec<Geometry>` with no interior
//!    mutability, so undo is "restore an older ledger" and nothing more.
//! 3. **The palette is a file.** [`palette::Palette`] is parsed once from
//!    `palette.toml`; every themed surface is rendered from it.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod color;
pub mod glyphs;
pub mod ipc;
pub mod keys;
pub mod layout;
pub mod ledger;
pub mod palette;
pub mod state;

pub use ledger::{Ledger, OrbitId, WinId};
pub use palette::Palette;

/// Errors produced while loading or validating helm configuration.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A colour literal was not a `#rrggbb` string.
    #[error("invalid colour literal {0:?}: expected #rrggbb")]
    BadColor(String),
    /// `palette.toml` did not parse.
    #[error("palette parse error: {0}")]
    PaletteParse(#[from] toml::de::Error),
    /// A palette value was outside its permitted range.
    #[error("palette value out of range: {0}")]
    PaletteRange(String),
    /// An IPC frame did not decode.
    #[error("ipc decode error: {0}")]
    Ipc(#[from] serde_json::Error),
}

/// Convenience alias used throughout helm.
pub type Result<T> = std::result::Result<T, Error>;
