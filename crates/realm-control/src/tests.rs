use std::ffi::OsString;
use std::fs;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use rustix::fs::{fstat, openat2, FileType, Mode, OFlags, ResolveFlags, Stat, CWD};
use rustix::io::Errno;
use rustix::process::{geteuid, umask};

use crate::{production_runtime_dir, test_runtime_dir, IpcPathError, RuntimeDir, SocketEndpoint};

fn environment_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn umask_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ProcessUmaskRestore {
    original: Mode,
}

impl ProcessUmaskRestore {
    fn replace(mask: u32) -> Self {
        Self {
            original: umask(Mode::from_raw_mode(mask)),
        }
    }
}

impl Drop for ProcessUmaskRestore {
    fn drop(&mut self) {
        umask(self.original);
    }
}

fn current_umask() -> u32 {
    let current = umask(Mode::empty());
    umask(current);
    current.bits()
}

#[derive(Clone)]
enum BridgeOpen {
    Requested,
    ProcFdParent,
    OtherDirectory(PathBuf),
    Error(Errno),
}

#[derive(Clone, Copy)]
enum BridgeStat {
    Actual,
    Error(Errno),
    WrongDevice,
    WrongInode,
    WrongType,
    WrongOwner,
    WrongMode,
}

#[derive(Clone)]
struct InjectedRuntimeBridge {
    open: BridgeOpen,
    stat: BridgeStat,
}

impl InjectedRuntimeBridge {
    fn actual() -> Self {
        Self {
            open: BridgeOpen::Requested,
            stat: BridgeStat::Actual,
        }
    }
}

impl crate::sys::RuntimeBridge for InjectedRuntimeBridge {
    fn open(&self, requested: &Path) -> rustix::io::Result<OwnedFd> {
        let path = match &self.open {
            BridgeOpen::Requested => requested,
            BridgeOpen::ProcFdParent => Path::new("/proc/self/fd"),
            BridgeOpen::OtherDirectory(path) => path,
            BridgeOpen::Error(error) => return Err(*error),
        };
        openat2(
            CWD,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::empty(),
        )
    }

    fn stat(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<Stat> {
        let mut stat = fstat(fd)?;
        match self.stat {
            BridgeStat::Actual => {}
            BridgeStat::Error(error) => return Err(error),
            BridgeStat::WrongDevice => stat.st_dev = stat.st_dev.wrapping_add(1),
            BridgeStat::WrongInode => stat.st_ino = stat.st_ino.wrapping_add(1),
            BridgeStat::WrongType => {
                stat.st_mode = FileType::RegularFile.as_raw_mode() | 0o700;
            }
            BridgeStat::WrongOwner => stat.st_uid = stat.st_uid.wrapping_add(1),
            BridgeStat::WrongMode => {
                stat.st_mode = FileType::Directory.as_raw_mode() | 0o755;
            }
        }
        Ok(stat)
    }
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

fn runtime_fixture() -> (tempfile::TempDir, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let runtime_path = temporary.path().join("runtime");
    fs::create_dir(&runtime_path).unwrap();
    set_mode(&runtime_path, 0o700);
    (temporary, runtime_path)
}

fn endpoint_error(result: Result<SocketEndpoint, IpcPathError>) -> IpcPathError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected endpoint preparation to fail"),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct EntryFingerprint {
    device: u64,
    inode: u64,
    mode: u32,
    bytes: Vec<u8>,
}

fn entry_fingerprint(path: &Path) -> EntryFingerprint {
    let metadata = fs::symlink_metadata(path).unwrap();
    EntryFingerprint {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        bytes: fs::read(path).unwrap(),
    }
}

fn assert_preflight_failure_preserves_namespace(bridge: InjectedRuntimeBridge) {
    let (_temporary, runtime_path) = runtime_fixture();
    let runtime = test_runtime_dir(&runtime_path).unwrap();
    endpoint_error(runtime.prepare_server_endpoint_with(&bridge, 108));
    assert!(!runtime_path.join("realm").exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let realm_path = runtime_path.join("realm");
    fs::create_dir(&realm_path).unwrap();
    set_mode(&realm_path, 0o700);
    let socket_path = realm_path.join("ctl.sock");
    fs::write(&socket_path, b"must remain untouched").unwrap();
    set_mode(&socket_path, 0o600);
    let before = entry_fingerprint(&socket_path);

    let runtime = test_runtime_dir(&runtime_path).unwrap();
    endpoint_error(runtime.prepare_server_endpoint_with(&bridge, 108));
    assert_eq!(entry_fingerprint(&socket_path), before);
}

/// Catches a regression where endpoint preparation inherits a hostile umask,
/// replaces or chmods an existing realm, or leaks its temporary umask.
#[test]
fn server_creates_realm_exactly_once_under_scoped_umask() {
    let _lock = umask_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (temporary, runtime_path) = runtime_fixture();
    let _restore = ProcessUmaskRestore::replace(0o777);

    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let realm_path = runtime_path.join("realm");
    let realm_metadata = fs::metadata(&realm_path).unwrap();
    assert_eq!(realm_metadata.mode() & 0o777, 0o700);
    assert_eq!(
        endpoint.path(),
        runtime_path.join("realm/ctl.sock").as_path()
    );
    assert_eq!(
        Mode::from_raw_mode(fstat(endpoint.realm_dir().as_fd()).unwrap().st_mode).bits(),
        0o700
    );
    assert_eq!(current_umask(), 0o777);
    let original_realm_inode = realm_metadata.ino();
    drop(endpoint);

    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let realm_metadata = fs::metadata(&realm_path).unwrap();
    assert_eq!(realm_metadata.ino(), original_realm_inode);
    assert_eq!(realm_metadata.mode() & 0o777, 0o700);
    assert_eq!(current_umask(), 0o777);
    drop(endpoint);

    set_mode(&realm_path, 0o755);
    assert!(matches!(
        endpoint_error(
            test_runtime_dir(&runtime_path)
                .unwrap()
                .prepare_server_endpoint()
        ),
        IpcPathError::UnsafeRealmDirectory
    ));
    let realm_metadata = fs::metadata(&realm_path).unwrap();
    assert_eq!(realm_metadata.ino(), original_realm_inode);
    assert_eq!(realm_metadata.mode() & 0o777, 0o755);
    assert_eq!(current_umask(), 0o777);

    let unwind = std::panic::catch_unwind(|| {
        let _scoped = crate::sys::ScopedUmask::new(Mode::from_raw_mode(0o077));
        assert_eq!(current_umask(), 0o077);
        panic!("intentional scoped-umask unwind");
    });
    assert!(unwind.is_err());
    assert_eq!(current_umask(), 0o777);
    drop(temporary);
}

/// Catches a regression where checking `/proc/self/fd` is mistaken for
/// traversing and validating the bridge for the retained runtime descriptor.
#[test]
fn actual_runtime_procfd_bridge_is_verified_before_mutation() {
    let _lock = umask_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint_with(&InjectedRuntimeBridge::actual(), 108)
        .unwrap();
    assert_eq!(
        endpoint.path(),
        runtime_path.join("realm/ctl.sock").as_path()
    );
    assert!(runtime_path.join("realm").is_dir());

    assert_preflight_failure_preserves_namespace(InjectedRuntimeBridge {
        open: BridgeOpen::ProcFdParent,
        stat: BridgeStat::Actual,
    });
}

/// Catches regressions that ignore bridge open/stat failures or omit any of
/// the retained runtime identity, type, owner, and exact-mode comparisons.
#[test]
fn missing_inaccessible_or_mismatched_procfd_fails_before_mutation() {
    let _lock = umask_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let other = tempfile::tempdir().unwrap();
    set_mode(other.path(), 0o700);

    for bridge in [
        InjectedRuntimeBridge {
            open: BridgeOpen::Error(Errno::NOENT),
            stat: BridgeStat::Actual,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Error(Errno::ACCESS),
            stat: BridgeStat::Actual,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::OtherDirectory(other.path().to_path_buf()),
            stat: BridgeStat::Actual,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::Error(Errno::IO),
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::WrongDevice,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::WrongInode,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::WrongType,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::WrongOwner,
        },
        InjectedRuntimeBridge {
            open: BridgeOpen::Requested,
            stat: BridgeStat::WrongMode,
        },
    ] {
        assert_preflight_failure_preserves_namespace(bridge);
    }
}

/// Catches a regression where canonical, NUL-containing, or widest-fd procfd
/// addresses are rejected only after `realm` has already been created.
#[test]
fn sockaddr_un_overflow_fails_before_mutation_without_fallback() {
    let _lock = umask_lock()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temporary = tempfile::tempdir().unwrap();
    let long_runtime = temporary.path().join("x".repeat(100));
    fs::create_dir(&long_runtime).unwrap();
    set_mode(&long_runtime, 0o700);
    endpoint_error(
        test_runtime_dir(&long_runtime)
            .unwrap()
            .prepare_server_endpoint(),
    );
    assert!(!long_runtime.join("realm").exists());

    let short_runtime = tempfile::Builder::new()
        .prefix("r")
        .tempdir_in("/tmp")
        .unwrap();
    set_mode(short_runtime.path(), 0o700);
    assert!(
        short_runtime
            .path()
            .join("realm/ctl.sock")
            .as_os_str()
            .as_bytes()
            .len()
            < 33
    );
    endpoint_error(
        test_runtime_dir(short_runtime.path())
            .unwrap()
            .prepare_server_endpoint_with(&InjectedRuntimeBridge::actual(), 33),
    );
    assert!(!short_runtime.path().join("realm").exists());

    let (_temporary, actual_runtime) = runtime_fixture();
    let nul_display = PathBuf::from(OsString::from_vec(b"/nul\0runtime".to_vec()));
    let runtime = crate::runtime::resolve_runtime_with(&nul_display, |_| {
        openat2(
            CWD,
            &actual_runtime,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
    })
    .unwrap();
    endpoint_error(runtime.prepare_server_endpoint_with(&InjectedRuntimeBridge::actual(), 108));
    assert!(!actual_runtime.join("realm").exists());
}
