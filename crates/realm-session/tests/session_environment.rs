use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Login {
    root: tempfile::TempDir,
    runtime: tempfile::TempDir,
    tools: tempfile::TempDir,
    selected: PathBuf,
}

impl Login {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700)).unwrap();
        realm_session::login_theme::prepare(root.path(), runtime.path(), std::process::id())
            .unwrap();
        let selected = realm_session::login_theme::load(runtime.path())
            .unwrap()
            .path()
            .to_owned();
        for name in ["foot", "fuzzel"] {
            let file = tools.path().join(name);
            fs::write(&file, r#"#!/bin/sh
if [ "$1" = --check-config ]; then
  printf '%s\n' "$2" >> "$TEST_ROOT/probes"
  if [ "$TEST_FOOT_MODE" = hung ]; then
    printf '%s\n' "$$" > "$TEST_ROOT/probe-pid"
    exec /bin/sleep 30
  fi
  case "$TEST_FOOT_MODE:$2" in
    invalid:*|legacy:*foot-modern.ini) exit 1 ;;
  esac
  exit 0
fi
printf '%s\n' "$@" > "$TEST_ROOT/argv"
printf '%s\n' "${REALM_GENERATION-unset}" "${ZDOTDIR-unset}" "${STARSHIP_CONFIG-unset}" "${YAZI_CONFIG_HOME-unset}" "${GTK_THEME-unset}" "${XDG_DATA_DIRS-unset}" "${QT_QPA_PLATFORMTHEME-unset}" "${XDG_CONFIG_DIRS-unset}" "$XDG_CONFIG_HOME" > "$TEST_ROOT/environment"
"#).unwrap();
            fs::set_permissions(file, fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self {
            root,
            runtime,
            tools,
            selected,
        }
    }

    fn run(&self, kind: &str, mode: &str, qt: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_realm-wm"));
        command
            .args(["--fixed-consumer", kind])
            .env("XDG_RUNTIME_DIR", self.runtime.path())
            .env("XDG_CONFIG_HOME", self.root.path())
            .env("PATH", self.tools.path())
            .env("TEST_ROOT", self.root.path())
            .env("TEST_FOOT_MODE", mode)
            .env("XDG_DATA_DIRS", "/existing/data")
            .env("XDG_CONFIG_DIRS", "/existing/config")
            .env_remove("QT_QPA_PLATFORMTHEME");
        if let Some(qt) = qt {
            command.env("QT_QPA_PLATFORMTHEME", qt);
        }
        for variable in [
            "REALM_GENERATION",
            "ZDOTDIR",
            "STARSHIP_CONFIG",
            "YAZI_CONFIG_HOME",
            "GTK_THEME",
        ] {
            command.env_remove(variable);
        }
        command.output().unwrap()
    }
}

#[test]
fn terminal_and_launcher_children_inherit_login_a_after_apply_b() {
    let login = Login::new();
    realm_theme::apply(login.root.path()).unwrap();
    for kind in ["terminal", "launcher"] {
        let output = login.run(kind, "modern", None);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let selected = login.selected.display().to_string();
        assert_eq!(
            fs::read_to_string(login.root.path().join("environment"))
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            vec![
                selected.clone(),
                format!("{selected}/zsh"),
                format!("{selected}/starship.toml"),
                format!("{selected}/yazi"),
                "realm".into(),
                format!("{selected}/share:/existing/data"),
                "qt6ct".into(),
                format!("{selected}:/existing/config"),
                login.root.path().display().to_string(),
            ]
        );
        let args = fs::read_to_string(login.root.path().join("argv")).unwrap();
        if kind == "terminal" {
            assert_eq!(
                args.lines().collect::<Vec<_>>(),
                vec![
                    format!("--config={selected}/foot/foot-modern.ini"),
                    "--log-level=error".into(),
                    "--override=key-bindings.spawn-terminal=none".into(),
                    "zsh".into(),
                ]
            );
        } else {
            assert_eq!(args, format!("--config={selected}/fuzzel/fuzzel.ini\n"));
        }
    }
}

#[test]
fn explicit_qt_selector_and_user_configuration_are_preserved() {
    let login = Login::new();
    let config = login.root.path().join("qt6ct/qt6ct.conf");
    fs::create_dir(config.parent().unwrap()).unwrap();
    fs::write(&config, b"user-owned configuration\n").unwrap();
    let before: Vec<_> = std::env::vars_os().collect();
    let output = login.run("launcher", "modern", Some("user-plugin"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let environment = fs::read_to_string(login.root.path().join("environment")).unwrap();
    assert_eq!(environment.lines().nth(6), Some("user-plugin"));
    assert_eq!(fs::read(config).unwrap(), b"user-owned configuration\n");
    assert_eq!(std::env::vars_os().collect::<Vec<_>>(), before);
}

#[test]
fn foot_legacy_probe_fallback_never_uses_mutable_configuration() {
    let login = Login::new();
    let output = login.run("terminal", "legacy", None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let selected = login.selected.display();
    assert_eq!(
        fs::read_to_string(login.root.path().join("probes")).unwrap(),
        format!("--config={selected}/foot/foot-modern.ini\n--config={selected}/foot/foot.ini\n")
    );
    assert!(fs::read_to_string(login.root.path().join("argv"))
        .unwrap()
        .starts_with(&format!("--config={selected}/foot/foot.ini\n")));
}

#[test]
fn rejected_foot_profiles_abort_before_launch() {
    let login = Login::new();
    let output = login.run("terminal", "invalid", None);
    assert!(!output.status.success());
    assert!(!login.root.path().join("argv").exists());
    assert_eq!(
        fs::read_dir(login.root.path().join("realm/generated/leases"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn hung_foot_probe_is_reaped_without_launch_or_lease_leak() {
    let login = Login::new();
    let started = std::time::Instant::now();
    let output = login.run("terminal", "hung", None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("shared one-second deadline"));
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    let pid = fs::read_to_string(login.root.path().join("probe-pid")).unwrap();
    assert!(!std::path::Path::new("/proc").join(pid.trim()).exists());
    assert!(!login.root.path().join("argv").exists());
    assert_eq!(
        fs::read_dir(login.root.path().join("realm/generated/leases"))
            .unwrap()
            .count(),
        1
    );
}
