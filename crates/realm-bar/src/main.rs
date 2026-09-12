#![forbid(unsafe_code)]

use std::path::Path;

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

    let palette = if Path::new(INSTALLED_PALETTE).is_file() {
        Palette::load(INSTALLED_PALETTE)
            .with_context(|| format!("load installed palette {INSTALLED_PALETTE}"))?
    } else {
        tracing::warn!(
            path = INSTALLED_PALETTE,
            "installed palette absent; using the shipped palette"
        );
        Palette::from_toml(SHIPPED_PALETTE).context("parse shipped palette")?
    };

    wayland::run(palette)
}
