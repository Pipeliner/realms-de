use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use rustix::fs::{fstat, FileType, Mode};
use rustix::io::Errno;
use rustix::process::geteuid;

use crate::{production_runtime_dir, test_runtime_dir, IpcPathError, RuntimeDir};

fn environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvironmentRestore {
    xdg_runtime_dir: Option<OsString>,
    realm_socket: Option<OsString>,
}

impl EnvironmentRestore {
    fn capture() -> Self {
        Self {
            xdg_runtime_dir: std::env::var_os("XDG_RUNTIME_DIR"),
            realm_socket: std::env::var_os("REALM_SOCKET"),
        }
    }
}

impl Drop for EnvironmentRestore {
    fn drop(&mut self) {
        match &self.xdg_runtime_dir {
            Some(value) => std::env::set_var("XDG_RUNTIME_DIR", value),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        match &self.realm_socket {
            Some(value) => std::env::set_var("REALM_SOCKET", value),
            None => std::env::remove_var("REALM_SOCKET"),
        }
    }
}

/// Catches a regression where a panic in an environment-mutating test leaks
/// its override into later tests.
#[test]
fn environment_restore_guard_restores_overrides_after_panic() {
    let _lock = environment_lock().lock().unwrap();
    let _actual_environment = EnvironmentRestore::capture();
    std::env::set_var("XDG_RUNTIME_DIR", "/before-panic");
    std::env::set_var("REALM_SOCKET", "/before-panic.sock");

    let result = std::panic::catch_unwind(|| {
        let _restored_environment = EnvironmentRestore::capture();
        std::env::set_var("XDG_RUNTIME_DIR", "/during-panic");
        std::env::set_var("REALM_SOCKET", "/during-panic.sock");
        panic!("intentional fixture panic");
    });

    assert!(result.is_err());
    assert_eq!(
        std::env::var_os("XDG_RUNTIME_DIR"),
        Some("/before-panic".into())
    );
    assert_eq!(
        std::env::var_os("REALM_SOCKET"),
        Some("/before-panic.sock".into())
    );
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn assert_error(result: Result<RuntimeDir, IpcPathError>, expected: fn(&IpcPathError) -> bool) {
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected an error"),
    };
    assert!(expected(&error), "unexpected error: {error:?}");
}

fn is_missing(error: &IpcPathError) -> bool {
    matches!(error, IpcPathError::MissingRuntimeDir)
}

fn is_unsafe_runtime(error: &IpcPathError) -> bool {
    matches!(error, IpcPathError::UnsafeRuntimeDir)
}

/// Catches a regression where an unsafe runtime input is opened or an opener
/// error is collapsed into a generic path failure.
#[test]
fn runtime_capability_rejects_every_unsafe_input_and_openat2_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("runtime");
    fs::create_dir(&root).unwrap();
    set_mode(&root, 0o700);

    assert_error(
        test_runtime_dir(&temporary.path().join("absent")),
        is_missing,
    );
    assert_error(test_runtime_dir(Path::new("relative-runtime")), is_missing);

    let not_a_directory = temporary.path().join("file");
    fs::write(&not_a_directory, "not a directory").unwrap();
    assert_error(test_runtime_dir(&not_a_directory), is_missing);

    let link = temporary.path().join("runtime-link");
    symlink(&root, &link).unwrap();
    assert_error(test_runtime_dir(&link), is_unsafe_runtime);

    for mode in [0o000, 0o600, 0o701, 0o710, 0o770, 0o777] {
        set_mode(&root, mode);
        assert_error(test_runtime_dir(&root), is_unsafe_runtime);
    }
    set_mode(&root, 0o700);

    assert!(matches!(
        crate::runtime::validate_directory_properties(
            FileType::Directory,
            geteuid().as_raw().wrapping_add(1),
            Mode::from_raw_mode(0o700),
        ),
        Err(IpcPathError::UnsafeRuntimeDir)
    ));

    let error = match crate::runtime::resolve_runtime_with(&root, |_| Err(Errno::IO)) {
        Err(error) => error,
        Ok(_) => panic!("expected opener failure"),
    };
    match error {
        IpcPathError::Io(error) => assert_eq!(error.raw_os_error(), Some(Errno::IO.raw_os_error())),
        other => panic!("expected retained opener errno, got {other:?}"),
    }

    for (errno, expected_raw_errno) in [(Errno::NOSYS, 38), (Errno::INVAL, 22), (Errno::XDEV, 18)] {
        let error = match crate::runtime::resolve_runtime_with(&root, |_| Err(errno)) {
            Err(error) => error,
            Ok(_) => panic!("expected opener failure"),
        };
        match error {
            IpcPathError::Io(error) => assert_eq!(error.raw_os_error(), Some(expected_raw_errno)),
            other => panic!("expected retained opener errno, got {other:?}"),
        }
    }
}

/// Catches a regression where production consults another socket override or
/// accepts an unset/relative XDG runtime path.
#[test]
fn production_runtime_reads_only_xdg_runtime_dir() {
    let _guard = environment_lock().lock().unwrap();
    let _environment = EnvironmentRestore::capture();
    let temporary = tempfile::tempdir().unwrap();
    let runtime = temporary.path().join("runtime");
    fs::create_dir(&runtime).unwrap();
    set_mode(&runtime, 0o700);

    std::env::remove_var("XDG_RUNTIME_DIR");
    std::env::set_var("REALM_SOCKET", "/must-not-be-read");
    assert_error(production_runtime_dir(), is_missing);

    std::env::set_var("XDG_RUNTIME_DIR", "relative-runtime");
    assert_error(production_runtime_dir(), is_missing);

    std::env::set_var("XDG_RUNTIME_DIR", &runtime);
    let resolved = production_runtime_dir().unwrap();
    assert_eq!(resolved.path(), runtime.as_path());
}

/// Catches a regression where `realm` is reopened through the mutable display
/// path or accepted with a non-exact directory mode.
#[test]
fn realm_directory_validation_is_descriptor_relative_and_exact() {
    let temporary = tempfile::tempdir().unwrap();
    let runtime_path = temporary.path().join("runtime");
    fs::create_dir(&runtime_path).unwrap();
    set_mode(&runtime_path, 0o700);
    let realm_path = runtime_path.join("realm");
    fs::create_dir(&realm_path).unwrap();
    set_mode(&realm_path, 0o700);

    let runtime = test_runtime_dir(&runtime_path).unwrap();
    let relocated = temporary.path().join("relocated-runtime");
    fs::rename(&runtime_path, &relocated).unwrap();
    symlink(&relocated, &runtime_path).unwrap();

    let realm = runtime.open_realm_dir().unwrap();
    let stat = fstat(realm.as_fd()).unwrap();
    assert_eq!(FileType::from_raw_mode(stat.st_mode), FileType::Directory);
    assert_eq!(Mode::from_raw_mode(stat.st_mode).bits(), 0o700);

    for mode in [0o000, 0o600, 0o701, 0o710, 0o770, 0o777] {
        set_mode(&relocated.join("realm"), mode);
        assert!(matches!(
            runtime.open_realm_dir(),
            Err(IpcPathError::UnsafeRealmDirectory)
        ));
    }
}
