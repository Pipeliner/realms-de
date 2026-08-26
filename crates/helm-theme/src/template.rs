//! The shipped template set and the shape of a template.

use std::path::PathBuf;

/// A file helm generates from the palette.
pub struct Template {
    /// Stable id, e.g. `"gtk4"`, `"foot"`, `"yazi"`.
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Where the rendered file lands, relative to `$XDG_CONFIG_HOME`.
    pub target: PathBuf,
    /// How live consumers are told to re-read it.
    pub reload: Reload,
}

/// How a themed program is told the theme changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// Nothing to do: read at next start.
    None,
    /// Send a signal to every process matching this name.
    Signal {
        /// Process name as it appears in `/proc/<pid>/comm`.
        process: &'static str,
        /// Signal number.
        signal: i32,
    },
    /// Run a command, e.g. `gsettings set ...`.
    Command(Vec<String>),
    /// Notify helm's own clients over the control socket.
    HelmClients,
}

/// The templates helm ships.
pub fn templates() -> Vec<Template> {
    unimplemented!()
}
