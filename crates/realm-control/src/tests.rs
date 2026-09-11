use std::cell::Cell;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rustix::fs::{
    fcntl_getfl, fstat, openat2, statat, AtFlags, FileType, Mode, OFlags, ResolveFlags, Stat, CWD,
};
use rustix::io::{Errno, FdFlags};
use rustix::net::sockopt::socket_acceptconn;
use rustix::net::{
    bind as bind_socket, getsockname, listen as listen_socket, SendFlags, SocketAddrUnix,
};
use rustix::process::{geteuid, umask};

use crate::protocol::{ConnectionMachine, ConnectionPhase, MachineAction, MachineClose};
use crate::{
    production_runtime_dir, test_runtime_dir, ActiveControlListener, BoundControlEndpoint,
    ClientError, ClientPhase, ConnectionId, ControlAction, ControlError, ControlServer,
    ControlToken, IpcPathError, ReadyEvent, RuntimeDir, SocketEndpoint, TestClientConnect,
    TestClientOperations, TestClientPoll, TestClientReceive, TestClientSend, TestClientSocketError,
    TestPeerCredential, TestReceive, TestSend,
};
use realm_core::ipc::{Event, Request, Response, MAX_FRAME_BYTES};
use realm_core::state::RealmState;

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

/// Catches a client resolution path that creates the fixed realm directory or
/// classifies its absence as an unsafe path.
#[test]
fn client_endpoint_missing_realm_is_retryable_and_creates_nothing() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();

    let error = endpoint.connect("realmctl").unwrap_err();

    assert!(matches!(error, ClientError::MissingRealm));
    assert!(error.is_retryable());
    assert!(!runtime_path.join("realm").exists());
}

/// Catches a reusable endpoint rereading the environment or reopening the
/// caller-visible absolute path after its runtime capability was retained.
#[test]
fn client_endpoint_ignores_environment_and_path_replacement() {
    let _lock = process_test_lock();
    let _actual_environment = EnvironmentRestore::capture();
    let (temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();

    let retained_path = temporary.path().join("retained-runtime");
    fs::rename(&runtime_path, &retained_path).unwrap();
    fs::create_dir(&runtime_path).unwrap();
    set_mode(&runtime_path, 0o700);
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    fs::create_dir(retained_path.join("realm")).unwrap();
    set_mode(&retained_path.join("realm"), 0o600);
    std::env::set_var("XDG_RUNTIME_DIR", &runtime_path);
    std::env::set_var("REALM_SOCKET", runtime_path.join("realm/elsewhere.sock"));

    let error = endpoint.connect("bar").unwrap_err();

    assert!(matches!(
        error,
        ClientError::Path(IpcPathError::UnsafeRealmDirectory)
    ));
}

/// Catches retaining one realm descriptor across attempts instead of reopening
/// the fixed descendant relative to the retained runtime descriptor each time.
#[test]
fn client_endpoint_reopens_realm_for_every_single_attempt() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();

    assert!(matches!(
        endpoint.connect("realmctl").unwrap_err(),
        ClientError::MissingRealm
    ));

    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o600);
    assert!(matches!(
        endpoint.connect("realmctl").unwrap_err(),
        ClientError::Path(IpcPathError::UnsafeRealmDirectory)
    ));

    set_mode(&runtime_path.join("realm"), 0o700);
    assert!(matches!(
        endpoint.connect("realmctl").unwrap_err(),
        ClientError::MissingRealm
    ));
}

/// Catches collapsing a client phase or version field, and prevents future
/// callers from accidentally broadening startup retry beyond absence/refusal.
#[test]
fn client_error_preserves_all_phases_versions_and_exact_retryability() {
    let phases = [
        ClientPhase::Connect,
        ClientPhase::HelloWrite,
        ClientPhase::HelloRead,
        ClientPhase::RequestWrite,
        ClientPhase::ResponseRead,
        ClientPhase::SubscribeWrite,
        ClientPhase::InitialState,
        ClientPhase::SubscriptionEvent,
    ];
    assert_eq!(phases[0], ClientPhase::Connect);
    assert_eq!(phases[1], ClientPhase::HelloWrite);
    assert_eq!(phases[2], ClientPhase::HelloRead);
    assert_eq!(phases[3], ClientPhase::RequestWrite);
    assert_eq!(phases[4], ClientPhase::ResponseRead);
    assert_eq!(phases[5], ClientPhase::SubscribeWrite);
    assert_eq!(phases[6], ClientPhase::InitialState);
    assert_eq!(phases[7], ClientPhase::SubscriptionEvent);

    let mismatch = ClientError::VersionMismatch {
        client: 17,
        server: 23,
    };
    match mismatch {
        ClientError::VersionMismatch { client, server } => {
            assert_eq!(client, 17);
            assert_eq!(server, 23);
        }
        other => panic!("unexpected client error: {other:?}"),
    }

    let terminal = [
        ClientError::Path(IpcPathError::UnsafeRealmDirectory),
        ClientError::VersionMismatch {
            client: 1,
            server: 2,
        },
        ClientError::Timeout {
            phase: ClientPhase::Connect,
        },
        ClientError::FrameTooLarge {
            phase: ClientPhase::HelloRead,
        },
        ClientError::InvalidRequest,
        ClientError::UnexpectedResponse {
            phase: ClientPhase::ResponseRead,
        },
        ClientError::MalformedResponse {
            phase: ClientPhase::InitialState,
        },
        ClientError::Eof {
            phase: ClientPhase::SubscriptionEvent,
        },
        ClientError::Io {
            phase: ClientPhase::Connect,
            source: std::io::Error::from(Errno::IO),
        },
    ];
    assert!(ClientError::MissingRealm.is_retryable());
    assert!(ClientError::Refused.is_retryable());
    for error in terminal {
        assert!(
            !error.is_retryable(),
            "unexpected retryable error: {error:?}"
        );
    }
}

fn client_listener_fixture() -> (tempfile::TempDir, crate::ClientEndpoint, UnixListener) {
    let (temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let listener = UnixListener::bind(runtime_path.join("realm/ctl.sock")).unwrap();
    (temporary, endpoint, listener)
}

fn read_test_request(stream: &mut UnixStream) -> Request {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    realm_core::ipc::decode(std::str::from_utf8(&bytes).unwrap()).unwrap()
}

fn write_test_response(stream: &mut UnixStream, value: &Response) {
    stream
        .write_all(realm_core::ipc::encode(value).unwrap().as_bytes())
        .unwrap();
}

fn matching_hello() -> Response {
    Response::Hello {
        version: realm_core::ipc::PROTOCOL_VERSION,
        session: "test-session".into(),
    }
}

fn assert_client_io(error: ClientError, phase: ClientPhase, expected: Errno) {
    match error {
        ClientError::Io {
            phase: actual,
            source,
        } => {
            assert_eq!(actual, phase);
            assert_eq!(source.raw_os_error(), Some(expected.raw_os_error()));
        }
        other => panic!("unexpected client error: {other:?}"),
    }
}

/// Catches skipping Hello, using the wrong client name, or creating a stream
/// without atomic NONBLOCK/CLOEXEC on the immediate-success path.
#[test]
fn single_attempt_client_immediate_connect_sends_named_hello_on_cloexec_nonblocking_stream() {
    let _lock = process_test_lock();
    let (_temporary, endpoint, listener) = client_listener_fixture();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_test_request(&mut stream);
        write_test_response(&mut stream, &matching_hello());
        request
    });

    let client = endpoint.connect("realm-bar").unwrap();

    assert_eq!(
        peer.join().unwrap(),
        Request::Hello {
            version: realm_core::ipc::PROTOCOL_VERSION,
            client: "realm-bar".into(),
        }
    );
    assert!(fcntl_getfl(client.fd_for_test())
        .unwrap()
        .contains(OFlags::NONBLOCK));
    assert!(rustix::io::fcntl_getfd(client.fd_for_test())
        .unwrap()
        .contains(FdFlags::CLOEXEC));
}

/// Catches an incomplete in-progress connect table, failure to inspect
/// SO_ERROR, or a second connect syscall after readiness.
#[test]
fn single_attempt_client_in_progress_so_error_table_is_total_and_connects_once() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();

    for initial in [Errno::INPROGRESS, Errno::AGAIN] {
        for completion in [
            TestClientSocketError::Connected,
            TestClientSocketError::Socket(Errno::NOENT),
            TestClientSocketError::Socket(Errno::NOTDIR),
            TestClientSocketError::Socket(Errno::CONNREFUSED),
            TestClientSocketError::Socket(Errno::IO),
            TestClientSocketError::Lookup(Errno::NOENT),
            TestClientSocketError::Lookup(Errno::BADF),
        ] {
            let operations = TestClientOperations::new(base);
            operations.push_connect(TestClientConnect::Error(initial));
            operations.push_poll(TestClientPoll::ReadyAfter(Duration::ZERO));
            operations.push_socket_error(completion);
            if matches!(completion, TestClientSocketError::Connected) {
                operations.push_receive(TestClientReceive::BytesAfter(
                    realm_core::ipc::encode(&matching_hello())
                        .unwrap()
                        .into_bytes(),
                    Duration::ZERO,
                ));
            }

            let result = endpoint.connect_with_test_operations("test", operations.clone());

            match completion {
                TestClientSocketError::Connected => assert!(result.is_ok()),
                TestClientSocketError::Socket(Errno::NOENT | Errno::NOTDIR) => {
                    assert!(matches!(result, Err(ClientError::MissingRealm)));
                }
                TestClientSocketError::Socket(Errno::CONNREFUSED) => {
                    assert!(matches!(result, Err(ClientError::Refused)));
                }
                TestClientSocketError::Socket(error) | TestClientSocketError::Lookup(error) => {
                    assert_client_io(result.unwrap_err(), ClientPhase::Connect, error);
                }
            }
            assert_eq!(operations.connect_calls(), 1);
        }
    }
}

/// Catches retrying terminal connect errors, repeating connect after EINTR, or
/// sliding the hard 100 ms deadline while poll is interrupted.
#[test]
fn single_attempt_client_connect_terminal_errors_and_hard_timeout_are_exact() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();

    for immediate in [Errno::INTR, Errno::ALREADY] {
        let operations = TestClientOperations::new(base);
        operations.push_connect(TestClientConnect::Error(immediate));
        let error = endpoint
            .connect_with_test_operations("test", operations.clone())
            .unwrap_err();
        assert_client_io(error, ClientPhase::Connect, immediate);
        assert_eq!(operations.connect_calls(), 1);
        assert!(operations.poll_timeouts().is_empty());
    }

    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Error(Errno::INPROGRESS));
    operations.push_poll(TestClientPoll::ErrorAfter(
        Errno::INTR,
        Duration::from_millis(60),
    ));
    operations.push_poll(TestClientPoll::TimeoutAfter(Duration::from_millis(40)));

    assert!(matches!(
        endpoint.connect_with_test_operations("test", operations.clone()),
        Err(ClientError::Timeout {
            phase: ClientPhase::Connect
        })
    ));
    assert_eq!(operations.connect_calls(), 1);
    assert_eq!(
        operations.poll_timeouts(),
        vec![
            Some(Duration::from_millis(100)),
            Some(Duration::from_millis(40))
        ]
    );

    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Error(Errno::INPROGRESS));
    operations.push_poll(TestClientPoll::ReadyAfter(Duration::from_millis(100)));
    operations.push_socket_error(TestClientSocketError::Connected);
    operations.push_receive(TestClientReceive::BytesAfter(
        realm_core::ipc::encode(&matching_hello())
            .unwrap()
            .into_bytes(),
        Duration::ZERO,
    ));
    assert!(matches!(
        endpoint.connect_with_test_operations("test", operations.clone()),
        Err(ClientError::Timeout {
            phase: ClientPhase::Connect
        })
    ));
    assert_eq!(operations.connect_calls(), 1);
}

/// Catches separate write/read timers, sliding readiness waits, or omission of
/// MSG_NOSIGNAL on client frames.
#[test]
fn client_hello_uses_one_hard_two_second_deadline_and_msg_nosignal() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();
    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Success);
    operations.push_send(TestClientSend::ErrorAfter(
        Errno::AGAIN,
        Duration::from_millis(1_500),
    ));
    operations.push_poll(TestClientPoll::ReadyAfter(Duration::from_millis(400)));
    operations.push_receive(TestClientReceive::ErrorAfter(Errno::AGAIN, Duration::ZERO));
    operations.push_poll(TestClientPoll::TimeoutAfter(Duration::from_millis(100)));

    assert!(matches!(
        endpoint.connect_with_test_operations("deadline-test", operations.clone()),
        Err(ClientError::Timeout {
            phase: ClientPhase::HelloRead
        })
    ));
    assert_eq!(
        operations.poll_timeouts(),
        vec![
            Some(Duration::from_millis(500)),
            Some(Duration::from_millis(100))
        ]
    );
    assert!(operations
        .send_flags()
        .iter()
        .all(|flags| flags.contains(SendFlags::NOSIGNAL)));
    let hello: Request =
        realm_core::ipc::decode(std::str::from_utf8(&operations.sent_bytes()).unwrap()).unwrap();
    assert_eq!(
        hello,
        Request::Hello {
            version: realm_core::ipc::PROTOCOL_VERSION,
            client: "deadline-test".into()
        }
    );
}

/// Catches collapsing version refusal, malformed JSON, wrong response kind,
/// EOF, or an impossible full prefix into a generic I/O error.
#[test]
fn client_hello_response_errors_are_phase_specific_and_bounded() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();

    let cases = [
        (
            realm_core::ipc::encode(&Response::Hello {
                version: 99,
                session: "future".into(),
            })
            .unwrap()
            .into_bytes(),
            0,
        ),
        (b"{not-json}\n".to_vec(), 1),
        (
            realm_core::ipc::encode(&Response::Ok).unwrap().into_bytes(),
            2,
        ),
        (vec![b'x'; MAX_FRAME_BYTES], 3),
    ];

    for (bytes, case) in cases {
        let operations = TestClientOperations::new(base);
        operations.push_connect(TestClientConnect::Success);
        operations.push_receive(TestClientReceive::BytesAfter(bytes, Duration::ZERO));
        let error = endpoint
            .connect_with_test_operations("test", operations)
            .unwrap_err();
        match case {
            0 => assert!(matches!(
                error,
                ClientError::VersionMismatch {
                    client: realm_core::ipc::PROTOCOL_VERSION,
                    server: 99
                }
            )),
            1 => assert!(matches!(
                error,
                ClientError::MalformedResponse {
                    phase: ClientPhase::HelloRead
                }
            )),
            2 => assert!(matches!(
                error,
                ClientError::UnexpectedResponse {
                    phase: ClientPhase::HelloRead
                }
            )),
            3 => assert!(matches!(
                error,
                ClientError::FrameTooLarge {
                    phase: ClientPhase::HelloRead
                }
            )),
            _ => unreachable!(),
        }
    }

    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Success);
    operations.push_receive(TestClientReceive::EofAfter(Duration::ZERO));
    assert!(matches!(
        endpoint.connect_with_test_operations("test", operations),
        Err(ClientError::Eof {
            phase: ClientPhase::HelloRead
        })
    ));
}

/// Catches sending locally invalid requests, resetting the request deadline
/// between write/read, or treating an application Error response as transport failure.
#[test]
fn client_request_is_local_forbidden_or_one_bounded_exchange() {
    let _lock = process_test_lock();
    let (_temporary, endpoint, listener) = client_listener_fixture();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_test_request(&mut stream);
        write_test_response(&mut stream, &matching_hello());
        let request = read_test_request(&mut stream);
        write_test_response(
            &mut stream,
            &Response::Error {
                message: "application refusal".into(),
            },
        );
        request
    });
    let mut client = endpoint.connect("realmctl").unwrap();

    assert!(matches!(
        client.request(Request::Hello {
            version: 1,
            client: "duplicate".into()
        }),
        Err(ClientError::InvalidRequest)
    ));
    assert!(matches!(
        client.request(Request::Subscribe),
        Err(ClientError::InvalidRequest)
    ));
    assert_eq!(
        client.request(Request::GetState).unwrap(),
        Response::Error {
            message: "application refusal".into()
        }
    );
    assert_eq!(peer.join().unwrap(), Request::GetState);
}

/// Catches a request read timer starting after its write or moving after EINTR.
#[test]
fn client_request_write_and_response_share_one_hard_deadline() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();
    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Success);
    operations.push_receive(TestClientReceive::BytesAfter(
        realm_core::ipc::encode(&matching_hello())
            .unwrap()
            .into_bytes(),
        Duration::ZERO,
    ));
    let mut client = endpoint
        .connect_with_test_operations("test", operations.clone())
        .unwrap();
    operations.push_send(TestClientSend::ErrorAfter(
        Errno::AGAIN,
        Duration::from_millis(1_000),
    ));
    operations.push_poll(TestClientPoll::ReadyAfter(Duration::from_millis(900)));
    operations.push_receive(TestClientReceive::ErrorAfter(
        Errno::INTR,
        Duration::from_millis(100),
    ));

    assert!(matches!(
        client.request(Request::GetState),
        Err(ClientError::Timeout {
            phase: ClientPhase::ResponseRead
        })
    ));
}

/// Catches returning Subscribe before the initial State, losing a coalesced
/// frame already read with it, or yielding Shutdown more than once.
#[test]
fn client_subscribe_returns_initial_state_and_yields_shutdown_once() {
    let _lock = process_test_lock();
    let (_temporary, endpoint, listener) = client_listener_fixture();
    let initial = protocol_state(41);
    let expected = initial.clone();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_test_request(&mut stream);
        write_test_response(&mut stream, &matching_hello());
        assert_eq!(read_test_request(&mut stream), Request::Subscribe);
        let mut frames = realm_core::ipc::encode(&Event::State(Box::new(initial))).unwrap();
        frames.push_str(&realm_core::ipc::encode(&Event::Shutdown).unwrap());
        stream.write_all(frames.as_bytes()).unwrap();
    });
    let client = endpoint.connect("realm-bar").unwrap();

    let mut subscription = client.subscribe().unwrap();
    assert!(matches!(
        subscription.next(),
        Some(Ok(Event::State(state))) if *state == expected
    ));
    assert!(matches!(subscription.next(), Some(Ok(Event::Shutdown))));
    assert!(subscription.next().is_none());
    peer.join().unwrap();
}

/// Catches bounding empty subscription idle or sliding the two-second partial
/// event deadline after the first positive byte.
#[test]
fn client_subscription_idle_is_unbounded_but_partial_event_deadline_is_hard() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    fs::create_dir(runtime_path.join("realm")).unwrap();
    set_mode(&runtime_path.join("realm"), 0o700);
    let endpoint = test_runtime_dir(&runtime_path).unwrap().client_endpoint();
    let base = Instant::now();
    let operations = TestClientOperations::new(base);
    operations.push_connect(TestClientConnect::Success);
    operations.push_receive(TestClientReceive::BytesAfter(
        realm_core::ipc::encode(&matching_hello())
            .unwrap()
            .into_bytes(),
        Duration::ZERO,
    ));
    let client = endpoint
        .connect_with_test_operations("bar", operations.clone())
        .unwrap();
    operations.push_receive(TestClientReceive::BytesAfter(
        realm_core::ipc::encode(&Event::State(Box::new(protocol_state(1))))
            .unwrap()
            .into_bytes(),
        Duration::ZERO,
    ));
    let mut subscription = client.subscribe().unwrap();
    assert!(matches!(subscription.next(), Some(Ok(Event::State(_)))));

    operations.push_receive(TestClientReceive::ErrorAfter(Errno::AGAIN, Duration::ZERO));
    operations.push_poll(TestClientPoll::ReadyAfter(Duration::from_secs(60 * 60)));
    operations.push_receive(TestClientReceive::BytesAfter(b"{".to_vec(), Duration::ZERO));
    operations.push_receive(TestClientReceive::ErrorAfter(
        Errno::AGAIN,
        Duration::from_millis(1_500),
    ));
    operations.push_poll(TestClientPoll::ErrorAfter(
        Errno::INTR,
        Duration::from_millis(500),
    ));

    assert!(matches!(
        subscription.next(),
        Some(Err(ClientError::Timeout {
            phase: ClientPhase::SubscriptionEvent
        }))
    ));
    assert!(subscription.next().is_none());
    assert_eq!(
        operations.poll_timeouts(),
        vec![None, Some(Duration::from_millis(500))]
    );
}

fn active_control_listener() -> (tempfile::TempDir, PathBuf, ActiveControlListener) {
    let (temporary, runtime_path) = runtime_fixture();
    let listener = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap()
        .activate()
        .unwrap();
    (temporary, runtime_path, listener)
}

/// Catches accepting without atomic NONBLOCK/CLOEXEC or allocating identity
/// before successful real same-euid SO_PEERCRED admission.
#[test]
fn linux_admission_checks_real_credentials_before_receive() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path, listener) = active_control_listener();
    let mut server = listener.into_server(Instant::now());
    let listener_token = server.poll_interests().next().unwrap().token;
    let _client = UnixStream::connect(runtime_path.join("realm/ctl.sock")).unwrap();

    assert_eq!(
        server
            .service_one(
                Instant::now(),
                ReadyEvent {
                    token: listener_token,
                    readable: true,
                    writable: false,
                },
            )
            .unwrap(),
        None
    );

    let accepted = server
        .poll_interests()
        .find(|interest| interest.token != listener_token)
        .expect("same-euid peer is admitted");
    assert!(fcntl_getfl(accepted.fd).unwrap().contains(OFlags::NONBLOCK));
    assert!(rustix::io::fcntl_getfd(accepted.fd)
        .unwrap()
        .contains(FdFlags::CLOEXEC));
    assert_eq!(server.receive_calls_for_test(), 0);
}

/// Catches credential lookup failure, absence, or a foreign uid allocating a
/// connection record or reaching receive before rejection.
#[test]
fn rejected_credentials_close_before_receive_or_identity_allocation() {
    let _lock = process_test_lock();
    for outcome in [
        TestPeerCredential::Error(Errno::IO),
        TestPeerCredential::Missing,
        TestPeerCredential::Uid(geteuid().as_raw().wrapping_add(1)),
    ] {
        let (_temporary, runtime_path, listener) = active_control_listener();
        let mut server = listener.into_server_with_test_credentials(Instant::now(), [outcome]);
        let listener_token = server.poll_interests().next().unwrap().token;
        let mut client = UnixStream::connect(runtime_path.join("realm/ctl.sock")).unwrap();
        client.write_all(b"{\"cmd\":\"get-state\"}\n").unwrap();

        assert_eq!(
            server
                .service_one(
                    Instant::now(),
                    ReadyEvent {
                        token: listener_token,
                        readable: true,
                        writable: false,
                    },
                )
                .unwrap(),
            None
        );
        assert_eq!(server.connection_count_for_test(), 0);
        assert_eq!(server.receive_calls_for_test(), 0);
        client.set_nonblocking(true).unwrap();
        let mut byte = [0_u8; 1];
        match client.read(&mut byte) {
            Ok(0) => {}
            Err(error) if error.raw_os_error() == Some(Errno::CONNRESET.raw_os_error()) => {}
            other => panic!("rejected peer was not closed: {other:?}"),
        }
    }
}

fn server_fixture(now: Instant) -> (tempfile::TempDir, PathBuf, ControlServer, ControlToken) {
    let (temporary, runtime_path, listener) = active_control_listener();
    let server = listener.into_server(now);
    let listener_token = server.poll_interests().next().unwrap().token;
    (temporary, runtime_path, server, listener_token)
}

fn admit_test_peer(
    server: &mut ControlServer,
    listener_token: ControlToken,
    socket_path: &Path,
    now: Instant,
) -> (UnixStream, ConnectionId, ControlToken, i32) {
    let existing_tokens: Vec<_> = server
        .poll_interests()
        .filter_map(|interest| (interest.token != listener_token).then_some(interest.token))
        .collect();
    let client = UnixStream::connect(socket_path).unwrap();
    assert_eq!(
        server
            .service_one(
                now,
                ReadyEvent {
                    token: listener_token,
                    readable: true,
                    writable: false,
                },
            )
            .unwrap(),
        None
    );
    let interest = server
        .poll_interests()
        .find(|interest| {
            interest.token != listener_token && !existing_tokens.contains(&interest.token)
        })
        .expect("peer admitted");
    let token = interest.token;
    let raw_fd = interest.fd.as_raw_fd();
    let connection = server.connection_id_for_token_for_test(token).unwrap();
    (client, connection, token, raw_fd)
}

fn ready(token: ControlToken, readable: bool, writable: bool) -> ReadyEvent {
    ReadyEvent {
        token,
        readable,
        writable,
    }
}

fn assert_control_errno(error: ControlError, expected: Errno, listener: bool) {
    let source = match error {
        ControlError::ListenerIo(source) if listener => source,
        ControlError::ResourceExhausted(source) if !listener => source,
        other => panic!("unexpected control error: {other:?}"),
    };
    assert_eq!(source.raw_os_error(), Some(expected.raw_os_error()));
}

/// Catches retryable accept errors becoming fatal, resource exhaustion being
/// downgraded, or any listener readiness performing more than one accept.
#[test]
fn listener_errno_classification_and_one_accept_quantum_are_total() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, _runtime_path, mut server, listener_token) = server_fixture(base);

    for error in [Errno::INTR, Errno::AGAIN, Errno::CONNABORTED] {
        server.inject_accept_error_for_test(error);
        let before = server.accept_calls_for_test();
        assert_eq!(
            server
                .service_one(base, ready(listener_token, true, false))
                .unwrap(),
            None
        );
        assert_eq!(server.accept_calls_for_test(), before + 1);
    }
    for error in [Errno::MFILE, Errno::NFILE, Errno::NOBUFS, Errno::NOMEM] {
        server.inject_accept_error_for_test(error);
        let failure = server
            .service_one(base, ready(listener_token, true, false))
            .unwrap_err();
        assert_control_errno(failure, error, false);
    }
    server.inject_accept_error_for_test(Errno::IO);
    let failure = server
        .service_one(base, ready(listener_token, true, false))
        .unwrap_err();
    assert_control_errno(failure, Errno::IO, true);
}

/// Catches connected EINTR/EAGAIN closing a peer, reset/pipe escaping as
/// diagnostics, or other peer failures leaving the affected record alive.
#[test]
fn connected_errno_classification_is_peer_local_and_total() {
    let _lock = process_test_lock();
    let base = Instant::now();

    for error in [Errno::INTR, Errno::AGAIN] {
        let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
        let (_client, _connection, token, _fd) = admit_test_peer(
            &mut server,
            listener_token,
            &runtime_path.join("realm/ctl.sock"),
            base,
        );
        server.inject_receive_for_test(TestReceive::Error(error));
        assert_eq!(
            server.service_one(base, ready(token, true, false)).unwrap(),
            None
        );
        assert_eq!(server.connection_count_for_test(), 1);
    }

    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, _connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    server.inject_receive_for_test(TestReceive::Error(Errno::CONNRESET));
    assert_eq!(
        server.service_one(base, ready(token, true, false)).unwrap(),
        None
    );
    assert_eq!(server.connection_count_for_test(), 0);

    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    server.inject_receive_for_test(TestReceive::Error(Errno::IO));
    match server
        .service_one(base, ready(token, true, false))
        .unwrap_err()
    {
        ControlError::PeerIo {
            connection: got,
            source,
        } => {
            assert_eq!(got, connection);
            assert_eq!(source.raw_os_error(), Some(Errno::IO.raw_os_error()));
        }
        other => panic!("unexpected receive error: {other:?}"),
    }
    assert_eq!(server.connection_count_for_test(), 0);

    for error in [Errno::CONNRESET, Errno::PIPE] {
        let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
        let (_client, _connection, token, _fd) = admit_test_peer(
            &mut server,
            listener_token,
            &runtime_path.join("realm/ctl.sock"),
            base,
        );
        server.inject_receive_for_test(TestReceive::Bytes(
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
        ));
        assert_eq!(
            server.service_one(base, ready(token, true, false)).unwrap(),
            None
        );
        server.inject_send_for_test(TestSend::Error(error));
        assert_eq!(
            server.service_one(base, ready(token, false, true)).unwrap(),
            None
        );
        assert_eq!(server.connection_count_for_test(), 0);
    }

    for error in [Errno::INTR, Errno::AGAIN] {
        let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
        let (_client, connection, token, _fd) = admit_test_peer(
            &mut server,
            listener_token,
            &runtime_path.join("realm/ctl.sock"),
            base,
        );
        server.inject_receive_for_test(TestReceive::Bytes(
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
        ));
        server.service_one(base, ready(token, true, false)).unwrap();
        server.inject_send_for_test(TestSend::Error(error));
        assert_eq!(
            server.service_one(base, ready(token, false, true)).unwrap(),
            None
        );
        assert!(server.has_connection_for_test(connection));
    }

    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    server.inject_receive_for_test(TestReceive::Bytes(
        b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
    ));
    server.service_one(base, ready(token, true, false)).unwrap();
    server.inject_send_for_test(TestSend::Error(Errno::IO));
    match server
        .service_one(base, ready(token, false, true))
        .unwrap_err()
    {
        ControlError::PeerIo {
            connection: got,
            source,
        } => {
            assert_eq!(got, connection);
            assert_eq!(source.raw_os_error(), Some(Errno::IO.raw_os_error()));
        }
        other => panic!("unexpected send error: {other:?}"),
    }
    assert!(!server.has_connection_for_test(connection));
}

/// Catches output winning over subscriber input, enabled-interest drift, and
/// multiple connected I/O calls from one copied readiness event.
#[test]
fn poll_interest_and_ready_arbitration_is_total() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );

    server.inject_receive_for_test(TestReceive::Bytes(
        b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
    ));
    assert_eq!(
        server.service_one(base, ready(token, true, false)).unwrap(),
        None
    );
    let hello_interest = server.interest_for_test(token).unwrap();
    assert_eq!(hello_interest, (false, true));
    let hello_len = server.output_len_for_test(connection).unwrap();
    server.inject_send_for_test(TestSend::Count(hello_len));
    assert_eq!(
        server.service_one(base, ready(token, false, true)).unwrap(),
        None
    );

    server.inject_receive_for_test(TestReceive::Bytes(b"{\"cmd\":\"subscribe\"}\n".to_vec()));
    assert_eq!(
        server.service_one(base, ready(token, true, false)).unwrap(),
        Some(ControlAction::Request {
            connection,
            request: Request::Subscribe,
        })
    );
    assert_eq!(server.interest_for_test(token).unwrap(), (false, false));
    server
        .complete_subscribe(base, connection, protocol_state(1))
        .unwrap();
    assert_eq!(server.interest_for_test(token).unwrap(), (true, true));

    let sends_before = server.send_calls_for_test();
    let receives_before = server.receive_calls_for_test();
    server.inject_receive_for_test(TestReceive::Bytes(vec![b'x']));
    assert_eq!(
        server.service_one(base, ready(token, true, true)).unwrap(),
        None
    );
    assert_eq!(server.receive_calls_for_test(), receives_before + 1);
    assert_eq!(server.send_calls_for_test(), sends_before);
    assert_eq!(server.connection_count_for_test(), 0);
}

/// Catches exact-deadline expiry occurring after a socket syscall.
#[test]
fn service_quantum_expires_before_socket_io() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, _connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    let calls = server.socket_calls_for_test();
    assert_eq!(
        server
            .service_one(base + Duration::from_secs(1), ready(token, true, true))
            .unwrap(),
        None
    );
    assert_eq!(server.socket_calls_for_test(), calls);
    assert_eq!(server.connection_count_for_test(), 0);
}

/// Catches resolving even a stale token before the mandatory global expiry
/// pass has closed every peer at its exact deadline.
#[test]
fn stale_readiness_runs_global_expiry_before_token_resolution() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let socket_path = runtime_path.join("realm/ctl.sock");
    let (stale_client, _stale_connection, stale_token, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    let (_due_client, _due_connection, _due_token, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    server.inject_receive_for_test(TestReceive::Error(Errno::CONNRESET));
    server
        .service_one(base, ready(stale_token, true, false))
        .unwrap();
    drop(stale_client);

    let socket_calls = server.socket_calls_for_test();
    assert_eq!(
        server
            .service_one(
                base + Duration::from_secs(1),
                ready(stale_token, true, true),
            )
            .unwrap(),
        None
    );
    assert_eq!(server.connection_count_for_test(), 0);
    assert_eq!(server.socket_calls_for_test(), socket_calls);
    assert_eq!(
        server.connections_at_last_token_resolution_for_test(),
        Some(0)
    );
}

/// Catches treating raw fd numbers as identity when the kernel reuses one.
#[test]
fn stale_token_and_connection_id_cannot_target_reused_fd() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let socket_path = runtime_path.join("realm/ctl.sock");
    let (old_client, old_connection, old_token, old_fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    server.inject_receive_for_test(TestReceive::Error(Errno::CONNRESET));
    server
        .service_one(base, ready(old_token, true, false))
        .unwrap();
    drop(old_client);

    let (_new_client, new_connection, new_token, new_fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    assert_eq!(new_fd, old_fd, "fixture requires actual numeric fd reuse");
    assert_ne!(new_connection, old_connection);
    assert_ne!(new_token, old_token);

    let calls = server.socket_calls_for_test();
    assert_eq!(
        server
            .service_one(base, ready(old_token, true, true))
            .unwrap(),
        None
    );
    assert_eq!(server.socket_calls_for_test(), calls);
    assert!(matches!(
        server.complete_request(base, old_connection, Response::Ok),
        Err(ControlError::StaleConnection { connection }) if connection == old_connection
    ));
    assert_eq!(server.connection_count_for_test(), 1);
}

fn finish_test_handshake(
    server: &mut ControlServer,
    connection: ConnectionId,
    token: ControlToken,
    now: Instant,
) {
    server.inject_receive_for_test(TestReceive::Bytes(
        b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
    ));
    assert_eq!(
        server.service_one(now, ready(token, true, false)).unwrap(),
        None
    );
    let output = server
        .output_len_for_test(connection)
        .unwrap_or_else(|| panic!("handshake did not queue output for {connection:?}"));
    server.inject_send_for_test(TestSend::Count(output));
    assert_eq!(
        server.service_one(now, ready(token, false, true)).unwrap(),
        None
    );
}

fn make_test_subscriber(
    server: &mut ControlServer,
    connection: ConnectionId,
    token: ControlToken,
    now: Instant,
    revision: u64,
) {
    finish_test_handshake(server, connection, token, now);
    server.inject_receive_for_test(TestReceive::Bytes(b"{\"cmd\":\"subscribe\"}\n".to_vec()));
    assert_eq!(
        server.service_one(now, ready(token, true, false)).unwrap(),
        Some(ControlAction::Request {
            connection,
            request: Request::Subscribe,
        })
    );
    server
        .complete_subscribe(now, connection, protocol_state(revision))
        .unwrap();
}

/// Catches oversized per-peer completions mutating a queue, remaining live,
/// or reporting an unbounded/unstable affected set.
#[test]
fn outbound_frame_bound_is_streaming_peer_local_and_stable() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    finish_test_handshake(&mut server, connection, token, base);
    server.inject_receive_for_test(TestReceive::Bytes(b"{\"cmd\":\"get-state\"}\n".to_vec()));
    assert!(matches!(
        server.service_one(base, ready(token, true, false)).unwrap(),
        Some(ControlAction::Request { connection: got, request: Request::GetState }) if got == connection
    ));
    let oversized = Response::Error {
        message: "x".repeat(MAX_FRAME_BYTES),
    };
    assert!(matches!(
        server.complete_request(base, connection, oversized),
        Err(ControlError::OutboundFrameTooLarge { connections }) if connections == vec![connection]
    ));
    assert_eq!(server.connection_count_for_test(), 0);

    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    finish_test_handshake(&mut server, connection, token, base);
    server.inject_receive_for_test(TestReceive::Bytes(b"{\"cmd\":\"subscribe\"}\n".to_vec()));
    assert!(matches!(
        server.service_one(base, ready(token, true, false)).unwrap(),
        Some(ControlAction::Request { connection: got, request: Request::Subscribe }) if got == connection
    ));
    let mut oversized_state = protocol_state(1);
    oversized_state.focused_title = "x".repeat(MAX_FRAME_BYTES);
    assert!(matches!(
        server.complete_subscribe(base, connection, oversized_state),
        Err(ControlError::OutboundFrameTooLarge { connections }) if connections == vec![connection]
    ));
    assert_eq!(server.connection_count_for_test(), 0);
}

/// Catches clearing pending completion state before bounded encoding succeeds,
/// and catches retaining the owned subscribe state through a full clone.
#[test]
fn completion_encoding_precedes_queue_mutation_and_overflow_is_retriable() {
    let base = Instant::now();
    let (mut ordinary, action) = pending_protocol_machine(base, b"{\"cmd\":\"get-state\"}\n");
    assert_eq!(action, MachineAction::Request(Request::GetState));
    let oversized_response = Response::Error {
        message: "x".repeat(MAX_FRAME_BYTES),
    };
    assert_eq!(
        ordinary.complete_request(base, &oversized_response),
        MachineAction::OutboundFrameTooLarge
    );
    assert_eq!(ordinary.phase(), ConnectionPhase::Ready);
    assert!(!ordinary.input_enabled());
    assert!(ordinary.output().is_none());
    assert_eq!(
        ordinary.complete_request(base, &Response::Ok),
        MachineAction::None
    );
    assert!(ordinary.output().is_some());

    let (mut subscriber, action) = pending_protocol_machine(base, b"{\"cmd\":\"subscribe\"}\n");
    assert_eq!(action, MachineAction::Request(Request::Subscribe));
    let mut oversized_state = protocol_state(1);
    oversized_state.focused_title = "x".repeat(MAX_FRAME_BYTES);
    assert_eq!(
        subscriber.complete_subscribe(base, oversized_state),
        MachineAction::OutboundFrameTooLarge
    );
    assert_eq!(subscriber.phase(), ConnectionPhase::Ready);
    assert!(!subscriber.input_enabled());
    assert!(subscriber.output().is_none());
    assert_eq!(
        subscriber.complete_subscribe(base, protocol_state(2)),
        MachineAction::None
    );
    assert_eq!(subscriber.phase(), ConnectionPhase::Subscriber);
    assert!(subscriber.output().is_some());
}

/// Catches publication encoding per subscriber, partial queue mutation before
/// overflow is known, or closing non-subscribers on event overflow.
#[test]
fn oversized_publish_closes_sorted_subscriber_set_after_one_encode() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let socket_path = runtime_path.join("realm/ctl.sock");
    let (_idle_client, idle_id, _idle_token, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    let (_client_a, id_a, token_a, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    let (_client_b, id_b, token_b, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    make_test_subscriber(&mut server, id_a, token_a, base, 1);
    make_test_subscriber(&mut server, id_b, token_b, base, 1);

    let mut oversized = protocol_state(2);
    oversized.focused_title = "x".repeat(MAX_FRAME_BYTES);
    assert!(matches!(
        server.publish_state(base, oversized),
        Err(ControlError::OutboundFrameTooLarge { connections })
            if connections == vec![id_a, id_b]
    ));
    assert_eq!(server.connection_count_for_test(), 1);
    assert!(server.has_connection_for_test(idle_id));

    let mut still_oversized = protocol_state(3);
    still_oversized.focused_title = "x".repeat(MAX_FRAME_BYTES);
    server.publish_state(base, still_oversized).unwrap();
    assert_eq!(server.connection_count_for_test(), 1);
}

/// Catches a partial shutdown transition, a moving hard deadline, post-start
/// actions/completions, or completion that is not observable at drain/expiry.
#[test]
fn shutdown_transition_is_total_idempotent_and_completion_observable() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let socket_path = runtime_path.join("realm/ctl.sock");
    let (_ordinary_client, ordinary_id, ordinary_token, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    let (_subscriber_client, subscriber_id, subscriber_token, _fd) =
        admit_test_peer(&mut server, listener_token, &socket_path, base);
    make_test_subscriber(&mut server, subscriber_id, subscriber_token, base, 1);
    assert_eq!(
        server.interest_for_test(ordinary_token),
        Some((true, false))
    );
    assert_eq!(
        server.interest_for_test(subscriber_token),
        Some((true, true))
    );

    server.begin_shutdown(base);
    assert!(!server.has_connection_for_test(ordinary_id));
    assert!(server.has_connection_for_test(subscriber_id));
    assert!(!server
        .poll_interests()
        .any(|interest| interest.token == listener_token));
    assert_eq!(
        server.next_deadline(),
        Some(base + Duration::from_millis(100))
    );
    assert!(!server.is_shutdown_complete());

    server.begin_shutdown(base + Duration::from_secs(1));
    assert_eq!(
        server.next_deadline(),
        Some(base + Duration::from_millis(100))
    );
    let calls = server.socket_calls_for_test();
    assert_eq!(
        server
            .service_one(base, ready(ordinary_token, true, true))
            .unwrap(),
        None
    );
    assert_eq!(server.socket_calls_for_test(), calls);
    assert!(matches!(
        server.complete_request(base, ordinary_id, Response::Ok),
        Err(ControlError::ShuttingDown)
    ));
    assert!(matches!(
        server.publish_state(base, protocol_state(2)),
        Err(ControlError::ShuttingDown)
    ));
    assert!(matches!(
        server.complete_subscribe(base, ordinary_id, protocol_state(2)),
        Err(ControlError::ShuttingDown)
    ));

    let initial = server.output_len_for_test(subscriber_id).unwrap();
    server.inject_send_for_test(TestSend::Count(initial));
    assert_eq!(
        server
            .service_one(base, ready(subscriber_token, false, true))
            .unwrap(),
        None
    );
    assert!(!server.is_shutdown_complete());
    let shutdown = server.output_len_for_test(subscriber_id).unwrap();
    server.inject_send_for_test(TestSend::Count(shutdown));
    assert_eq!(
        server
            .service_one(base, ready(subscriber_token, false, true))
            .unwrap(),
        None
    );
    assert!(server.is_shutdown_complete());
    assert_eq!(server.next_deadline(), None);

    let (_temporary, runtime_path, mut expiring, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut expiring,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    make_test_subscriber(&mut expiring, connection, token, base, 1);
    expiring.begin_shutdown(base);
    let calls = expiring.socket_calls_for_test();
    expiring.expire(base + Duration::from_millis(100)).unwrap();
    assert_eq!(expiring.socket_calls_for_test(), calls);
    assert!(expiring.is_shutdown_complete());

    let (_temporary, _runtime_path, mut empty, _listener_token) = server_fixture(base);
    empty.begin_shutdown(base);
    assert!(empty.is_shutdown_complete());
    assert_eq!(empty.next_deadline(), None);
}

/// Catches refusing capacity without the required one accept/close quantum or
/// allocating a 65th record and identity.
#[test]
fn capacity_refusal_is_one_fd_per_quantum_and_allocates_nothing() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, listener) = active_control_listener();
    let credentials = std::iter::repeat_n(TestPeerCredential::Uid(geteuid().as_raw()), 65);
    let mut server = listener.into_server_with_test_credentials(base, credentials);
    let listener_token = server.poll_interests().next().unwrap().token;
    let socket_path = runtime_path.join("realm/ctl.sock");
    let mut clients = Vec::new();
    for _ in 0..64 {
        let (client, _id, _token, _fd) =
            admit_test_peer(&mut server, listener_token, &socket_path, base);
        clients.push(client);
    }
    let ids_before = server.connection_ids_for_test();
    let next_identities_before = server.next_identities_for_test();
    let accepts_before = server.accept_calls_for_test();
    let mut excess = UnixStream::connect(&socket_path).unwrap();
    assert_eq!(
        server
            .service_one(base, ready(listener_token, true, false))
            .unwrap(),
        None
    );
    assert_eq!(server.accept_calls_for_test(), accepts_before + 1);
    assert_eq!(server.connection_count_for_test(), 64);
    assert_eq!(server.connection_ids_for_test(), ids_before);
    assert_eq!(server.next_identities_for_test(), next_identities_before);
    assert_eq!(server.credential_outcomes_for_test(), 0);
    excess.set_nonblocking(true).unwrap();
    let mut byte = [0_u8; 1];
    match excess.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.raw_os_error() == Some(Errno::CONNRESET.raw_os_error()) => {}
        other => panic!("excess peer was not closed: {other:?}"),
    }
    drop(clients);
}

/// Catches wrapping, panicking, reusing, or colliding stable identities after
/// the final monotonic ConnectionId/ControlToken pair has been allocated.
#[test]
fn exhausted_monotonic_identities_refuse_without_wrap_or_collision() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let socket_path = runtime_path.join("realm/ctl.sock");
    server.set_next_identities_for_test(u64::MAX, u64::MAX);

    let _last_client = UnixStream::connect(&socket_path).unwrap();
    server
        .service_one(base, ready(listener_token, true, false))
        .unwrap();
    assert_eq!(server.connection_count_for_test(), 1);
    let identities = server.connection_ids_for_test();

    let mut refused = UnixStream::connect(&socket_path).unwrap();
    let accepts = server.accept_calls_for_test();
    server
        .service_one(base, ready(listener_token, true, false))
        .unwrap();
    assert_eq!(server.accept_calls_for_test(), accepts + 1);
    assert_eq!(server.connection_count_for_test(), 1);
    assert_eq!(server.connection_ids_for_test(), identities);
    refused.set_nonblocking(true).unwrap();
    let mut byte = [0_u8; 1];
    match refused.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.raw_os_error() == Some(Errno::CONNRESET.raw_os_error()) => {}
        other => panic!("identity-exhausted peer was not closed: {other:?}"),
    }
}

/// Catches any ready-token branch growing into a drain loop or performing both
/// receive and send in one service quantum.
#[test]
fn one_ready_token_performs_at_most_one_socket_io() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);

    let before = server.socket_calls_for_test();
    assert_eq!(
        server
            .service_one(base, ready(listener_token, false, true))
            .unwrap(),
        None
    );
    assert_eq!(server.socket_calls_for_test() - before, 0);

    let client = UnixStream::connect(runtime_path.join("realm/ctl.sock")).unwrap();
    let before = server.socket_calls_for_test();
    server
        .service_one(base, ready(listener_token, true, false))
        .unwrap();
    assert_eq!(server.socket_calls_for_test() - before, 1);
    let peer = server
        .poll_interests()
        .find(|interest| interest.token != listener_token)
        .unwrap();
    let token = peer.token;

    server.inject_receive_for_test(TestReceive::Bytes(
        b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
    ));
    let before = server.socket_calls_for_test();
    server.service_one(base, ready(token, true, true)).unwrap();
    assert_eq!(server.socket_calls_for_test() - before, 1);

    server.inject_send_for_test(TestSend::Error(Errno::CONNRESET));
    let before = server.socket_calls_for_test();
    server.service_one(base, ready(token, true, true)).unwrap();
    assert_eq!(server.socket_calls_for_test() - before, 1);

    let before = server.socket_calls_for_test();
    server.service_one(base, ready(token, true, true)).unwrap();
    assert_eq!(server.socket_calls_for_test() - before, 0);
    drop(client);
}

/// Catches the production send-call boundary omitting MSG_NOSIGNAL even when
/// its deterministic test syscall result is injected.
#[test]
fn production_send_path_supplies_msg_nosignal() {
    let _lock = process_test_lock();
    let base = Instant::now();
    let (_temporary, runtime_path, mut server, listener_token) = server_fixture(base);
    let (_client, connection, token, _fd) = admit_test_peer(
        &mut server,
        listener_token,
        &runtime_path.join("realm/ctl.sock"),
        base,
    );
    server.inject_receive_for_test(TestReceive::Bytes(
        b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n".to_vec(),
    ));
    server.service_one(base, ready(token, true, false)).unwrap();
    let output = server.output_len_for_test(connection).unwrap();
    server.inject_send_for_test(TestSend::Count(output));
    server.service_one(base, ready(token, false, true)).unwrap();

    assert_eq!(server.last_send_flags_for_test(), Some(SendFlags::NOSIGNAL));
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
    WrongAddress,
    Error(Errno),
    ReplaceWithFileThenError(PathBuf, Errno),
}

enum InjectedAcceptconn {
    Actual,
    True,
    Error(Errno),
}

struct InjectedBindOperations {
    bind: InjectedBind,
    stat: InjectedStat,
    getsockname: InjectedVerification,
    acceptconn: InjectedAcceptconn,
}

impl InjectedBindOperations {
    fn actual() -> Self {
        Self {
            bind: InjectedBind::Actual,
            stat: InjectedStat::Actual,
            getsockname: InjectedVerification::Actual,
            acceptconn: InjectedAcceptconn::Actual,
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
            InjectedVerification::WrongAddress => {
                SocketAddrUnix::new(Path::new("/tmp/realm-control-wrong-address"))
            }
            InjectedVerification::Error(error) => Err(*error),
            InjectedVerification::ReplaceWithFileThenError(path, error) => {
                replace_with_file(path);
                Err(*error)
            }
        }
    }

    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool> {
        match self.acceptconn {
            InjectedAcceptconn::Actual => socket_acceptconn(socket),
            InjectedAcceptconn::True => Ok(true),
            InjectedAcceptconn::Error(error) => Err(error),
        }
    }
}

enum InjectedActivation {
    Actual,
    ListenError(Errno),
    AcceptconnError(Errno),
    AcceptconnFalse,
    ReplaceWithFileThenAcceptconnError(PathBuf, Errno),
}

struct InjectedActivationOperations {
    outcome: InjectedActivation,
    listen_calls: Cell<u32>,
    backlog: Cell<Option<i32>>,
}

impl InjectedActivationOperations {
    fn actual() -> Self {
        Self {
            outcome: InjectedActivation::Actual,
            listen_calls: Cell::new(0),
            backlog: Cell::new(None),
        }
    }

    fn with_outcome(outcome: InjectedActivation) -> Self {
        Self {
            outcome,
            ..Self::actual()
        }
    }
}

impl crate::endpoint::ActivationOperations for InjectedActivationOperations {
    fn listen(&self, socket: BorrowedFd<'_>, backlog: i32) -> rustix::io::Result<()> {
        self.listen_calls.set(self.listen_calls.get() + 1);
        self.backlog.set(Some(backlog));
        match self.outcome {
            InjectedActivation::ListenError(error) => Err(error),
            InjectedActivation::Actual
            | InjectedActivation::AcceptconnError(_)
            | InjectedActivation::AcceptconnFalse
            | InjectedActivation::ReplaceWithFileThenAcceptconnError(_, _) => {
                listen_socket(socket, backlog)
            }
        }
    }

    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool> {
        match &self.outcome {
            InjectedActivation::Actual => socket_acceptconn(socket),
            InjectedActivation::ListenError(_) => {
                panic!("activation queried SO_ACCEPTCONN after listen failed")
            }
            InjectedActivation::AcceptconnError(error) => Err(*error),
            InjectedActivation::AcceptconnFalse => Ok(false),
            InjectedActivation::ReplaceWithFileThenAcceptconnError(path, error) => {
                replace_with_file(path);
                Err(*error)
            }
        }
    }
}

fn bind_error(result: Result<BoundControlEndpoint, IpcPathError>) -> IpcPathError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected bind to fail"),
    }
}

fn activation_error(result: Result<ActiveControlListener, IpcPathError>) -> IpcPathError {
    match result {
        Err(error) => error,
        Ok(_) => panic!("expected activation to fail"),
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
    let realm_fd = bound.realm_dir().as_fd().as_raw_fd();
    let expected_address =
        SocketAddrUnix::new(PathBuf::from(format!("/proc/self/fd/{realm_fd}/ctl.sock"))).unwrap();
    let display_address = SocketAddrUnix::new(&socket_path).unwrap();
    let actual_address: SocketAddrUnix = getsockname(socket).unwrap().try_into().unwrap();
    assert_eq!(actual_address, expected_address);
    assert_ne!(actual_address, display_address);
    assert!(!socket_acceptconn(socket).unwrap());
}

/// Catches a regression where activation borrows the bound capability instead
/// of consuming it, which would make a second activation type-check.
#[test]
fn activation_signature_consumes_bound_capability() {
    let _lock = process_test_lock();
    let _: fn(BoundControlEndpoint) -> Result<ActiveControlListener, IpcPathError> =
        BoundControlEndpoint::activate;
}

/// Catches a regression where activation either listens more than once, uses a
/// different backlog, or returns a listener that is not actually accepting.
#[test]
fn activation_is_consuming_one_shot_and_listens_with_backlog_64() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedActivationOperations::actual();

    let active = bound.activate_with(&operations).unwrap();

    assert_eq!(operations.listen_calls.get(), 1);
    assert_eq!(operations.backlog.get(), Some(64));
    assert!(socket_acceptconn(active.socket_fd()).unwrap());
    UnixStream::connect(&socket_path).unwrap();
    assert_eq!(active.endpoint().path(), socket_path.as_path());
    assert_eq!(
        active.realm_dir().as_fd().as_raw_fd(),
        active.endpoint().realm_dir().as_fd().as_raw_fd()
    );
}

/// Catches a regression where an activation failure leaks an unchanged owned
/// socket or removes a pathname replacement observed by cleanup.
#[test]
fn activation_failure_uses_identity_checked_cleanup() {
    let _lock = process_test_lock();

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations =
        InjectedActivationOperations::with_outcome(InjectedActivation::ListenError(Errno::IO));
    assert_io_errno(
        activation_error(bound.activate_with(&operations)),
        Errno::IO,
    );
    assert_eq!(operations.listen_calls.get(), 1);
    assert_eq!(operations.backlog.get(), Some(64));
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations =
        InjectedActivationOperations::with_outcome(InjectedActivation::AcceptconnFalse);
    assert!(matches!(
        activation_error(bound.activate_with(&operations)),
        IpcPathError::UnsafeSocketEntry
    ));
    assert_eq!(operations.listen_calls.get(), 1);
    assert_eq!(operations.backlog.get(), Some(64));
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations =
        InjectedActivationOperations::with_outcome(InjectedActivation::AcceptconnError(Errno::IO));
    assert_io_errno(
        activation_error(bound.activate_with(&operations)),
        Errno::IO,
    );
    assert_eq!(operations.listen_calls.get(), 1);
    assert_eq!(operations.backlog.get(), Some(64));
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let bound = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedActivationOperations::with_outcome(
        InjectedActivation::ReplaceWithFileThenAcceptconnError(socket_path.clone(), Errno::IO),
    );
    assert_io_errno(
        activation_error(bound.activate_with(&operations)),
        Errno::IO,
    );
    assert_eq!(fs::read(&socket_path).unwrap(), b"replacement must survive");
}

/// Catches a regression where the active wrapper's drop cleanup deletes a
/// socket replacement that differs from the retained pathname identity.
#[test]
fn active_drop_preserves_detected_replacement() {
    let _lock = process_test_lock();
    let (_temporary, runtime_path) = runtime_fixture();
    let active = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap()
        .bind()
        .unwrap()
        .activate()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let replacement_path = runtime_path.join("realm/replacement.sock");
    let _replacement = bind_control_listener(&replacement_path, 0o600);
    fs::remove_file(&socket_path).unwrap();
    fs::rename(&replacement_path, &socket_path).unwrap();
    let replacement_identity = path_identity(&socket_path);

    drop(active);

    assert_eq!(path_identity(&socket_path), replacement_identity);
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
        acceptconn: InjectedAcceptconn::Error(Errno::IO),
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
        getsockname: InjectedVerification::WrongAddress,
        ..InjectedBindOperations::actual()
    };
    assert!(matches!(
        bind_error(endpoint.bind_with(&operations)),
        IpcPathError::UnsafeSocketEntry
    ));
    assert!(!socket_path.exists());

    let (_temporary, runtime_path) = runtime_fixture();
    let endpoint = test_runtime_dir(&runtime_path)
        .unwrap()
        .prepare_server_endpoint()
        .unwrap();
    let socket_path = runtime_path.join("realm/ctl.sock");
    let operations = InjectedBindOperations {
        acceptconn: InjectedAcceptconn::True,
        ..InjectedBindOperations::actual()
    };
    assert!(matches!(
        bind_error(endpoint.bind_with(&operations)),
        IpcPathError::UnsafeSocketEntry
    ));
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

#[test]
fn protocol_state_machine_is_total() {
    struct Case {
        name: &'static str,
        prepare_ready: bool,
        input: Vec<u8>,
        expected_action: MachineAction,
        expected_output: Option<&'static [u8]>,
        expected_phase: ConnectionPhase,
        expected_input_enabled: bool,
    }

    let hello = b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n";
    let hello_reply =
        b"{\"reply\":\"hello\",\"data\":{\"version\":1,\"session\":\"test-session\"}}\n";
    let expected_hello_error = b"{\"reply\":\"error\",\"data\":{\"message\":\"expected Hello\"}}\n";
    let invalid_request_error =
        b"{\"reply\":\"error\",\"data\":{\"message\":\"invalid request\"}}\n";
    let duplicate_hello_error =
        b"{\"reply\":\"error\",\"data\":{\"message\":\"duplicate Hello\"}}\n";

    let cases = vec![
        Case {
            name: "matching Hello waits for its reply",
            prepare_ready: false,
            input: hello.to_vec(),
            expected_action: MachineAction::None,
            expected_output: Some(hello_reply),
            expected_phase: ConnectionPhase::SendingHello,
            expected_input_enabled: false,
        },
        Case {
            name: "mismatched Hello discards its pipeline",
            prepare_ready: false,
            input: b"{\"cmd\":\"hello\",\"arg\":{\"version\":2,\"client\":\"test\"}}\n{\"cmd\":\"get-state\"}\n".to_vec(),
            expected_action: MachineAction::None,
            expected_output: Some(hello_reply),
            expected_phase: ConnectionPhase::CloseAfterReply,
            expected_input_enabled: false,
        },
        Case {
            name: "ordinary request before Hello is terminal",
            prepare_ready: false,
            input: b"{\"cmd\":\"get-state\"}\n".to_vec(),
            expected_action: MachineAction::None,
            expected_output: Some(expected_hello_error),
            expected_phase: ConnectionPhase::CloseAfterReply,
            expected_input_enabled: false,
        },
        Case {
            name: "invalid UTF-8 closes silently",
            prepare_ready: false,
            input: vec![0xff, b'\n'],
            expected_action: MachineAction::Close(MachineClose::InvalidInput),
            expected_output: None,
            expected_phase: ConnectionPhase::Closing,
            expected_input_enabled: false,
        },
        Case {
            name: "invalid JSON closes silently",
            prepare_ready: false,
            input: b"{]\n".to_vec(),
            expected_action: MachineAction::Close(MachineClose::InvalidInput),
            expected_output: None,
            expected_phase: ConnectionPhase::Closing,
            expected_input_enabled: false,
        },
        Case {
            name: "valid JSON outside Request gets one error",
            prepare_ready: false,
            input: b"{\"cmd\":\"detonate\"}\n".to_vec(),
            expected_action: MachineAction::None,
            expected_output: Some(invalid_request_error),
            expected_phase: ConnectionPhase::CloseAfterReply,
            expected_input_enabled: false,
        },
        Case {
            name: "duplicate Hello is terminal",
            prepare_ready: true,
            input: hello.to_vec(),
            expected_action: MachineAction::None,
            expected_output: Some(duplicate_hello_error),
            expected_phase: ConnectionPhase::CloseAfterReply,
            expected_input_enabled: false,
        },
        Case {
            name: "Subscribe is emitted unchanged",
            prepare_ready: true,
            input: b"{\"cmd\":\"subscribe\"}\n".to_vec(),
            expected_action: MachineAction::Request(Request::Subscribe),
            expected_output: None,
            expected_phase: ConnectionPhase::Ready,
            expected_input_enabled: false,
        },
        Case {
            name: "second ordinary pipeline closes before dispatch",
            prepare_ready: true,
            input: b"{\"cmd\":\"get-state\"}\n{\"cmd\":\"quit\"}\n".to_vec(),
            expected_action: MachineAction::Close(MachineClose::ExcessPipeline),
            expected_output: None,
            expected_phase: ConnectionPhase::Closing,
            expected_input_enabled: false,
        },
        Case {
            name: "third startup frame closes before retained dispatch",
            prepare_ready: false,
            input: b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n{\"cmd\":\"get-state\"}\n{\"cmd\":\"quit\"}\n".to_vec(),
            expected_action: MachineAction::Close(MachineClose::ExcessPipeline),
            expected_output: None,
            expected_phase: ConnectionPhase::Closing,
            expected_input_enabled: false,
        },
        Case {
            name: "exactly full unterminated prefix is impossible",
            prepare_ready: false,
            input: vec![b' '; MAX_FRAME_BYTES],
            expected_action: MachineAction::Close(MachineClose::FrameTooLarge),
            expected_output: None,
            expected_phase: ConnectionPhase::Closing,
            expected_input_enabled: false,
        },
    ];

    for case in cases {
        let now = Instant::now();
        let mut machine = ConnectionMachine::new(now, "test-session");
        if case.prepare_ready {
            assert_eq!(machine.ingest(now, hello), MachineAction::None);
            assert_eq!(machine.output(), Some(hello_reply.as_slice()));
            assert_eq!(
                machine.advance_output(now, hello_reply.len()),
                MachineAction::None
            );
            assert_eq!(machine.phase(), ConnectionPhase::Ready);
        }

        assert_eq!(
            machine.ingest(now, &case.input),
            case.expected_action,
            "{} action",
            case.name
        );
        assert_eq!(
            machine.output(),
            case.expected_output,
            "{} output",
            case.name
        );
        assert_eq!(machine.phase(), case.expected_phase, "{} phase", case.name);
        assert_eq!(
            machine.input_enabled(),
            case.expected_input_enabled,
            "{} input interest",
            case.name
        );
    }
}

#[test]
fn hello_reply_drains_before_retained_request_dispatch() {
    let now = Instant::now();
    let hello_reply =
        b"{\"reply\":\"hello\",\"data\":{\"version\":1,\"session\":\"test-session\"}}\n";
    let mut machine = ConnectionMachine::new(now, "test-session");

    assert_eq!(
        machine.ingest(
            now,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"shell\"}}\n{\"cmd\":\"get-state\"}\n"
        ),
        MachineAction::None
    );
    assert_eq!(machine.phase(), ConnectionPhase::SendingHello);
    assert!(!machine.input_enabled());
    assert_eq!(machine.output(), Some(hello_reply.as_slice()));

    assert_eq!(
        machine.advance_output(now, hello_reply.len() - 1),
        MachineAction::None
    );
    assert_eq!(machine.phase(), ConnectionPhase::SendingHello);
    assert_eq!(
        machine.output(),
        Some(&hello_reply[hello_reply.len() - 1..])
    );

    assert_eq!(
        machine.advance_output(now, 1),
        MachineAction::Request(Request::GetState)
    );
    assert_eq!(machine.phase(), ConnectionPhase::Ready);
    assert_eq!(machine.output(), None);
    assert!(!machine.input_enabled());
}

#[test]
fn mismatched_hello_reply_drains_then_closes() {
    let now = Instant::now();
    let mut machine = ConnectionMachine::new(now, "test-session");
    assert_eq!(
        machine.ingest(
            now,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":2,\"client\":\"test\"}}\n{\"cmd\":\"quit\"}\n"
        ),
        MachineAction::None
    );
    let reply_len = machine.output().unwrap().len();
    assert_eq!(
        machine.advance_output(now, reply_len),
        MachineAction::Close(MachineClose::OutputComplete)
    );
    assert_eq!(machine.phase(), ConnectionPhase::Closing);
    assert!(!machine.input_enabled());
}

fn ready_protocol_machine(now: Instant) -> ConnectionMachine {
    let mut machine = ConnectionMachine::new(now, "test-session");
    assert_eq!(
        machine.ingest(
            now,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n"
        ),
        MachineAction::None
    );
    let hello_len = machine.output().unwrap().len();
    assert_eq!(machine.advance_output(now, hello_len), MachineAction::None);
    machine
}

fn pending_protocol_machine(
    now: Instant,
    request_frame: &[u8],
) -> (ConnectionMachine, MachineAction) {
    let mut machine = ready_protocol_machine(now);
    let action = machine.ingest(now, request_frame);
    (machine, action)
}

fn subscriber_protocol_machine(now: Instant, revision: u64) -> ConnectionMachine {
    let (mut machine, action) = pending_protocol_machine(now, b"{\"cmd\":\"subscribe\"}\n");
    assert_eq!(action, MachineAction::Request(Request::Subscribe));
    let state = protocol_state(revision);
    assert_eq!(machine.complete_subscribe(now, state), MachineAction::None);
    machine
}

fn protocol_state(revision: u64) -> RealmState {
    RealmState {
        revision,
        ..RealmState::default()
    }
}

fn output_event(machine: &ConnectionMachine) -> Event {
    let frame = std::str::from_utf8(machine.output().unwrap()).unwrap();
    realm_core::ipc::decode(frame).unwrap()
}

fn publish_protocol_state(
    machine: &mut ConnectionMachine,
    now: Instant,
    state: &RealmState,
) -> MachineAction {
    let frame = realm_core::ipc::encode(&Event::State(Box::new(state.clone()))).unwrap();
    machine.publish_state(now, frame.as_bytes())
}

#[test]
fn hard_read_deadlines_and_impossible_full_prefix_close_exactly() {
    let base = Instant::now();
    let mut awaiting = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        awaiting.next_deadline(),
        Some(base + Duration::from_secs(1))
    );
    assert_eq!(
        awaiting.expire(base + Duration::from_secs(1) - Duration::from_nanos(1)),
        MachineAction::None
    );
    assert_eq!(
        awaiting.expire(base + Duration::from_secs(1)),
        MachineAction::Close(MachineClose::Deadline)
    );

    // Catches sliding the Ready partial-frame deadline on later input.
    let mut ready = ready_protocol_machine(base);
    let first_byte = base + Duration::from_millis(10);
    assert_eq!(ready.ingest(first_byte, b"{"), MachineAction::None);
    assert_eq!(
        ready.next_deadline(),
        Some(first_byte + Duration::from_secs(2))
    );
    assert_eq!(
        ready.ingest(first_byte + Duration::from_secs(1), b" "),
        MachineAction::None
    );
    assert_eq!(
        ready.next_deadline(),
        Some(first_byte + Duration::from_secs(2))
    );
    assert_eq!(
        ready.expire(first_byte + Duration::from_secs(2) - Duration::from_nanos(1)),
        MachineAction::None
    );
    assert_eq!(
        ready.expire(first_byte + Duration::from_secs(2)),
        MachineAction::Close(MachineClose::Deadline)
    );

    let mut retained_partial = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        retained_partial.ingest(
            first_byte,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n{"
        ),
        MachineAction::None
    );
    let hello_len = retained_partial.output().unwrap().len();
    assert_eq!(
        retained_partial.advance_output(first_byte + Duration::from_secs(1), hello_len),
        MachineAction::None
    );
    assert_eq!(
        retained_partial.next_deadline(),
        Some(first_byte + Duration::from_secs(2))
    );
}

#[test]
fn all_output_classes_obey_exact_nonblocking_deadlines() {
    let base = Instant::now();

    // Catches resetting a two-second output deadline without positive progress.
    let mut no_progress = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        no_progress.ingest(
            base,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n"
        ),
        MachineAction::None
    );
    assert_eq!(
        no_progress.advance_output(base + Duration::from_secs(1), 0),
        MachineAction::None
    );
    assert_eq!(
        no_progress.expire(base + Duration::from_secs(2)),
        MachineAction::Close(MachineClose::Deadline)
    );

    let mut progress = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        progress.ingest(
            base,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n"
        ),
        MachineAction::None
    );
    assert_eq!(
        progress.advance_output(base + Duration::from_secs(1), 1),
        MachineAction::None
    );
    assert_eq!(
        progress.next_deadline(),
        Some(base + Duration::from_secs(3))
    );
    assert_eq!(
        progress.expire(base + Duration::from_secs(2)),
        MachineAction::None
    );
    assert_eq!(
        progress.expire(base + Duration::from_secs(3)),
        MachineAction::Close(MachineClose::Deadline)
    );

    // Catches sliding the hard terminal deadline after positive output.
    let mut terminal = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        terminal.ingest(
            base,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":2,\"client\":\"test\"}}\n"
        ),
        MachineAction::None
    );
    assert_eq!(
        terminal.advance_output(base + Duration::from_millis(50), 1),
        MachineAction::None
    );
    assert_eq!(
        terminal.next_deadline(),
        Some(base + Duration::from_millis(100))
    );
    assert_eq!(
        terminal.expire(base + Duration::from_millis(100)),
        MachineAction::Close(MachineClose::Deadline)
    );

    let mut subscriber = subscriber_protocol_machine(base, 1);
    assert_eq!(
        subscriber.advance_output(base + Duration::from_secs(1), 1),
        MachineAction::None
    );
    assert_eq!(
        subscriber.next_deadline(),
        Some(base + Duration::from_secs(3))
    );

    let (mut ordinary, action) = pending_protocol_machine(base, b"{\"cmd\":\"get-state\"}\n");
    assert_eq!(action, MachineAction::Request(Request::GetState));
    assert_eq!(
        ordinary.complete_request(base, &Response::Ok),
        MachineAction::None
    );
    assert_eq!(
        ordinary.advance_output(base + Duration::from_secs(1), 1),
        MachineAction::None
    );
    assert_eq!(
        ordinary.next_deadline(),
        Some(base + Duration::from_secs(3))
    );

    let mut shutdown = subscriber_protocol_machine(base, 1);
    let initial_len = shutdown.output().unwrap().len();
    assert_eq!(
        shutdown.advance_output(base, initial_len),
        MachineAction::None
    );
    assert_eq!(shutdown.begin_shutdown(base), MachineAction::None);
    assert_eq!(
        shutdown.advance_output(base + Duration::from_millis(50), 1),
        MachineAction::None
    );
    assert_eq!(
        shutdown.next_deadline(),
        Some(base + Duration::from_millis(100))
    );
}

#[test]
fn shutdown_subscriber_input_interest_tracks_read_half_close() {
    let base = Instant::now();
    let mut read_open = subscriber_protocol_machine(base, 1);
    assert_eq!(read_open.begin_shutdown(base), MachineAction::None);

    // Catches suppressing subscriber reads merely because shutdown is draining.
    assert!(read_open.input_enabled());
    assert_eq!(
        read_open.ingest(base, b"x"),
        MachineAction::Close(MachineClose::SubscriberInput)
    );

    let mut half_closed = subscriber_protocol_machine(base, 1);
    assert_eq!(half_closed.read_eof(base), MachineAction::None);
    assert_eq!(half_closed.begin_shutdown(base), MachineAction::None);
    assert!(!half_closed.input_enabled());
}

#[test]
fn oversized_hello_reply_returns_close_action() {
    let now = Instant::now();
    let oversized_session = "x".repeat(MAX_FRAME_BYTES);
    let mut machine = ConnectionMachine::new(now, &oversized_session);

    // Catches discarding the close action produced by bounded Hello encoding.
    assert_eq!(
        machine.ingest(
            now,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"test\"}}\n"
        ),
        MachineAction::Close(MachineClose::FrameTooLarge)
    );
    assert_eq!(machine.phase(), ConnectionPhase::Closing);
    assert_eq!(machine.output(), None);
    assert!(!machine.input_enabled());
}

#[test]
fn half_close_preserves_authorized_two_frame_work_and_subscribers() {
    let base = Instant::now();
    let mut ordinary = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        ordinary.ingest(
            base,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"shell\"}}\n{\"cmd\":\"get-state\"}\n"
        ),
        MachineAction::None
    );
    assert_eq!(ordinary.read_eof(base), MachineAction::None);
    let hello_len = ordinary.output().unwrap().len();
    assert_eq!(
        ordinary.advance_output(base, hello_len),
        MachineAction::Request(Request::GetState)
    );
    assert_eq!(
        ordinary.complete_request(base, &Response::Ok),
        MachineAction::None
    );
    assert_eq!(ordinary.output(), Some(b"{\"reply\":\"ok\"}\n".as_slice()));
    assert_eq!(
        ordinary.advance_output(base, b"{\"reply\":\"ok\"}\n".len()),
        MachineAction::Close(MachineClose::PeerClosed)
    );

    let mut subscriber = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        subscriber.ingest(
            base,
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"bar\"}}\n{\"cmd\":\"subscribe\"}\n"
        ),
        MachineAction::None
    );
    assert_eq!(subscriber.read_eof(base), MachineAction::None);
    let hello_len = subscriber.output().unwrap().len();
    assert_eq!(
        subscriber.advance_output(base, hello_len),
        MachineAction::Request(Request::Subscribe)
    );
    let initial = protocol_state(7);
    let update = RealmState {
        revision: 8,
        ..initial.clone()
    };
    assert_eq!(
        subscriber.complete_subscribe(base, initial),
        MachineAction::None
    );
    let initial_len = subscriber.output().unwrap().len();
    assert_eq!(
        subscriber.advance_output(base, initial_len),
        MachineAction::None
    );
    assert_eq!(subscriber.phase(), ConnectionPhase::Subscriber);
    assert!(!subscriber.input_enabled());
    assert_eq!(
        publish_protocol_state(&mut subscriber, base, &update),
        MachineAction::None
    );
    assert!(matches!(
        output_event(&subscriber),
        Event::State(state) if state.revision == 8
    ));

    let mut before_hello = ConnectionMachine::new(base, "test-session");
    assert_eq!(
        before_hello.read_eof(base),
        MachineAction::Close(MachineClose::PeerClosed)
    );
    let mut partial = ready_protocol_machine(base);
    assert_eq!(partial.ingest(base, b"{"), MachineAction::None);
    assert_eq!(
        partial.read_eof(base),
        MachineAction::Close(MachineClose::PeerClosed)
    );
}

#[test]
fn read_half_closed_application_error_drains_then_closes() {
    let base = Instant::now();
    let (mut machine, action) = pending_protocol_machine(base, b"{\"cmd\":\"quit\"}\n");
    assert_eq!(action, MachineAction::Request(Request::Quit));
    assert_eq!(machine.read_eof(base), MachineAction::None);
    assert_eq!(
        machine.complete_request(
            base,
            &Response::Error {
                message: "application refused".to_owned(),
            }
        ),
        MachineAction::None
    );
    let expected = b"{\"reply\":\"error\",\"data\":{\"message\":\"application refused\"}}\n";
    assert_eq!(machine.output(), Some(expected.as_slice()));
    assert_eq!(
        machine.advance_output(base, expected.len()),
        MachineAction::Close(MachineClose::PeerClosed)
    );
}

#[test]
fn subscriber_current_and_latest_coalesce_and_shutdown_exactly() {
    let base = Instant::now();
    for first_progress in [0, 1] {
        let mut machine = subscriber_protocol_machine(base, 1);
        let initial = machine.output().unwrap().to_vec();
        if first_progress == 1 {
            assert_eq!(machine.advance_output(base, 1), MachineAction::None);
            assert_eq!(machine.output(), Some(&initial[1..]));
        }
        let state = protocol_state(2);
        assert_eq!(
            publish_protocol_state(&mut machine, base, &state),
            MachineAction::None
        );
        let state = protocol_state(3);
        assert_eq!(
            publish_protocol_state(&mut machine, base, &state),
            MachineAction::None
        );

        // Catches replacing initial A or queueing both B and C.
        assert_eq!(machine.output(), Some(&initial[first_progress..]));
        assert_eq!(
            machine.advance_output(base, initial.len() - first_progress),
            MachineAction::None
        );
        assert!(matches!(
            output_event(&machine),
            Event::State(state) if state.revision == 3
        ));
        let latest_len = machine.output().unwrap().len();
        assert_eq!(
            machine.advance_output(base, latest_len),
            MachineAction::None
        );
        assert_eq!(machine.output(), None);
    }

    let mut positive_input = subscriber_protocol_machine(base, 1);
    assert_eq!(
        positive_input.ingest(base, b"x"),
        MachineAction::Close(MachineClose::SubscriberInput)
    );
}

#[test]
fn shutdown_preserves_unstarted_initial_state_before_shutdown() {
    let base = Instant::now();
    let mut machine = subscriber_protocol_machine(base, 11);
    let initial = machine.output().unwrap().to_vec();
    let stale_latest = protocol_state(12);
    assert_eq!(
        publish_protocol_state(&mut machine, base, &stale_latest),
        MachineAction::None
    );
    assert_eq!(machine.begin_shutdown(base), MachineAction::None);
    assert_eq!(machine.output(), Some(initial.as_slice()));
    assert_eq!(
        machine.advance_output(base, initial.len()),
        MachineAction::None
    );
    assert_eq!(output_event(&machine), Event::Shutdown);
    let shutdown_len = machine.output().unwrap().len();
    assert_eq!(
        machine.advance_output(base, shutdown_len),
        MachineAction::Close(MachineClose::Shutdown)
    );
}

#[test]
fn shutdown_queue_branches_and_deadline_are_total_and_idempotent() {
    let base = Instant::now();

    // A partial initial frame finishes alone.
    let mut partial_initial = subscriber_protocol_machine(base, 1);
    let discarded = protocol_state(9);
    assert_eq!(
        publish_protocol_state(&mut partial_initial, base, &discarded),
        MachineAction::None
    );
    assert_eq!(partial_initial.advance_output(base, 1), MachineAction::None);
    assert_eq!(partial_initial.begin_shutdown(base), MachineAction::None);
    let remaining = partial_initial.output().unwrap().len();
    assert_eq!(
        partial_initial.advance_output(base, remaining),
        MachineAction::Close(MachineClose::Shutdown)
    );

    // An unstarted later current is replaced by Shutdown.
    let mut unstarted_later = subscriber_protocol_machine(base, 1);
    let initial_len = unstarted_later.output().unwrap().len();
    assert_eq!(
        unstarted_later.advance_output(base, initial_len),
        MachineAction::None
    );
    let update = protocol_state(2);
    assert_eq!(
        publish_protocol_state(&mut unstarted_later, base, &update),
        MachineAction::None
    );
    assert_eq!(unstarted_later.begin_shutdown(base), MachineAction::None);
    assert_eq!(output_event(&unstarted_later), Event::Shutdown);

    // A partial later current finishes alone.
    let mut partial_later = subscriber_protocol_machine(base, 1);
    let initial_len = partial_later.output().unwrap().len();
    assert_eq!(
        partial_later.advance_output(base, initial_len),
        MachineAction::None
    );
    assert_eq!(
        publish_protocol_state(&mut partial_later, base, &update),
        MachineAction::None
    );
    assert_eq!(partial_later.advance_output(base, 1), MachineAction::None);
    let newer_update = protocol_state(3);
    assert_eq!(
        publish_protocol_state(&mut partial_later, base, &newer_update),
        MachineAction::None
    );
    assert_eq!(partial_later.begin_shutdown(base), MachineAction::None);
    let remaining = partial_later.output().unwrap().len();
    assert_eq!(
        partial_later.advance_output(base, remaining),
        MachineAction::Close(MachineClose::Shutdown)
    );

    // With no current, Shutdown is queued directly.
    let mut idle = subscriber_protocol_machine(base, 1);
    let initial_len = idle.output().unwrap().len();
    assert_eq!(idle.advance_output(base, initial_len), MachineAction::None);
    assert_eq!(idle.begin_shutdown(base), MachineAction::None);
    assert_eq!(output_event(&idle), Event::Shutdown);

    // Catches moving the first hard shutdown deadline or rebuilding output.
    let queued = idle.output().unwrap().to_vec();
    assert_eq!(
        idle.begin_shutdown(base + Duration::from_secs(1)),
        MachineAction::None
    );
    assert_eq!(idle.output(), Some(queued.as_slice()));
    assert_eq!(
        idle.next_deadline(),
        Some(base + Duration::from_millis(100))
    );
    assert_eq!(
        idle.expire(base + Duration::from_millis(100) - Duration::from_nanos(1)),
        MachineAction::None
    );
    assert_eq!(
        idle.expire(base + Duration::from_millis(100)),
        MachineAction::Close(MachineClose::Deadline)
    );

    let mut ordinary = ready_protocol_machine(base);
    assert_eq!(
        ordinary.begin_shutdown(base),
        MachineAction::Close(MachineClose::Shutdown)
    );
}
