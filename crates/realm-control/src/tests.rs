use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rustix::fs::{
    fcntl_getfl, fstat, openat2, statat, AtFlags, FileType, Mode, OFlags, ResolveFlags, Stat, CWD,
};
use rustix::io::{Errno, FdFlags};
use rustix::net::sockopt::socket_acceptconn;
use rustix::net::{bind as bind_socket, getsockname, SocketAddrUnix};
use rustix::process::{geteuid, umask};

use crate::{
    production_runtime_dir, test_runtime_dir, BoundControlEndpoint, IpcPathError, RuntimeDir,
    SocketEndpoint,
};

fn process_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    Requested { expected: PathBuf },
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
    fn requested(runtime: &RuntimeDir, stat: BridgeStat) -> Self {
        Self {
            open: BridgeOpen::Requested {
                expected: PathBuf::from(format!("/proc/self/fd/{}", runtime.raw_fd())),
            },
            stat,
        }
    }
}

#[derive(Clone)]
enum BridgeScenario {
    Requested(BridgeStat),
    ProcFdParent,
    OtherDirectory(PathBuf),
    Error(Errno),
}

impl BridgeScenario {
    fn bridge(&self, runtime: &RuntimeDir) -> InjectedRuntimeBridge {
        match self {
            Self::Requested(stat) => InjectedRuntimeBridge::requested(runtime, *stat),
            Self::ProcFdParent => InjectedRuntimeBridge {
                open: BridgeOpen::ProcFdParent,
                stat: BridgeStat::Actual,
            },
            Self::OtherDirectory(path) => InjectedRuntimeBridge {
                open: BridgeOpen::OtherDirectory(path.clone()),
                stat: BridgeStat::Actual,
            },
            Self::Error(error) => InjectedRuntimeBridge {
                open: BridgeOpen::Error(*error),
                stat: BridgeStat::Actual,
            },
        }
    }
}

impl crate::sys::RuntimeBridge for InjectedRuntimeBridge {
    fn open(&self, requested: &Path) -> rustix::io::Result<OwnedFd> {
        let path = match &self.open {
            BridgeOpen::Requested { expected } => {
                assert_eq!(requested, expected);
                requested
            }
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
    let _lock = process_test_lock();
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
    let _lock = process_test_lock();
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
    let _lock = process_test_lock();
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
    let _lock = process_test_lock();
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

fn assert_preflight_failure_preserves_namespace(scenario: BridgeScenario) {
    let (_temporary, runtime_path) = runtime_fixture();
    let runtime = test_runtime_dir(&runtime_path).unwrap();
    let bridge = scenario.bridge(&runtime);
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
    let bridge = scenario.bridge(&runtime);
    endpoint_error(runtime.prepare_server_endpoint_with(&bridge, 108));
    assert_eq!(entry_fingerprint(&socket_path), before);
}

/// Catches a regression where endpoint preparation inherits a hostile umask,
/// replaces or chmods an existing realm, or leaks its temporary umask.
#[test]
fn server_creates_realm_exactly_once_under_scoped_umask() {
    let _lock = process_test_lock();
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
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let runtime = test_runtime_dir(&runtime_path).unwrap();
    let bridge = InjectedRuntimeBridge::requested(&runtime, BridgeStat::Actual);
    let endpoint = runtime.prepare_server_endpoint_with(&bridge, 108).unwrap();
    assert_eq!(
        endpoint.path(),
        runtime_path.join("realm/ctl.sock").as_path()
    );
    assert!(runtime_path.join("realm").is_dir());

    assert_preflight_failure_preserves_namespace(BridgeScenario::ProcFdParent);
}

/// Catches regressions that ignore bridge open/stat failures or omit any of
/// the retained runtime identity, type, owner, and exact-mode comparisons.
#[test]
fn missing_inaccessible_or_mismatched_procfd_fails_before_mutation() {
    let _lock = process_test_lock();
    let other = tempfile::tempdir().unwrap();
    set_mode(other.path(), 0o700);

    for scenario in [
        BridgeScenario::Error(Errno::NOENT),
        BridgeScenario::Error(Errno::ACCESS),
        BridgeScenario::OtherDirectory(other.path().to_path_buf()),
        BridgeScenario::Requested(BridgeStat::Error(Errno::IO)),
        BridgeScenario::Requested(BridgeStat::WrongDevice),
        BridgeScenario::Requested(BridgeStat::WrongInode),
        BridgeScenario::Requested(BridgeStat::WrongType),
        BridgeScenario::Requested(BridgeStat::WrongOwner),
        BridgeScenario::Requested(BridgeStat::WrongMode),
    ] {
        assert_preflight_failure_preserves_namespace(scenario);
    }
}

/// Catches a regression where canonical, NUL-containing, or widest-fd procfd
/// addresses are rejected only after `realm` has already been created.
#[test]
fn sockaddr_un_overflow_fails_before_mutation_without_fallback() {
    let _lock = process_test_lock();
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
    let runtime = test_runtime_dir(short_runtime.path()).unwrap();
    let bridge = InjectedRuntimeBridge::requested(&runtime, BridgeStat::Actual);
    endpoint_error(runtime.prepare_server_endpoint_with(&bridge, 33));
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
    let bridge = InjectedRuntimeBridge::requested(&runtime, BridgeStat::Actual);
    endpoint_error(runtime.prepare_server_endpoint_with(&bridge, 108));
    assert!(!actual_runtime.join("realm").exists());
}

/// Catches a regression where an uncertain Linux connect or completion result
/// is treated as authority to reclaim a pathname.
#[test]
fn linux_stale_probe_completion_table_is_total() {
    let _lock = process_test_lock();
    use crate::endpoint::{
        classify_initial_probe, classify_poll_completion, PollCompletion, ProbeDecision,
    };

    for (case, result, expected) in [
        ("success", Ok(()), ProbeDecision::Preserve),
        (
            "immediate refusal",
            Err(Errno::CONNREFUSED),
            ProbeDecision::Stale,
        ),
        ("would block", Err(Errno::AGAIN), ProbeDecision::Poll),
        ("in progress", Err(Errno::INPROGRESS), ProbeDecision::Poll),
        (
            "unexpected already",
            Err(Errno::ALREADY),
            ProbeDecision::Preserve,
        ),
        (
            "other immediate error",
            Err(Errno::ACCESS),
            ProbeDecision::Preserve,
        ),
    ] {
        assert_eq!(classify_initial_probe(result), expected, "{case}");
    }

    for (case, completion, expected) in [
        ("timeout", PollCompletion::Timeout, ProbeDecision::Preserve),
        (
            "poll failure",
            PollCompletion::PollFailure,
            ProbeDecision::Preserve,
        ),
        (
            "ready successful completion",
            PollCompletion::Ready(Ok(Ok(()))),
            ProbeDecision::Preserve,
        ),
        (
            "ready refused completion",
            PollCompletion::Ready(Ok(Err(Errno::CONNREFUSED))),
            ProbeDecision::Stale,
        ),
        (
            "ready other completion error",
            PollCompletion::Ready(Ok(Err(Errno::ACCESS))),
            ProbeDecision::Preserve,
        ),
        (
            "socket error query failure",
            PollCompletion::Ready(Err(Errno::IO)),
            ProbeDecision::Preserve,
        ),
    ] {
        assert_eq!(classify_poll_completion(completion), expected, "{case}");
    }
}

#[derive(Debug, PartialEq, Eq)]
struct PathIdentity {
    device: u64,
    inode: u64,
    mode: u32,
}

fn path_identity(path: &Path) -> PathIdentity {
    let metadata = fs::symlink_metadata(path).unwrap();
    PathIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
    }
}

fn result_error<T>(result: Result<T, IpcPathError>) -> IpcPathError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected operation to fail"),
    }
}

fn bind_control_listener(path: &Path, mode: u32) -> UnixListener {
    let listener = UnixListener::bind(path).unwrap();
    set_mode(path, mode);
    listener
}

/// Catches a regression where ctl.sock is inspected before the private,
/// independent realm-directory open-file description has won its flock.
#[test]
fn singleton_lock_precedes_socket_inspection() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let first = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let first_lock = first.acquire_lock_and_reclaim().unwrap();

    let socket_path = runtime_path.join("realm/ctl.sock");
    fs::write(&socket_path, b"unsafe entry must not be inspected").unwrap();
    set_mode(&socket_path, 0o600);
    let before = entry_fingerprint(&socket_path);
    let second = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();

    assert!(matches!(
        result_error(second.acquire_lock_and_reclaim()),
        IpcPathError::EndpointInUse
    ));
    assert_eq!(entry_fingerprint(&socket_path), before);
    drop(first_lock);
}

/// Catches regressions that accept an unsafe lock-directory view or alter an
/// existing ctl.sock whose type, owner, or exact mode is unsafe.
#[test]
fn unsafe_realm_and_socket_entries_are_preserved() {
    let _lock = process_test_lock();

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let realm_path = runtime_path.join("realm");
    set_mode(&realm_path, 0o755);
    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::UnsafeRealmDirectory
    ));
    assert_eq!(fs::metadata(&realm_path).unwrap().mode() & 0o777, 0o755);

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    fs::write(&socket_path, b"not a socket").unwrap();
    set_mode(&socket_path, 0o600);
    let before = entry_fingerprint(&socket_path);
    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::UnsafeSocketEntry
    ));
    assert_eq!(entry_fingerprint(&socket_path), before);

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let realm_path = runtime_path.join("realm");
    let socket_path = realm_path.join("ctl.sock");
    let target_path = realm_path.join("target");
    fs::write(&target_path, b"symlink target").unwrap();
    symlink(&target_path, &socket_path).unwrap();
    let before = path_identity(&socket_path);
    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::UnsafeSocketEntry
    ));
    assert_eq!(path_identity(&socket_path), before);

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let _listener = bind_control_listener(&socket_path, 0o660);
    let before = path_identity(&socket_path);
    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::UnsafeSocketEntry
    ));
    assert_eq!(path_identity(&socket_path), before);
    set_mode(&socket_path, 0o600);

    let mut socket_stat = statat(
        endpoint.realm_dir().as_fd(),
        "ctl.sock",
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .unwrap();
    socket_stat.st_uid = geteuid().as_raw().wrapping_add(1);
    assert!(matches!(
        crate::endpoint::validate_socket_stat(&socket_stat, geteuid().as_raw()),
        Err(IpcPathError::UnsafeSocketEntry)
    ));

    let expected_realm = fstat(endpoint.realm_dir().as_fd()).unwrap();
    let mut wrong_device = expected_realm;
    wrong_device.st_dev = wrong_device.st_dev.wrapping_add(1);
    let mut wrong_inode = expected_realm;
    wrong_inode.st_ino = wrong_inode.st_ino.wrapping_add(1);
    let mut wrong_type = expected_realm;
    wrong_type.st_mode = FileType::RegularFile.as_raw_mode() | 0o700;
    let mut wrong_owner = expected_realm;
    wrong_owner.st_uid = wrong_owner.st_uid.wrapping_add(1);
    let mut wrong_mode = expected_realm;
    wrong_mode.st_mode = FileType::Directory.as_raw_mode() | 0o755;
    for (case, candidate) in [
        ("device", wrong_device),
        ("inode", wrong_inode),
        ("type", wrong_type),
        ("owner", wrong_owner),
        ("mode", wrong_mode),
    ] {
        assert!(
            matches!(
                crate::endpoint::validate_lock_stat(
                    &expected_realm,
                    &candidate,
                    geteuid().as_raw(),
                    FdFlags::CLOEXEC,
                ),
                Err(IpcPathError::UnsafeRealmDirectory)
            ),
            "{case}"
        );
    }
    assert!(matches!(
        crate::endpoint::validate_lock_stat(
            &expected_realm,
            &expected_realm,
            geteuid().as_raw(),
            FdFlags::empty(),
        ),
        Err(IpcPathError::UnsafeRealmDirectory)
    ));
}

/// Catches a regression where a reachable listener is reclaimed or a refused
/// unchanged socket is preserved forever.
#[test]
fn live_listener_is_preserved_and_verified_refusal_is_reclaimed() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let listener = bind_control_listener(&socket_path, 0o600);
    let live_identity = path_identity(&socket_path);

    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::EndpointInUse
    ));
    assert_eq!(path_identity(&socket_path), live_identity);

    drop(listener);
    let _lock_owner = endpoint.acquire_lock_and_reclaim().unwrap();
    assert!(!socket_path.exists());
}

/// Catches a regression where the stale probe's original pathname identity is
/// not compared with a replacement made before the final no-follow stat.
#[test]
fn stale_reclaim_rechecks_identity_before_unlink() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let stale_listener = bind_control_listener(&socket_path, 0o600);
    let stale_identity = path_identity(&socket_path);
    drop(stale_listener);

    let replacement_path = runtime_path.join("realm/replacement.sock");
    let mut replacement = None;
    let error = result_error(endpoint.acquire_lock_and_reclaim_with(|| {
        replacement = Some(bind_control_listener(&replacement_path, 0o600));
        fs::remove_file(&socket_path).unwrap();
        fs::rename(&replacement_path, &socket_path).unwrap();
    }));

    assert!(matches!(error, IpcPathError::EndpointInUse));
    assert!(replacement.is_some());
    let replacement_identity = path_identity(&socket_path);
    assert_ne!(replacement_identity, stale_identity);
}

/// Catches a regression where endpoint validation resamples the ambient euid
/// instead of using the authority retained by `RealmDir` at resolution time.
#[test]
fn endpoint_validation_uses_retained_realm_euid() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let mut endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let _listener = bind_control_listener(&socket_path, 0o600);
    let before = path_identity(&socket_path);
    let socket_stat = statat(
        endpoint.realm_dir().as_fd(),
        "ctl.sock",
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .unwrap();
    let retained_euid = geteuid().as_raw().wrapping_add(1);
    endpoint.set_retained_euid_for_test(retained_euid);

    assert_eq!(endpoint.realm_dir().retained_euid(), retained_euid);
    assert!(matches!(
        endpoint.validate_socket_stat(&socket_stat),
        Err(IpcPathError::UnsafeSocketEntry)
    ));
    assert!(matches!(
        result_error(endpoint.acquire_lock_and_reclaim()),
        IpcPathError::UnsafeRealmDirectory
    ));
    assert_eq!(path_identity(&socket_path), before);
}

enum InjectedBind {
    Actual,
    Error(Errno),
}

enum InjectedStat {
    Actual,
    Error(Errno),
    ReplaceWithFile(PathBuf),
}

enum InjectedVerification {
    Actual,
    Error(Errno),
    ReplaceWithFileThenError(PathBuf, Errno),
}

struct InjectedBindOperations {
    bind: InjectedBind,
    stat: InjectedStat,
    getsockname: InjectedVerification,
    acceptconn_error: Option<Errno>,
}

impl InjectedBindOperations {
    fn actual() -> Self {
        Self {
            bind: InjectedBind::Actual,
            stat: InjectedStat::Actual,
            getsockname: InjectedVerification::Actual,
            acceptconn_error: None,
        }
    }
}

fn replace_with_file(path: &Path) {
    fs::remove_file(path).unwrap();
    fs::write(path, b"replacement must survive").unwrap();
    set_mode(path, 0o600);
}

impl crate::endpoint::BindOperations for InjectedBindOperations {
    fn bind(&self, socket: BorrowedFd<'_>, address: &SocketAddrUnix) -> rustix::io::Result<()> {
        match self.bind {
            InjectedBind::Actual => bind_socket(socket, address),
            InjectedBind::Error(error) => Err(error),
        }
    }

    fn post_bind_stat(&self, realm_dir: BorrowedFd<'_>) -> rustix::io::Result<Stat> {
        match &self.stat {
            InjectedStat::Actual => statat(realm_dir, "ctl.sock", AtFlags::SYMLINK_NOFOLLOW),
            InjectedStat::Error(error) => Err(*error),
            InjectedStat::ReplaceWithFile(path) => {
                replace_with_file(path);
                statat(realm_dir, "ctl.sock", AtFlags::SYMLINK_NOFOLLOW)
            }
        }
    }

    fn getsockname(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<SocketAddrUnix> {
        match &self.getsockname {
            InjectedVerification::Actual => getsockname(socket)?.try_into(),
            InjectedVerification::Error(error) => Err(*error),
            InjectedVerification::ReplaceWithFileThenError(path, error) => {
                replace_with_file(path);
                Err(*error)
            }
        }
    }

    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool> {
        match self.acceptconn_error {
            Some(error) => Err(error),
            None => socket_acceptconn(socket),
        }
    }
}

fn bind_error(result: Result<BoundControlEndpoint, IpcPathError>) -> IpcPathError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected bind to fail"),
    }
}

fn assert_io_errno(error: IpcPathError, expected: Errno) {
    match error {
        IpcPathError::Io(error) => {
            assert_eq!(error.raw_os_error(), Some(expected.raw_os_error()));
        }
        other => panic!("expected Io({expected:?}), got {other:?}"),
    }
}

/// Catches a regression where the singleton lock is released before the
/// non-listening bound capability has completed its ownership lifetime.
#[test]
fn singleton_lock_protects_the_prelisten_state() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let first = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let identity = path_identity(&socket_path);

    let second = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    assert!(matches!(
        bind_error(second.bind()),
        IpcPathError::EndpointInUse
    ));
    assert_eq!(path_identity(&socket_path), identity);
    drop(first);
}

/// Catches regressions that bind through the display path, omit either atomic
/// fd flag, accept the wrong pathname identity, or listen before activation.
#[test]
fn bound_capability_has_exact_path_fd_and_address_properties() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let display_stat = fs::symlink_metadata(&socket_path).unwrap();
    let relative_stat = statat(
        bound.realm_dir().as_fd(),
        "ctl.sock",
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .unwrap();

    assert_eq!(bound.endpoint().path(), socket_path.as_path());
    assert_eq!(
        bound.realm_dir().as_fd().as_raw_fd(),
        bound.endpoint().realm_dir().as_fd().as_raw_fd()
    );
    assert_eq!(
        FileType::from_raw_mode(display_stat.mode()),
        FileType::Socket
    );
    assert_eq!(display_stat.uid(), geteuid().as_raw());
    assert_eq!(display_stat.mode() & 0o777, 0o600);
    assert_eq!(display_stat.dev(), relative_stat.st_dev);
    assert_eq!(display_stat.ino(), relative_stat.st_ino);

    let socket = bound.socket_fd_for_test();
    assert!(fcntl_getfl(socket).unwrap().contains(OFlags::NONBLOCK));
    assert!(rustix::io::fcntl_getfd(socket)
        .unwrap()
        .contains(FdFlags::CLOEXEC));
    let actual_address: SocketAddrUnix = getsockname(socket).unwrap().try_into().unwrap();
    assert_eq!(&actual_address, bound.bind_address_for_test());
    assert!(!socket_acceptconn(socket).unwrap());
}

/// Catches a regression where a successfully read unsafe post-bind entry is
/// cleaned up despite being a detected replacement rather than owned state.
#[test]
fn post_bind_bad_properties_preserve_detected_replacement() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        stat: InjectedStat::ReplaceWithFile(socket_path.clone()),
        ..InjectedBindOperations::actual()
    };

    assert!(matches!(
        bind_error(endpoint.bind_with(&operations)),
        IpcPathError::UnsafeSocketEntry
    ));
    assert_eq!(fs::read(&socket_path).unwrap(), b"replacement must survive");
    assert_eq!(
        fs::symlink_metadata(&socket_path).unwrap().mode() & 0o777,
        0o600
    );
}

/// Catches a regression where a post-bind stat syscall error is collapsed or
/// authorizes cleanup of a pathname whose identity was never retained.
#[test]
fn post_bind_stat_error_preserves_errno_and_pathname() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        stat: InjectedStat::Error(Errno::IO),
        ..InjectedBindOperations::actual()
    };

    assert_io_errno(bind_error(endpoint.bind_with(&operations)), Errno::IO);
    let metadata = fs::symlink_metadata(&socket_path).unwrap();
    assert_eq!(FileType::from_raw_mode(metadata.mode()), FileType::Socket);
    assert_eq!(metadata.mode() & 0o777, 0o600);
}

/// Catches a regression where the bind-specific 0177 umask leaks on either
/// the successful path or an injected bind syscall error.
#[test]
fn bind_scoped_umask_restores_after_success_and_error() {
    let _lock = process_test_lock();
    let (_successful_temporary, successful_runtime_path) = runtime_fixture();
    let (_error_temporary, error_runtime_path) = runtime_fixture();
    let _restore = ProcessUmaskRestore::replace(0o777);

    let bound = test_runtime_dir(&successful_runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(current_umask(), 0o777);
    assert_eq!(
        fs::symlink_metadata(successful_runtime_path.join("realm/ctl.sock"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    drop(bound);

    let endpoint = test_runtime_dir(&error_runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let operations = InjectedBindOperations {
        bind: InjectedBind::Error(Errno::IO),
        ..InjectedBindOperations::actual()
    };
    assert_io_errno(bind_error(endpoint.bind_with(&operations)), Errno::IO);
    assert_eq!(current_umask(), 0o777);
    assert!(!error_runtime_path.join("realm/ctl.sock").exists());
}

/// Catches regressions where failures after ownership construction either
/// leak the owned entry or delete a replacement detected during cleanup.
#[test]
fn post_bind_verification_failures_use_ownership_safe_cleanup() {
    let _lock = process_test_lock();

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        getsockname: InjectedVerification::Error(Errno::IO),
        ..InjectedBindOperations::actual()
    };
    assert_io_errno(bind_error(endpoint.bind_with(&operations)), Errno::IO);
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        acceptconn_error: Some(Errno::IO),
        ..InjectedBindOperations::actual()
    };
    assert_io_errno(bind_error(endpoint.bind_with(&operations)), Errno::IO);
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        getsockname: InjectedVerification::ReplaceWithFileThenError(socket_path.clone(), Errno::IO),
        ..InjectedBindOperations::actual()
    };
    assert_io_errno(bind_error(endpoint.bind_with(&operations)), Errno::IO);
    assert_eq!(fs::read(&socket_path).unwrap(), b"replacement must survive");
}

/// Catches regressions where Drop leaks an unchanged entry, leaves the socket
/// fd open, or removes a matching-property replacement with another identity.
#[test]
fn bound_drop_closes_and_removes_only_matching_identity() {
    let _lock = process_test_lock();

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let socket_fd_path = PathBuf::from(format!(
        "/proc/self/fd/{}",
        bound.socket_fd_for_test().as_raw_fd()
    ));
    assert!(socket_fd_path.exists());
    drop(bound);
    assert!(!socket_fd_path.exists());
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let replacement_path = runtime_path.join("realm/replacement.sock");
    let _replacement = bind_control_listener(&replacement_path, 0o600);
    fs::remove_file(&socket_path).unwrap();
    fs::rename(&replacement_path, &socket_path).unwrap();
    let replacement_identity = path_identity(&socket_path);

    drop(bound);
    assert_eq!(path_identity(&socket_path), replacement_identity);
}

fn kill_and_reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for_exit_or_kill(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            kill_and_reap(child);
            panic!("exec helper did not exit within {timeout:?}");
        }
        std::thread::yield_now();
    }
}

/// Catches a regression where the private independent singleton-lock fd loses
/// CLOEXEC and remains locked by a long-lived exec-launched child.
#[test]
fn singleton_lock_fd_is_private_directory_cloexec_and_not_inherited_across_exec() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let lock_fd = bound.lock_fd_for_test();
    let lock_stat = fstat(lock_fd).unwrap();
    let realm_stat = fstat(bound.realm_dir().as_fd()).unwrap();
    assert_ne!(lock_fd.as_raw_fd(), bound.realm_dir().as_fd().as_raw_fd());
    assert_eq!(
        (lock_stat.st_dev, lock_stat.st_ino),
        (realm_stat.st_dev, realm_stat.st_ino)
    );
    assert_eq!(
        FileType::from_raw_mode(lock_stat.st_mode),
        FileType::Directory
    );
    assert!(rustix::io::fcntl_getfd(lock_fd)
        .unwrap()
        .contains(FdFlags::CLOEXEC));

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "tests::singleton_lock_exec_child",
            "--nocapture",
        ])
        .env("REALM_CONTROL_LOCK_EXEC_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let readiness = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut observed = String::new();
        let mut ready_tx = Some(ready_tx);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    if let Some(ready_tx) = ready_tx.take() {
                        let _ = ready_tx.send(Err(observed.clone()));
                    }
                    return observed;
                }
                Ok(_) => {
                    observed.push_str(&line);
                    if line.contains("REALM_CONTROL_LOCK_EXEC_READY") {
                        if let Some(ready_tx) = ready_tx.take() {
                            let _ = ready_tx.send(Ok(observed.clone()));
                        }
                    }
                }
                Err(error) => {
                    let diagnostic = format!("{observed}\nread error: {error}");
                    if let Some(ready_tx) = ready_tx.take() {
                        let _ = ready_tx.send(Err(diagnostic.clone()));
                    }
                    return diagnostic;
                }
            }
        }
    });

    match ready_rx.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(_)) => {}
        Ok(Err(output)) => {
            kill_and_reap(&mut child);
            let _ = readiness.join();
            panic!("exec helper exited before readiness: {output}");
        }
        Err(error) => {
            kill_and_reap(&mut child);
            let _ = readiness.join();
            panic!("timed out waiting for exec helper readiness: {error}");
        }
    }

    drop(bound);
    let second = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    drop(child.stdin.take());
    let status = wait_for_exit_or_kill(&mut child, Duration::from_secs(3));
    let output = readiness.join().unwrap();
    assert!(status.success(), "exec helper failed: {status}");
    assert!(output.contains("REALM_CONTROL_LOCK_EXEC_READY"));
    drop(second);
}

#[test]
#[ignore = "exec helper for singleton-lock CLOEXEC coverage"]
fn singleton_lock_exec_child() {
    let _lock = process_test_lock();
    if std::env::var_os("REALM_CONTROL_LOCK_EXEC_CHILD").is_none() {
        return;
    }
    println!("REALM_CONTROL_LOCK_EXEC_READY");
    std::io::stdout().flush().unwrap();
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).unwrap();
}
