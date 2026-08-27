//! Render every themed file in the desktop from one palette, atomically.
//!
//! `palette.toml` is the only place a colour is written down (ADR 0005). This
//! crate is what makes that affordable: it turns the palette into a GTK
//! stylesheet, a Qt colour scheme, a terminal's ANSI table and four TUI configs,
//! and swaps them all in as one step so no user ever sees a new terminal palette
//! against an old GTK stylesheet.
//!
//! Three things carry the design, and all three are specified in
//! `docs/specs/0002-theme-pipeline.md`:
//!
//! 1. **Templates address the derived palette.** [`helm_core::Palette::derived`]
//!    folds `contrast` in exactly once, here, at this boundary. No template
//!    applies contrast itself and nothing downstream filters (ADR 0006).
//! 2. **An unknown placeholder is a hard error.** A silently blank colour is the
//!    bug the whole design exists to prevent, so [`render()`] names the offending
//!    placeholder and aborts the apply before anything is written.
//! 3. **Application is atomic.** Everything is rendered to memory, written to
//!    a unique no-follow `.<final>.helm-tmp.<pid>.<sequence>` staging sibling,
//!    `fsync`ed, `rename(2)`d into place, and only then are reloads fanned out —
//!    once each.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod reload;
pub mod render;
pub mod template;
pub mod theme;

pub use reload::{Reloader, SystemReloader};
pub use render::render;
pub use template::{templates, Reload, Template};
pub use theme::{
    apply, apply_with, diff, lint, load_palette, Applied, Change, HueSeparation, LintReport,
    SHIPPED_PALETTE, USER_PALETTE,
};

use std::path::PathBuf;

/// Errors produced while rendering or applying a theme.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A placeholder named something the palette does not have.
    ///
    /// Deliberately fatal: rendering it as an empty string would produce a file
    /// that looks fine and displays wrong.
    #[error("{template}: unknown placeholder {{{{ {placeholder} }}}}")]
    UnknownPlaceholder {
        /// Template id the placeholder appeared in.
        template: String,
        /// The placeholder body, verbatim.
        placeholder: String,
    },
    /// A placeholder was opened with `{{` and never closed.
    #[error("{template}: unterminated placeholder near byte {offset}")]
    UnterminatedPlaceholder {
        /// Template id.
        template: String,
        /// Byte offset of the opening braces.
        offset: usize,
    },
    /// The palette failed its own readability floors.
    ///
    /// Carries every finding, not the first, so one round of fixes is enough.
    #[error("palette refused: {}", .0.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    PaletteRefused(Vec<helm_core::palette::Finding>),
    /// A file could not be read, written or renamed.
    #[error("{path}: {source}")]
    Io {
        /// The path being operated on.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A reload mechanism ran and failed.
    #[error("reload failed: {0}")]
    Reload(String),
    /// A template target could escape or collide within the configuration root.
    #[error("unsafe template target {target}: {reason}")]
    UnsafeTarget {
        /// Target supplied by the template.
        target: PathBuf,
        /// Why it is unsafe.
        reason: &'static str,
    },
    /// The palette itself did not parse or validate.
    #[error(transparent)]
    Core(#[from] helm_core::Error),
}

/// Convenience alias used throughout this crate.
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Attach a path to an [`std::io::Error`], because "No such file or
    /// directory" without a path is the least useful message in computing.
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
