//! Render a complete desktop theme from one palette and publish it as a sealed
//! immutable generation.
//!
//! `palette.toml` is the only place a colour is written down (ADR 0005). This
//! crate is what makes that affordable: it turns the palette into a GTK
//! stylesheet, a Qt colour scheme, a terminal's ANSI table and four TUI configs,
//! then binds the complete normalized output set to one validated generation
//! for future launches (ADR 0017 and SPEC 0011).
//!
//! Four things carry the supported contract, specified in
//! `docs/specs/0002-theme-pipeline.md` and
//! `docs/specs/0011-theme-activation-generations.md`:
//!
//! 1. **Templates address the derived palette.** [`helm_core::Palette::derived`]
//!    folds `contrast` in exactly once, here, at this boundary. No template
//!    applies contrast itself and nothing downstream filters (ADR 0006).
//! 2. **An unknown placeholder is a hard error.** A silently blank colour is the
//!    bug the whole design exists to prevent, so [`render()`] names the offending
//!    placeholder and aborts before a generation is published.
//! 3. **Application is generation-only.** One captured input set is rendered,
//!    sealed and durably selected. The supported path does not compare or write
//!    mutable target files and does not return written/unchanged/reloaded lists.
//!    A pointer switch affects future launches only and never reloads, signals,
//!    commands or notifies an existing process.
//! 4. **Diff is read-only and generation-aware.** Candidate normalized outputs
//!    are compared with the manifest-listed bytes of a fully validated current
//!    generation as added, removed or byte-different paths. Diff performs no
//!    control initialization, recovery, lease, publication or output write.
//!
//! The mutable writer and reload modules remain implementation history while
//! callers migrate; they are not an alternative supported activation contract.
//! No-op optimization, future live upgrade and wire compatibility are outside
//! this crate-level contract.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod generation;
pub mod reload;
pub mod render;
pub mod template;
pub mod theme;

pub use reload::{Reloader, SystemReloader};
pub use render::render;
pub use template::{templates, Reload, Template};
pub use theme::{
    apply, apply_with_snapshot, diff, diff_with_snapshot, lint, load_palette, HueSeparation,
    LintReport, ThemeOutputChange, ThemeSnapshot, SHIPPED_PALETTE, USER_PALETTE,
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
    /// A generation input, bootstrap or publication operation was refused.
    #[error("theme generation: {0}")]
    Generation(String),
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
