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

/// How a template's Helm-owned output reaches its consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activation {
    /// An absent user-owned file may atomically receive a minimal shim.
    Shim {
        /// Path to the user-owned activation file, relative to `$XDG_CONFIG_HOME`.
        user_path: PathBuf,
        /// The complete contents of the shim.
        contents: ShimContents,
    },
    /// Helm must not create a user configuration; the launcher or user follows
    /// this target-specific remedy instead.
    Manual(ManualActivation),
}

/// The contents of an atomically published first-run shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimContents {
    /// A complete literal shim, including its terminal newline.
    Literal(&'static str),
    /// A foot include whose required absolute path is derived at apply time.
    FootInclude,
}

/// A consumer whose generated output requires an explicit launch/config step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualActivation {
    /// Yazi reads the generated directory when `YAZI_CONFIG_HOME` selects it.
    Yazi,
    /// btop requires its custom theme directory and a user `color_theme` value.
    Btop,
    /// Starship reads its generated configuration through `STARSHIP_CONFIG`.
    Starship,
    /// Helm launches fuzzel with its generated configuration path.
    Fuzzel,
    /// qt6ct requires its platform-theme environment and appearance settings.
    Qt6ct,
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
/// Targets land below `$XDG_CONFIG_HOME/helm/generated`; each declares the
/// explicit mechanism that consumes it without granting Helm ownership of an
/// existing user configuration.
pub fn templates() -> Vec<Template> {
    vec![
        Template {
            id: "gtk4",
            source: include_str!("../../../configs/templates/gtk4.css"),
            target: PathBuf::from("helm/generated/gtk-4.0/helm.css"),
            activation: Some(Activation::Shim {
                user_path: PathBuf::from("gtk-4.0/gtk.css"),
                contents: ShimContents::Literal(
                    "@import url(\"../helm/generated/gtk-4.0/helm.css\");\n",
                ),
            }),
            reload: gtk_restyle(),
        },
        Template {
            id: "gtk3",
            source: include_str!("../../../configs/templates/gtk3.css"),
            target: PathBuf::from("helm/generated/gtk-3.0/helm.css"),
            activation: Some(Activation::Shim {
                user_path: PathBuf::from("gtk-3.0/gtk.css"),
                contents: ShimContents::Literal(
                    "@import url(\"../helm/generated/gtk-3.0/helm.css\");\n",
                ),
            }),
            reload: gtk_restyle(),
        },
        Template {
            id: "foot",
            source: include_str!("../../../configs/templates/foot.ini"),
            target: PathBuf::from("helm/generated/foot/foot.ini"),
            activation: Some(Activation::Shim {
                user_path: PathBuf::from("foot/foot.ini"),
                contents: ShimContents::FootInclude,
            }),
            reload: Reload::Signal {
                process: "foot",
                signal: SIGUSR1,
            },
        },
        Template {
            id: "yazi",
            source: include_str!("../../../configs/templates/yazi-theme.toml"),
            target: PathBuf::from("helm/generated/yazi/theme.toml"),
            activation: Some(Activation::Manual(ManualActivation::Yazi)),
            reload: Reload::None,
        },
        Template {
            id: "btop",
            source: include_str!("../../../configs/templates/btop.theme"),
            target: PathBuf::from("helm/generated/btop/themes/helm.theme"),
            activation: Some(Activation::Manual(ManualActivation::Btop)),
            reload: Reload::None,
        },
        Template {
            id: "starship",
            source: include_str!("../../../configs/templates/starship.toml"),
            target: PathBuf::from("helm/generated/starship/starship.toml"),
            activation: Some(Activation::Manual(ManualActivation::Starship)),
            reload: Reload::None,
        },
        Template {
            id: "fuzzel",
            source: include_str!("../../../configs/templates/fuzzel.ini"),
            target: PathBuf::from("helm/generated/fuzzel/fuzzel.ini"),
            activation: Some(Activation::Manual(ManualActivation::Fuzzel)),
            reload: Reload::None,
        },
        Template {
            id: "qt6ct",
            source: include_str!("../../../configs/templates/qt6ct-colors.conf"),
            target: PathBuf::from("helm/generated/qt6ct/colors/helm.conf"),
            activation: Some(Activation::Manual(ManualActivation::Qt6ct)),
            reload: Reload::None,
        },
    ]
}
