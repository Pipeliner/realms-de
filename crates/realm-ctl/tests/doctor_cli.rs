use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn no_session_report_is_ordered_bounded_and_keeps_independent_warnings() {
    let temp = tempfile::tempdir().unwrap();
    fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let config_home = temp.path().join("config");
    let portal_config = config_home.join("xdg-desktop-portal");
    let data_home = temp.path().join("data");
    let portal_metadata = data_home.join("xdg-desktop-portal/portals");
    fs::create_dir_all(&portal_config).unwrap();
    fs::create_dir_all(&portal_metadata).unwrap();
    fs::write(
        portal_config.join("realm-portals.conf"),
        "[preferred]\ndefault=gtk\norg.freedesktop.impl.portal.Settings=gtk\norg.freedesktop.impl.portal.Inhibit=none\norg.freedesktop.impl.portal.ScreenCast=wlr\norg.freedesktop.impl.portal.Screenshot=wlr\n",
    )
    .unwrap();
    fs::write(
        portal_metadata.join("gtk.portal"),
        "[portal]\nInterfaces=org.freedesktop.impl.portal.FileChooser;org.freedesktop.impl.portal.Settings;\n",
    )
    .unwrap();
    fs::write(
        portal_metadata.join("wlr.portal"),
        "[portal]\nInterfaces=org.freedesktop.impl.portal.ScreenCast;org.freedesktop.impl.portal.Screenshot;\n",
    )
    .unwrap();
    let palette = temp.path().join("palette.toml");
    fs::write(
        &palette,
        include_str!("../../../palette.toml")
            .replace("IBM Plex Mono", "Realm Fixture Missing Primary")
            .replace("Symbols Nerd Font Mono", "Realm Fixture Missing Symbols")
            .replace("Noto Sans Symbols 2", "Realm Fixture Missing Noto")
            .replace(
                "Noto Sans Egyptian Hieroglyphs",
                "Realm Fixture Missing Egyptian",
            )
            .replace("Symbola", "Realm Fixture Missing Symbola")
            .replace("DejaVu Sans Mono", "Realm Fixture Missing Mono"),
    )
    .unwrap();

    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_realmctl"))
        .args(["doctor", "--json", "--palette"])
        .arg(&palette)
        .env("XDG_RUNTIME_DIR", temp.path())
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_DATA_DIRS", &data_home)
        .env("XDG_CURRENT_DESKTOP", "realm")
        .env("XDG_SESSION_TYPE", "wayland")
        .env("XDG_SESSION_DESKTOP", "realm")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .output()
        .unwrap();
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "doctor failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(elapsed < Duration::from_secs(3), "doctor took {elapsed:?}");
    assert!(output.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let checks = report["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 32);
    assert_eq!(checks[0]["id"], "session/socket");
    assert_eq!(checks[0]["status"], "skip");
    assert_eq!(checks[10]["id"], "env/wayland-display/dbus");
    assert_eq!(checks[10]["status"], "skip");
    assert_eq!(checks[24]["id"], "portal/config");
    assert_eq!(checks[24]["status"], "ok");
    assert_eq!(checks[29]["id"], "fonts/glyphs");
    assert_eq!(checks[29]["status"], "warn");
    assert!(checks[29]["summary"]
        .as_str()
        .unwrap()
        .starts_with("fonts: 0/"));
}
