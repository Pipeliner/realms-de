use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use realm_theme::generation::{GenerationPublication, GenerationStore};

struct RunningStub {
    child: Child,
    pid_path: PathBuf,
    args_path: PathBuf,
    selectors_path: PathBuf,
}

impl RunningStub {
    fn wait_until_stopped(&mut self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(raw) = fs::read_to_string(&self.pid_path) {
                return raw.trim().parse().unwrap();
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("fixed consumer exited before exec fixture stopped: {status}");
            }
            assert!(
                Instant::now() < deadline,
                "fixed consumer fixture timed out"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn resume_and_wait(&mut self, pid: u32) -> ExitStatus {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::CONT).unwrap();
        self.child.wait().unwrap()
    }
}

fn install_stopping_stub(directory: &Path, name: &str) {
    let path = directory.join(name);
    fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$REALM_TEST_PID\"\nprintf '%s\\n' \"$@\" > \"$REALM_TEST_ARGS\"\nprintf '%s\\n' \"${REALM_GENERATION-unset}\" \"${ZDOTDIR-unset}\" \"${STARSHIP_CONFIG-unset}\" \"${YAZI_CONFIG_HOME-unset}\" > \"$REALM_TEST_SELECTORS\"\nkill -STOP \"$$\"\n",
    )
    .unwrap();
    fs::set_permissions(path, PermissionsExt::from_mode(0o700)).unwrap();
}

fn spawn_consumer(root: &Path, stub_dir: &Path, kind: &str) -> RunningStub {
    let pid_path = root.join(format!("{kind}.pid"));
    let args_path = root.join(format!("{kind}.args"));
    let selectors_path = root.join(format!("{kind}.selectors"));
    let child = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
        .args(["--fixed-consumer", kind])
        .env("XDG_CONFIG_HOME", root)
        .env_remove("HOME")
        .env("PATH", stub_dir)
        .env("REALM_TEST_PID", &pid_path)
        .env("REALM_TEST_ARGS", &args_path)
        .env("REALM_TEST_SELECTORS", &selectors_path)
        .spawn()
        .unwrap();
    RunningStub {
        child,
        pid_path,
        args_path,
        selectors_path,
    }
}

fn lease_names(root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(root.join("realm/generated/leases"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn fixed_consumers_exec_exact_generation_argv_and_hold_the_ordinary_process_lease() {
    let stubs = tempfile::tempdir().unwrap();
    install_stopping_stub(stubs.path(), "foot");
    install_stopping_stub(stubs.path(), "fuzzel");

    for kind in ["terminal", "launcher"] {
        let root = tempfile::tempdir().unwrap();
        realm_theme::apply(root.path()).unwrap();
        let generation = fs::read_to_string(root.path().join("realm/generated/current"))
            .unwrap()
            .trim()
            .to_owned();
        let generation_path = root
            .path()
            .join("realm/generated/generations")
            .join(&generation);
        let expected_args = if kind == "terminal" {
            vec![
                format!(
                    "--config={}",
                    generation_path.join("foot/foot.ini").display()
                ),
                "--override=key-bindings.spawn-terminal=none".to_owned(),
                "zsh".to_owned(),
            ]
        } else {
            vec![format!(
                "--config={}",
                generation_path.join("fuzzel/fuzzel.ini").display()
            )]
        };
        let mut running = spawn_consumer(root.path(), stubs.path(), kind);
        let pid = running.wait_until_stopped();
        assert_eq!(pid, running.child.id(), "exec changed the consumer pid");
        assert_eq!(
            fs::read_to_string(&running.args_path)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected_args.iter().map(String::as_str).collect::<Vec<_>>()
        );
        let expected_selectors = if kind == "terminal" {
            vec![
                generation_path.display().to_string(),
                generation_path.join("zsh").display().to_string(),
                generation_path.join("starship.toml").display().to_string(),
                generation_path.join("yazi").display().to_string(),
            ]
        } else {
            vec!["unset".to_owned(); 4]
        };
        assert_eq!(
            fs::read_to_string(&running.selectors_path)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected_selectors
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        );
        let leases = lease_names(root.path());
        assert_eq!(leases.len(), 1, "consumer did not retain exactly one lease");
        assert!(
            fs::read_to_string(root.path().join("realm/generated/leases").join(&leases[0]))
                .unwrap()
                .contains(&format!("pid {pid}\n"))
        );

        let next = realm_theme::apply(root.path()).unwrap().as_str().to_owned();
        assert_ne!(next, generation, "the fixture did not switch current");
        assert!(
            generation_path.is_dir(),
            "a later apply removed N while its fixed consumer was still alive"
        );

        assert!(running.resume_and_wait(pid).success());
        realm_theme::apply(root.path()).unwrap();
        assert!(
            generation_path.is_dir(),
            "a later apply removed N after Foot exited"
        );
    }
}

#[test]
fn terminal_rejects_an_incomplete_valid_generation_before_exec() {
    let root = tempfile::tempdir().unwrap();
    realm_theme::apply(root.path()).unwrap();
    let store = GenerationStore::open(&root.path().join("realm/generated")).unwrap();
    let digest = "0".repeat(64);
    store
        .publish(|| {
            GenerationPublication::new(
                [
                    digest.clone(),
                    digest.clone(),
                    digest.clone(),
                    digest.clone(),
                    digest,
                ],
                vec![("foot/foot.ini".to_owned(), b"font=monospace\n".to_vec())],
            )
        })
        .unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
        .args(["--fixed-consumer", "terminal"])
        .env("XDG_CONFIG_HOME", root.path())
        .env_remove("HOME")
        .env("PATH", empty_path.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("required output zsh/.zshrc is unavailable"),
        "missing output was not identified"
    );
}

#[test]
fn fixed_consumer_mode_accepts_only_one_known_consumer_argument() {
    let root = tempfile::tempdir().unwrap();
    realm_theme::apply(root.path()).unwrap();
    for args in [
        vec!["--fixed-consumer"],
        vec!["--fixed-consumer", "unknown"],
        vec!["--fixed-consumer", "terminal", "extra"],
    ] {
        let status = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
            .args(args)
            .env("XDG_CONFIG_HOME", root.path())
            .env_remove("HOME")
            .status()
            .unwrap();
        assert!(!status.success());
        assert!(lease_names(root.path()).is_empty());
    }
}

#[test]
fn fixed_consumer_exec_failure_releases_its_process_lease() {
    let root = tempfile::tempdir().unwrap();
    realm_theme::apply(root.path()).unwrap();
    let empty_path = tempfile::tempdir().unwrap();

    let status = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
        .args(["--fixed-consumer", "terminal"])
        .env("XDG_CONFIG_HOME", root.path())
        .env_remove("HOME")
        .env("PATH", empty_path.path())
        .status()
        .unwrap();

    assert!(!status.success());
    assert!(lease_names(root.path()).is_empty());
}
