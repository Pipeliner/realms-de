//! The shipped template set and the shape of a template.

use std::path::PathBuf;

/// `SIGUSR1`, the signal foot re-reads its configuration on.
const SIGUSR1: i32 = rustix::process::Signal::USR1.as_raw();

/// A file helm generates from the palette.
pub struct Template {
    /// Stable id, e.g. `"gtk4"`, `"foot"`, `"yazi"`.
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Where the rendered file lands, relative to `$XDG_CONFIG_HOME`.
    pub target: PathBuf,
    /// User-owned file that activates this template, when the program needs one.
    pub activation: Option<Activation>,
    /// How live consumers are told to re-read it.
    pub reload: Reload,
}

/// The user-owned file and exact import that activate a generated template.
///
/// This is metadata only. Applying it is intentionally a separate concern so
/// the owned-output writer never gains authority over an existing user file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    /// Path to the user-owned activation file, relative to `$XDG_CONFIG_HOME`.
    pub user_path: PathBuf,
    /// The complete import line a user file needs, including its newline.
    pub import: &'static str,
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

/// Nudge GTK into re-reading its stylesheet.
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

/// The templates helm ships.
///
/// Targets are relative to `$XDG_CONFIG_HOME`. GTK's user-owned stylesheet is
/// declared as activation metadata; the writer handles that metadata separately
/// from the Helm-owned generated output.
pub fn templates() -> Vec<Template> {
    vec![
        Template {
            id: "gtk4",
            source: include_str!("../../../configs/templates/gtk4.css"),
            target: PathBuf::from("gtk-4.0/helm.css"),
            activation: Some(Activation {
                user_path: PathBuf::from("gtk-4.0/gtk.css"),
                import: "@import url(\"../helm/generated/gtk-4.0/helm.css\");\n",
            }),
            reload: gtk_restyle(),
        },
        Template {
            id: "gtk3",
            source: include_str!("../../../configs/templates/gtk3.css"),
            target: PathBuf::from("gtk-3.0/helm.css"),
            activation: Some(Activation {
                user_path: PathBuf::from("gtk-3.0/gtk.css"),
                import: "@import url(\"../helm/generated/gtk-3.0/helm.css\");\n",
            }),
            reload: gtk_restyle(),
        },
        Template {
            id: "foot",
            source: include_str!("../../../configs/templates/foot.ini"),
            target: PathBuf::from("foot/foot.ini"),
            activation: None,
            reload: Reload::Signal {
                process: "foot",
                signal: SIGUSR1,
            },
        },
        Template {
            id: "yazi",
            source: include_str!("../../../configs/templates/yazi-theme.toml"),
            target: PathBuf::from("yazi/theme.toml"),
            activation: None,
            reload: Reload::None,
        },
        Template {
            id: "btop",
            source: include_str!("../../../configs/templates/btop.theme"),
            target: PathBuf::from("btop/themes/helm.theme"),
            activation: None,
            reload: Reload::None,
        },
        Template {
            id: "starship",
            source: include_str!("../../../configs/templates/starship.toml"),
            target: PathBuf::from("starship.toml"),
            activation: None,
            reload: Reload::None,
        },
        Template {
            id: "fuzzel",
            source: include_str!("../../../configs/templates/fuzzel.ini"),
            target: PathBuf::from("fuzzel/fuzzel.ini"),
            activation: None,
            reload: Reload::None,
        },
        Template {
            id: "qt6ct",
            source: include_str!("../../../configs/templates/qt6ct-colors.conf"),
            target: PathBuf::from("qt6ct/colors/helm.conf"),
            activation: None,
            reload: Reload::None,
        },
    ]
}
