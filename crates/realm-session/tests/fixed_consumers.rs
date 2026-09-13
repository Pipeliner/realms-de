use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
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
    pid_gate_path: PathBuf,
    stop_gate_path: PathBuf,
}

impl RunningStub {
    fn wait_until_stopped(&mut self) -> u32 {
        let mut released_pid_gate = false;
        let mut released_stop_gate = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let (Ok(raw_pid), Ok(raw_args), Ok(raw_selectors)) = (
                fs::read_to_string(&self.pid_path),
                fs::read_to_string(&self.args_path),
                fs::read_to_string(&self.selectors_path),
            ) {
                if raw_pid.is_empty() && !released_pid_gate {
                    fs::write(&self.pid_gate_path, b"release\n").unwrap();
                    released_pid_gate = true;
                }
                if let Ok(pid) = raw_pid.trim().parse() {
                    if !raw_args.is_empty() && !raw_selectors.is_empty() {
                        if !released_stop_gate {
                            fs::write(&self.stop_gate_path, b"release\n").unwrap();
                            released_stop_gate = true;
                        }
                        if process_is_stopped(pid) {
                            return pid;
                        }
                    }
                }
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("fixed consumer exited before exec fixture stopped: {status}");
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let status = self.child.wait();
                panic!("fixed consumer fixture did not stop before its deadline: {status:?}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn resume_and_wait(&mut self, pid: u32) -> ExitStatus {
        let pid = rustix::process::Pid::from_raw(i32::try_from(pid).unwrap()).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::CONT).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let status = self.child.wait();
                panic!("resumed fixed consumer did not exit before its deadline: {status:?}");
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn process_is_stopped(pid: u32) -> bool {
    let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    status.lines().any(|line| {
        line.strip_prefix("State:")
            .is_some_and(|state| state.trim_start().starts_with('T'))
    })
}

fn install_stopping_stub(directory: &Path, name: &str) {
    let path = directory.join(name);
    fs::write(
        &path,
        "#!/bin/sh\n: > \"$REALM_TEST_PID\"\n: > \"$REALM_TEST_ARGS\"\n: > \"$REALM_TEST_SELECTORS\"\nwhile [ ! -e \"$REALM_TEST_PID_GATE\" ]; do :; done\nprintf '%s\\n' \"$$\" > \"$REALM_TEST_PID\"\nprintf '%s\\n' \"$@\" > \"$REALM_TEST_ARGS\"\nprintf '%s\\n' \"${REALM_GENERATION-unset}\" \"${ZDOTDIR-unset}\" \"${STARSHIP_CONFIG-unset}\" \"${YAZI_CONFIG_HOME-unset}\" \"${GTK_THEME-unset}\" \"${XDG_DATA_DIRS-unset}\" \"${QT_QPA_PLATFORMTHEME-unset}\" \"${XDG_CONFIG_DIRS-unset}\" > \"$REALM_TEST_SELECTORS\"\nwhile [ ! -e \"$REALM_TEST_STOP_GATE\" ]; do :; done\nkill -STOP \"$$\"\n",
    )
    .unwrap();
    fs::set_permissions(path, PermissionsExt::from_mode(0o700)).unwrap();
}

fn spawn_consumer(root: &Path, stub_dir: &Path, kind: &str) -> RunningStub {
    let pid_path = root.join(format!("{kind}.pid"));
    let args_path = root.join(format!("{kind}.args"));
    let selectors_path = root.join(format!("{kind}.selectors"));
    let pid_gate_path = root.join(format!("{kind}.pid-gate"));
    let stop_gate_path = root.join(format!("{kind}.stop-gate"));
    let child = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
        .args(["--fixed-consumer", kind])
        .env("XDG_CONFIG_HOME", root)
        .env_remove("HOME")
        .env("PATH", stub_dir)
        .env("XDG_DATA_DIRS", "/existing/data:/second/data")
        .env("XDG_CONFIG_DIRS", "/existing/config:/second/config")
        .env("REALM_TEST_PID", &pid_path)
        .env("REALM_TEST_ARGS", &args_path)
        .env("REALM_TEST_SELECTORS", &selectors_path)
        .env("REALM_TEST_PID_GATE", &pid_gate_path)
        .env("REALM_TEST_STOP_GATE", &stop_gate_path)
        .spawn()
        .unwrap();
    RunningStub {
        child,
        pid_path,
        args_path,
        selectors_path,
        pid_gate_path,
        stop_gate_path,
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
fn fixed_consumers_exec_exact_generation_argv_environment_and_hold_the_ordinary_process_lease() {
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
                "--log-level=error".to_owned(),
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
                "realm".to_owned(),
                format!(
                    "{}/share:/existing/data:/second/data",
                    generation_path.display()
                ),
                "qt6ct".to_owned(),
                format!(
                    "{}:/existing/config:/second/config",
                    generation_path.display()
                ),
            ]
        } else {
            vec![
                "unset".to_owned(),
                "unset".to_owned(),
                "unset".to_owned(),
                "unset".to_owned(),
                "unset".to_owned(),
                "/existing/data:/second/data".to_owned(),
                "unset".to_owned(),
                "/existing/config:/second/config".to_owned(),
            ]
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
fn terminal_refuses_generation_paths_that_xdg_lists_cannot_represent() {
    let parent = tempfile::tempdir().unwrap();
    let empty_path = tempfile::tempdir().unwrap();
    let roots = [
        parent.path().join("colon:root"),
        parent
            .path()
            .join(OsString::from_vec(b"non-utf8-\xff".to_vec())),
    ];

    for root in roots {
        fs::create_dir(&root).unwrap();
        realm_theme::apply(&root).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_realm-wm"))
            .args(["--fixed-consumer", "terminal"])
            .env("XDG_CONFIG_HOME", &root)
            .env_remove("HOME")
            .env("PATH", empty_path.path())
            .output()
            .unwrap();

        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("generation path cannot be represented in XDG search lists"),
            "unrepresentable generation path was not rejected before exec"
        );
        assert!(lease_names(&root).is_empty());
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
