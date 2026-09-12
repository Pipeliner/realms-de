#![forbid(unsafe_code)]

use std::{ffi::OsString, path::PathBuf};

use anyhow::{Context, Result};
use realm_core::palette::Palette;

mod wayland;

const SHIPPED_PALETTE: &str = include_str!("../../../palette.toml");
const INSTALLED_PALETTE: &str = "/etc/realm/palette.toml";

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "realm_bar=info".into()),
        )
        .with_target(false)
        .init();

    let palette = load_palette()?;

    wayland::run(palette)
}

fn load_palette() -> Result<Palette> {
    let user = user_palette_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    );
    load_palette_from_paths(user.as_deref(), PathBuf::from(INSTALLED_PALETTE).as_path())
}

fn load_palette_from_paths(
    user: Option<&std::path::Path>,
    installed: &std::path::Path,
) -> Result<Palette> {
    if let Some(path) = user.filter(|path| path.exists()) {
        tracing::info!(path = %path.display(), "loading user palette");
        return Palette::load(path)
            .with_context(|| format!("load user palette {}", path.display()));
    }
    if installed.exists() {
        tracing::info!(path = %installed.display(), "loading installed palette");
        return Palette::load(installed)
            .with_context(|| format!("load installed palette {}", installed.display()));
    }
    tracing::warn!("user and installed palettes absent; using the shipped palette");
    Palette::from_toml(SHIPPED_PALETTE).context("parse shipped palette")
}

fn user_palette_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    xdg_config_home
        .map(PathBuf::from)
        .or_else(|| home.map(|path| PathBuf::from(path).join(".config")))
        .map(|root| root.join("realm/palette.toml"))
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, fs, path::PathBuf};

    use super::{load_palette_from_paths, user_palette_path, SHIPPED_PALETTE};

    #[test]
    fn user_palette_uses_xdg_then_home_config_location() {
        assert_eq!(
            user_palette_path(
                Some(OsString::from("/xdg")),
                Some(OsString::from("/home/user"))
            ),
            Some(PathBuf::from("/xdg/realm/palette.toml"))
        );
        assert_eq!(
            user_palette_path(None, Some(OsString::from("/home/user"))),
            Some(PathBuf::from("/home/user/.config/realm/palette.toml"))
        );
    }

    #[test]
    fn user_palette_precedes_system_and_a_broken_user_palette_is_not_hidden() {
        let root = std::env::temp_dir().join(format!(
            "realm-bar-palette-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir(&root).unwrap();
        let user = root.join("user.toml");
        let system = root.join("system.toml");
        fs::write(
            &user,
            SHIPPED_PALETTE.replacen("family = \"IBM Plex Mono\"", "family = \"User Font\"", 1),
        )
        .unwrap();
        fs::write(&system, SHIPPED_PALETTE).unwrap();

        let loaded = load_palette_from_paths(Some(&user), &system).unwrap();
        assert_eq!(loaded.typography.family, "User Font");
        fs::write(&user, "not a palette\n").unwrap();
        let error = load_palette_from_paths(Some(&user), &system).unwrap_err();
        assert!(error.to_string().contains("load user palette"));

        fs::remove_file(user).unwrap();
        fs::remove_file(system).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
