//! The shipped template catalogue and its legacy mutable metadata.
//!
//! Supported generation apply treats targets as normalized paths inside one
//! sealed generation and binds reload metadata into the catalogue digest without
//! executing it. The mutable writer's target and reload interpretation remains
//! only for implementation migration and historical tests.

use std::path::PathBuf;

/// `SIGUSR1`, the signal foot re-reads its configuration on.
const SIGUSR1: i32 = rustix::process::Signal::USR1.as_raw();

/// One generated output and its catalogue metadata.
#[derive(Debug)]
pub struct Template {
    /// Stable id, e.g. `"gtk4"`, `"foot"`, `"yazi"`.
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Output path.
    ///
    /// The legacy writer interprets it relative to its caller-supplied root;
    /// supported apply normalizes it inside the staged generation.
    pub target: PathBuf,
    /// Canonical reload metadata.
    ///
    /// The legacy writer executes it. Supported apply only digests it and never
    /// reloads a process on pointer switch.
    pub reload: Reload,
}

/// Canonical reload metadata retained for catalogue identity.
///
/// Only the legacy mutable writer executes these variants. A future live
/// upgrade requires a separately specified generation-aware owned-process
/// protocol; supported pointer publication never executes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reload {
    /// Catalogue declares that the consumer reads at next start.
    None,
    /// Legacy signal metadata for a named process.
    Signal {
        /// Process name as it appears in `/proc/<pid>/comm`.
        process: &'static str,
        /// Signal number.
        signal: i32,
    },
    /// Legacy command metadata, e.g. `gsettings set ...`.
    Command(Vec<String>),
    /// Metadata identifying Helm-owned clients.
    HelmClients,
}

/// Build GTK's legacy reload metadata.
///
/// GTK watches the settings it gets over the settings portal and rebuilds its
/// style cascade when one changes; writing the theme name it already has is the
/// cheapest way to say "look again". Both GTK templates share this, which is
/// why the fan-out deduplicates.
fn gtk_restyle() -> Reload {
    Reload::Command(
        [
            "gsettings",
            "set",
            "org.gnome.desktop.interface",
            "gtk-theme",
            "Adwaita-dark",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect(),
    )
}

/// The template catalogue Helm ships.
///
/// The legacy mutable writer interprets targets relative to its supplied root.
/// Supported apply treats the same values as normalized output paths within a
/// sealed generation. Reload fields remain part of the canonical catalogue
/// digest but are not executed by supported apply.
pub fn templates() -> Vec<Template> {
    vec![
        Template {
            id: "gtk4",
            source: include_str!("../../../configs/templates/gtk4.css"),
            target: PathBuf::from("gtk-4.0/helm.css"),
            reload: gtk_restyle(),
        },
        Template {
            id: "gtk3",
            source: include_str!("../../../configs/templates/gtk3.css"),
            target: PathBuf::from("gtk-3.0/helm.css"),
            reload: gtk_restyle(),
        },
        Template {
            id: "foot",
            source: include_str!("../../../configs/templates/foot.ini"),
            target: PathBuf::from("foot/foot.ini"),
            reload: Reload::Signal {
                process: "foot",
                signal: SIGUSR1,
            },
        },
        Template {
            id: "yazi",
            source: include_str!("../../../configs/templates/yazi-theme.toml"),
            target: PathBuf::from("yazi/theme.toml"),
            reload: Reload::None,
        },
        Template {
            id: "btop",
            source: include_str!("../../../configs/templates/btop.theme"),
            target: PathBuf::from("btop/themes/helm.theme"),
            reload: Reload::None,
        },
        Template {
            id: "starship",
            source: include_str!("../../../configs/templates/starship.toml"),
            target: PathBuf::from("starship.toml"),
            reload: Reload::None,
        },
        Template {
            id: "fuzzel",
            source: include_str!("../../../configs/templates/fuzzel.ini"),
            target: PathBuf::from("fuzzel/fuzzel.ini"),
            reload: Reload::None,
        },
        Template {
            id: "qt6ct",
            source: include_str!("../../../configs/templates/qt6ct-colors.conf"),
            target: PathBuf::from("qt6ct/colors/helm.conf"),
            reload: Reload::None,
        },
    ]
}
