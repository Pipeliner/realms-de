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
    /// Metadata identifying Realm-owned clients.
    RealmClients,
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

/// The template catalogue Realm ships.
///
/// The legacy mutable writer interprets targets relative to its supplied root.
/// Supported apply treats the same values as normalized output paths within a
/// sealed generation. Reload fields remain part of the canonical catalogue
/// digest but are not executed by supported apply.
pub fn templates() -> Vec<Template> {
    vec![
        Template {
            id: "gtk4",
            source: ::core::include_str!("../../../configs/templates/gtk4.css"),
            target: PathBuf::from("gtk-4.0/realm.css"),
            reload: gtk_restyle(),
        },
        Template {
            id: "gtk3",
            source: ::core::include_str!("../../../configs/templates/gtk3.css"),
            target: PathBuf::from("gtk-3.0/realm.css"),
            reload: gtk_restyle(),
        },
        Template {
            id: "gtk4-profile",
            source: ::core::include_str!("../../../configs/templates/gtk4.css"),
            target: PathBuf::from("share/themes/realm/gtk-4.0/gtk.css"),
            reload: Reload::None,
        },
        Template {
            id: "gtk3-profile",
            source: ::core::include_str!("../../../configs/templates/gtk3.css"),
            target: PathBuf::from("share/themes/realm/gtk-3.0/gtk.css"),
            reload: Reload::None,
        },
        Template {
            id: "foot",
            source: ::core::include_str!("../../../configs/templates/foot.ini"),
            target: PathBuf::from("foot/foot.ini"),
            reload: Reload::Signal {
                process: "foot",
                signal: SIGUSR1,
            },
        },
        Template {
            id: "zsh-profile",
            source: ::core::include_str!("../../../configs/templates/zshrc"),
            target: PathBuf::from("zsh/.zshrc"),
            reload: Reload::None,
        },
        Template {
            id: "yazi-config",
            source: ::core::include_str!("../../../configs/templates/yazi.toml"),
            target: PathBuf::from("yazi/yazi.toml"),
            reload: Reload::None,
        },
        Template {
            id: "yazi-keymap",
            source: ::core::include_str!("../../../configs/templates/yazi-keymap.toml"),
            target: PathBuf::from("yazi/keymap.toml"),
            reload: Reload::None,
        },
        Template {
            id: "yazi",
            source: ::core::include_str!("../../../configs/templates/yazi-theme.toml"),
            target: PathBuf::from("yazi/theme.toml"),
            reload: Reload::None,
        },
        Template {
            id: "btop-config",
            source: ::core::include_str!("../../../configs/templates/btop.conf"),
            target: PathBuf::from("btop/btop.conf"),
            reload: Reload::None,
        },
        Template {
            id: "btop",
            source: ::core::include_str!("../../../configs/templates/btop.theme"),
            target: PathBuf::from("btop/themes/realm.theme"),
            reload: Reload::None,
        },
        Template {
            id: "starship",
            source: ::core::include_str!("../../../configs/templates/starship.toml"),
            target: PathBuf::from("starship.toml"),
            reload: Reload::None,
        },
        Template {
            id: "fuzzel",
            source: ::core::include_str!("../../../configs/templates/fuzzel.ini"),
            target: PathBuf::from("fuzzel/fuzzel.ini"),
            reload: Reload::None,
        },
        Template {
            id: "qt6ct",
            source: ::core::include_str!("../../../configs/templates/qt6ct-colors.conf"),
            target: PathBuf::from("qt6ct/colors/realm.conf"),
            reload: Reload::None,
        },
        Template {
            id: "qt6ct-config",
            source: ::core::include_str!("../../../configs/templates/qt6ct.conf"),
            target: PathBuf::from("qt6ct/qt6ct.conf"),
            reload: Reload::None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::templates;

    #[test]
    fn terminal_tool_profile_is_complete_and_generation_local() {
        let actual = templates()
            .into_iter()
            .filter_map(|template| {
                ["zsh-profile", "yazi-config", "yazi-keymap", "btop-config"]
                    .contains(&template.id)
                    .then(|| (template.id, template.target, template.source.to_owned()))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                (
                    "zsh-profile",
                    "zsh/.zshrc".into(),
                    concat!(
                        "eval \"$(starship init zsh)\"\n",
                        "btop() {\n",
                        "  command btop --config=\"$REALM_GENERATION/btop/btop.conf\" \\\n",
                        "    --themes-dir=\"$REALM_GENERATION/btop/themes\" \"$@\"\n",
                        "}\n",
                    )
                    .to_owned(),
                ),
                (
                    "yazi-config",
                    "yazi/yazi.toml".into(),
                    "[manager]\nratio = [1, 4, 3]\nsort_by = \"alphabetical\"\nsort_sensitive = false\nsort_reverse = false\nsort_dir_first = true\nlinemode = \"size\"\nshow_hidden = false\nshow_symlink = true\nscrolloff = 5\n".to_owned(),
                ),
                (
                    "yazi-keymap",
                    "yazi/keymap.toml".into(),
                    "[manager]\nprepend_keymap = [\n  { on = \"<C-p>\", run = \"shell 'btop --config=\\\"$REALM_GENERATION/btop/btop.conf\\\" --themes-dir=\\\"$REALM_GENERATION/btop/themes\\\"' --block\", desc = \"Open Realm system monitor\" },\n]\n".to_owned(),
                ),
                (
                    "btop-config",
                    "btop/btop.conf".into(),
                    "color_theme = \"realm\"\ntheme_background = False\ntruecolor = True\nforce_tty = False\nvim_keys = True\nrounded_corners = True\ngraph_symbol = \"braille\"\nshown_boxes = \"cpu mem net proc\"\n".to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn terminal_toolkit_profile_has_named_gtk_themes_and_qt6ct_default() {
        let actual = templates()
            .into_iter()
            .filter_map(|template| {
                ["gtk4-profile", "gtk3-profile", "qt6ct-config"]
                    .contains(&template.id)
                    .then(|| (template.id, template.target, template.source.to_owned()))
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            vec![
                (
                    "gtk4-profile",
                    "share/themes/realm/gtk-4.0/gtk.css".into(),
                    ::core::include_str!("../../../configs/templates/gtk4.css").to_owned(),
                ),
                (
                    "gtk3-profile",
                    "share/themes/realm/gtk-3.0/gtk.css".into(),
                    ::core::include_str!("../../../configs/templates/gtk3.css").to_owned(),
                ),
                (
                    "qt6ct-config",
                    "qt6ct/qt6ct.conf".into(),
                    ::core::include_str!("../../../configs/templates/qt6ct.conf").to_owned(),
                ),
            ]
        );
    }

    #[test]
    fn compiled_catalogue_embeds_the_declared_template_sources() {
        let actual: Vec<(&str, &str)> = templates()
            .into_iter()
            .map(|template| (template.id, template.source))
            .collect();
        assert_eq!(
            actual,
            vec![
                (
                    "gtk4",
                    ::core::include_str!("../../../configs/templates/gtk4.css")
                ),
                (
                    "gtk3",
                    ::core::include_str!("../../../configs/templates/gtk3.css")
                ),
                (
                    "gtk4-profile",
                    ::core::include_str!("../../../configs/templates/gtk4.css")
                ),
                (
                    "gtk3-profile",
                    ::core::include_str!("../../../configs/templates/gtk3.css")
                ),
                (
                    "foot",
                    ::core::include_str!("../../../configs/templates/foot.ini")
                ),
                (
                    "zsh-profile",
                    ::core::include_str!("../../../configs/templates/zshrc")
                ),
                (
                    "yazi-config",
                    ::core::include_str!("../../../configs/templates/yazi.toml")
                ),
                (
                    "yazi-keymap",
                    ::core::include_str!("../../../configs/templates/yazi-keymap.toml")
                ),
                (
                    "yazi",
                    ::core::include_str!("../../../configs/templates/yazi-theme.toml")
                ),
                (
                    "btop-config",
                    ::core::include_str!("../../../configs/templates/btop.conf")
                ),
                (
                    "btop",
                    ::core::include_str!("../../../configs/templates/btop.theme")
                ),
                (
                    "starship",
                    ::core::include_str!("../../../configs/templates/starship.toml")
                ),
                (
                    "fuzzel",
                    ::core::include_str!("../../../configs/templates/fuzzel.ini")
                ),
                (
                    "qt6ct",
                    ::core::include_str!("../../../configs/templates/qt6ct-colors.conf")
                ),
                (
                    "qt6ct-config",
                    ::core::include_str!("../../../configs/templates/qt6ct.conf")
                ),
            ],
        );
    }
}
