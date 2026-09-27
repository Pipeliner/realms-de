#![forbid(unsafe_code)]

use anyhow::{Context, Result};
use realm_core::palette::Palette;
use realm_theme::generation::GenerationSelection;
use std::path::Path;

mod wayland;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "realm_bar=info".into()),
        )
        .with_target(false)
        .init();

    let runtime = realm_control::production_runtime_dir()?;
    let (selection, palette) = load_palette(runtime.path())?;
    let result = wayland::run(palette);
    drop(selection);
    result
}

fn load_palette(runtime: &Path) -> Result<(GenerationSelection, Palette)> {
    let selection = realm_theme::login_theme::load(runtime).map_err(anyhow::Error::msg)?;
    let bytes = selection
        .read_output("realm/palette.toml")
        .map_err(anyhow::Error::msg)?;
    let text = std::str::from_utf8(&bytes).context("selected palette is not UTF-8")?;
    let palette = Palette::from_toml(text).context("parse selected login palette")?;
    tracing::info!(generation = selection.as_str(), "loading login palette");
    Ok((selection, palette))
}

#[cfg(test)]
mod tests {
    use super::load_palette;
    use realm_theme::{login_theme, SHIPPED_PALETTE};
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::{Child, Command},
    };

    struct Owner(Child);
    impl Drop for Owner {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn bar_restart_keeps_login_palette_after_source_edit_and_apply() {
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let mut owner = Owner(Command::new("sleep").arg("30").spawn().unwrap());
        login_theme::prepare(root.path(), runtime.path(), owner.0.id()).unwrap();
        let user = root.path().join("realm/palette.toml");
        let (first_lease, first) = load_palette(runtime.path()).unwrap();
        fs::write(
            &user,
            SHIPPED_PALETTE.replacen("IBM Plex Mono", "Next Login Font", 1),
        )
        .unwrap();
        let next = realm_theme::apply(root.path()).unwrap();
        let (restart_lease, restarted) = load_palette(runtime.path()).unwrap();
        assert_eq!(restarted.typography.family, first.typography.family);
        assert_eq!(restart_lease.as_str(), first_lease.as_str());
        assert_ne!(restart_lease.as_str(), next.as_str());
        drop(restart_lease);
        owner.0.kill().unwrap();
        owner.0.wait().unwrap();
        login_theme::prepare(root.path(), runtime.path(), std::process::id()).unwrap();
        let (next_lease, next_palette) = load_palette(runtime.path()).unwrap();
        assert_eq!(next_palette.typography.family, "Next Login Font");
        assert_eq!(next_lease.as_str(), next.as_str());
        let store =
            realm_theme::generation::GenerationStore::open(&root.path().join("realm/generated"))
                .unwrap();
        store.garbage_collect().unwrap();
        assert!(first_lease.read_output("realm/palette.toml").is_ok());
    }

    #[test]
    fn missing_or_corrupt_selected_palette_never_uses_mutable_fallback() {
        for missing in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let runtime = tempfile::tempdir().unwrap();
            assert!(load_palette(runtime.path()).is_err());
            login_theme::prepare(root.path(), runtime.path(), std::process::id()).unwrap();
            let selection = login_theme::load(runtime.path()).unwrap();
            let snapshot = selection.path().join("realm/palette.toml");
            if missing {
                fs::set_permissions(
                    snapshot.parent().unwrap(),
                    fs::Permissions::from_mode(0o700),
                )
                .unwrap();
                fs::remove_file(&snapshot).unwrap();
            } else {
                fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&snapshot, b"broken palette").unwrap();
                fs::set_permissions(&snapshot, fs::Permissions::from_mode(0o400)).unwrap();
            }
            assert!(load_palette(runtime.path()).is_err());
            assert_eq!(
                fs::read(root.path().join("realm/palette.toml")).unwrap(),
                SHIPPED_PALETTE.as_bytes()
            );
        }
    }
}
