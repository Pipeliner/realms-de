use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[test]
fn private_login_helper_prepares_once_for_the_graphical_owner() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    std::fs::set_permissions(runtime.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_realm-wm"))
            .args(["--prepare-session-theme", &std::process::id().to_string()])
            .env("XDG_CONFIG_HOME", root.path())
            .env("XDG_RUNTIME_DIR", runtime.path())
            .output()
            .unwrap()
    };
    let first = run();
    assert!(first.status.success(), "{first:?}");
    let selection = realm_session::login_theme::load(runtime.path()).unwrap();
    assert!(
        !run().status.success(),
        "second helper overwrote a live selection"
    );
    realm_theme::apply(root.path()).unwrap();
    assert_eq!(
        realm_session::login_theme::load(runtime.path())
            .unwrap()
            .as_str(),
        selection.as_str()
    );
}

#[test]
fn next_login_selects_b_only_after_a_owner_exits() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    let mut owner = Command::new("sleep").arg("30").spawn().unwrap();
    let preparation = realm_session::login_theme::prepare(root.path(), runtime.path(), owner.id());
    let active = preparation.and_then(|()| realm_session::login_theme::load(runtime.path()));
    let next = realm_theme::apply(root.path());
    let before_exit = realm_session::login_theme::load(runtime.path());
    owner.kill().unwrap();
    owner.wait().unwrap();
    let active = active.unwrap();
    let next = next.unwrap();
    assert_ne!(active.as_str(), next.as_str());
    assert_eq!(before_exit.unwrap().as_str(), active.as_str());
    assert!(realm_session::login_theme::load(runtime.path()).is_err());
    realm_session::login_theme::prepare(root.path(), runtime.path(), std::process::id()).unwrap();
    assert_eq!(
        realm_session::login_theme::load(runtime.path())
            .unwrap()
            .as_str(),
        next.as_str()
    );
}
