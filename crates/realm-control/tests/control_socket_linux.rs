use std::fs;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use realm_control::{
    test_runtime_dir, ClientEndpoint, ConnectionId, ControlAction, ControlServer, ControlToken,
    ReadyEvent,
};
use realm_core::ipc::{self, Event, Request, Response, PROTOCOL_VERSION};
use realm_core::state::RealmState;
use rustix::io::ioctl_fionread;
use rustix::net::sockopt::set_socket_recv_buffer_size;
use tempfile::TempDir;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
struct TestDeadline(Instant);

impl TestDeadline {
    fn new() -> Self {
        Self(Instant::now() + TEST_TIMEOUT)
    }

    fn check(self, operation: &str) {
        assert!(
            Instant::now() < self.0,
            "{operation} exceeded the absolute three-second test deadline"
        );
    }
}

struct Fixture {
    runtime: TempDir,
    socket_path: PathBuf,
    server: ControlServer,
    listener_token: ControlToken,
}

impl Fixture {
    fn new() -> Self {
        static STARTUP: OnceLock<Mutex<()>> = OnceLock::new();
        let startup_guard = STARTUP
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let runtime = tempfile::tempdir().expect("create an absolute test runtime directory");
        fs::set_permissions(runtime.path(), fs::Permissions::from_mode(0o700))
            .expect("make the test runtime directory exact 0700");
        let endpoint = test_runtime_dir(runtime.path())
            .expect("resolve the test runtime")
            .prepare_server_endpoint()
            .expect("prepare the fixed server endpoint");
        let bound = endpoint.bind().expect("bind the fixed server endpoint");
        let listener = bound.activate().expect("activate the server endpoint");
        let socket_path = listener.endpoint().path().to_path_buf();
        let server = listener.into_server(Instant::now());
        let interests = copy_interests(&server);
        let listener_token = interests
            .iter()
            .find(|(_, readable, writable)| *readable && !*writable)
            .map(|(token, _, _)| *token)
            .expect("a new server exposes its listener interest");
        drop(startup_guard);

        Self {
            runtime,
            socket_path,
            server,
            listener_token,
        }
    }

    fn client_endpoint(&self) -> ClientEndpoint {
        test_runtime_dir(self.runtime.path())
            .expect("reopen the validated runtime for the real client")
            .client_endpoint()
    }

    fn connect_raw(&mut self, deadline: TestDeadline) -> (UnixStream, ControlToken) {
        deadline.check("raw peer connect");
        let existing = self.peer_tokens();
        let stream = UnixStream::connect(&self.socket_path).expect("connect the raw unix peer");
        stream
            .set_nonblocking(true)
            .expect("make the raw peer nonblocking");
        self.server
            .service_one(
                Instant::now(),
                ReadyEvent {
                    token: self.listener_token,
                    readable: true,
                    terminal: false,
                    writable: false,
                },
            )
            .expect("accept one real unix peer");
        let token = self
            .peer_tokens()
            .into_iter()
            .find(|token| !existing.contains(token))
            .expect("the same-euid peer is admitted with a stable token");
        (stream, token)
    }

    fn peer_tokens(&self) -> Vec<ControlToken> {
        copy_interests(&self.server)
            .into_iter()
            .filter_map(|(token, _, _)| (token != self.listener_token).then_some(token))
            .collect()
    }

    fn interest(&self, token: ControlToken) -> Option<(bool, bool)> {
        copy_interests(&self.server)
            .into_iter()
            .find_map(|(candidate, readable, writable)| {
                (candidate == token).then_some((readable, writable))
            })
    }

    fn service(
        &mut self,
        token: ControlToken,
        readable: bool,
        writable: bool,
    ) -> Option<ControlAction> {
        self.server
            .service_one(
                Instant::now(),
                ReadyEvent {
                    token,
                    readable,
                    terminal: false,
                    writable,
                },
            )
            .expect("service one real nonblocking socket quantum")
    }

    fn receive_pipeline(&mut self, token: ControlToken) -> Option<ControlAction> {
        self.service(token, true, false)
    }

    fn drain_writes(
        &mut self,
        token: ControlToken,
        deadline: TestDeadline,
    ) -> Option<ControlAction> {
        let mut action = None;
        loop {
            deadline.check("server output drain");
            let Some((_, writable)) = self.interest(token) else {
                return action;
            };
            if !writable {
                return action;
            }
            let next = self.service(token, false, true);
            assert!(
                action.is_none() || next.is_none(),
                "one request is exposed once"
            );
            action = action.or(next);
            thread::yield_now();
        }
    }
}

fn copy_interests(server: &ControlServer) -> Vec<(ControlToken, bool, bool)> {
    server
        .poll_interests()
        .map(|interest| (interest.token, interest.readable, interest.writable))
        .collect()
}

fn encode_requests(requests: &[Request]) -> Vec<u8> {
    requests
        .iter()
        .flat_map(|request| {
            ipc::encode(request)
                .expect("integration request fits the frame bound")
                .into_bytes()
        })
        .collect()
}

fn write_all_nonblocking(stream: &UnixStream, bytes: &[u8], deadline: TestDeadline) {
    let mut stream = stream;
    let mut written = 0;
    while written < bytes.len() {
        deadline.check("raw peer write");
        match stream.write(&bytes[written..]) {
            Ok(0) => panic!("raw peer write made no progress"),
            Ok(count) => written += count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::yield_now(),
            Err(error) => panic!("raw peer write failed: {error}"),
        }
    }
}

fn read_to_eof(stream: &UnixStream, deadline: TestDeadline) -> Vec<u8> {
    let mut stream = stream;
    let mut output = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        deadline.check("raw peer read to EOF");
        match stream.read(&mut chunk) {
            Ok(0) => return output,
            Ok(count) => output.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::yield_now(),
            Err(error) => panic!("raw peer read failed: {error}"),
        }
    }
}

fn decode_response_lines(bytes: &[u8]) -> Vec<Response> {
    let text = std::str::from_utf8(bytes).expect("control output is UTF-8");
    assert!(text.ends_with('\n'), "every output frame is LF terminated");
    text.lines()
        .map(|line| ipc::decode(line).expect("control output is a decodable response frame"))
        .collect()
}

fn expect_request(action: Option<ControlAction>, expected: Request) -> ConnectionId {
    match action {
        Some(ControlAction::Request {
            connection,
            request,
        }) => {
            assert_eq!(request, expected);
            connection
        }
        other => panic!("expected one application request, got {other:?}"),
    }
}

fn assert_server_hello(response: &Response) {
    assert!(
        matches!(response, Response::Hello { version, .. } if *version == PROTOCOL_VERSION),
        "first frame must be the server Hello, got {response:?}"
    );
}

fn respond(server: &mut ControlServer, action: ControlAction) {
    let now = Instant::now();
    match action {
        ControlAction::Request {
            connection,
            request: Request::GetState,
        } => {
            server
                .complete_request(now, connection, Response::State(Box::default()))
                .expect("the test responder completes GetState");
        }
        ControlAction::Request {
            connection,
            request: Request::Subscribe,
        } => server
            .complete_subscribe(now, connection, RealmState::default())
            .expect("the test responder completes Subscribe"),
        ControlAction::Request { connection, .. } => {
            server
                .complete_request(
                    now,
                    connection,
                    Response::Error {
                        message: "unsupported by integration fixture".into(),
                    },
                )
                .expect("the test responder completes an unsupported request");
        }
    }
}

struct ServerThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Result<(), String>>>,
}

impl ServerThread {
    fn spawn(mut server: ControlServer, deadline: TestDeadline) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                if Instant::now() >= deadline.0 {
                    return Err("server thread exceeded its absolute three-second deadline".into());
                }
                let interests = copy_interests(&server);
                for (token, readable, writable) in interests {
                    if let Some(action) = server
                        .service_one(
                            Instant::now(),
                            ReadyEvent {
                                token,
                                readable,
                                terminal: false,
                                writable,
                            },
                        )
                        .map_err(|error| error.to_string())?
                    {
                        respond(&mut server, action);
                    }
                }
                thread::yield_now();
            }
            Ok(())
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) {
        self.stop.store(true, Ordering::Release);
        let result = self
            .handle
            .take()
            .expect("server thread handle is present")
            .join()
            .expect("server thread does not panic");
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }
}

impl Drop for ServerThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn spawn(command: &mut Command) -> Self {
        Self {
            child: Some(command.spawn().expect("launch socat")),
        }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child process is present")
    }

    fn wait(mut self, deadline: TestDeadline) -> (ExitStatus, Vec<u8>, Vec<u8>) {
        let status = loop {
            deadline.check("socat child");
            match self
                .child_mut()
                .try_wait()
                .expect("query socat child status")
            {
                Some(status) => break status,
                None => thread::yield_now(),
            }
        };
        let child = self.child_mut();
        let mut stdout = Vec::new();
        child
            .stdout
            .take()
            .expect("capture socat stdout")
            .read_to_end(&mut stdout)
            .expect("read socat stdout after exit");
        let mut stderr = Vec::new();
        child
            .stderr
            .take()
            .expect("capture socat stderr")
            .read_to_end(&mut stderr)
            .expect("read socat stderr after exit");
        self.child.take();
        (status, stdout, stderr)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn matching_hello_and_get_state_survive_write_half_eof_in_order() {
    let deadline = TestDeadline::new();
    let mut fixture = Fixture::new();
    let (stream, token) = fixture.connect_raw(deadline);
    write_all_nonblocking(
        &stream,
        &encode_requests(&[
            Request::Hello {
                version: PROTOCOL_VERSION,
                client: "half-close-request".into(),
            },
            Request::GetState,
        ]),
        deadline,
    );
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close request input");

    assert!(fixture.receive_pipeline(token).is_none());
    let connection = expect_request(fixture.drain_writes(token, deadline), Request::GetState);
    let expected = RealmState::default();
    fixture
        .server
        .complete_request(
            Instant::now(),
            connection,
            Response::State(Box::new(expected.clone())),
        )
        .expect("complete the retained GetState request");
    assert!(fixture.drain_writes(token, deadline).is_none());
    assert!(fixture.service(token, true, false).is_none());

    let responses = decode_response_lines(&read_to_eof(&stream, deadline));
    assert_eq!(responses.len(), 2);
    assert_server_hello(&responses[0]);
    assert_eq!(responses[1], Response::State(Box::new(expected)));
}

#[test]
fn mismatched_hello_discards_a_pipelined_request_and_sends_only_hello() {
    let deadline = TestDeadline::new();
    let mut fixture = Fixture::new();
    let (stream, token) = fixture.connect_raw(deadline);
    write_all_nonblocking(
        &stream,
        &encode_requests(&[
            Request::Hello {
                version: PROTOCOL_VERSION + 1,
                client: "mismatch-pipeline".into(),
            },
            Request::GetState,
        ]),
        deadline,
    );
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close mismatched input");

    assert!(fixture.receive_pipeline(token).is_none());
    assert!(fixture.drain_writes(token, deadline).is_none());
    assert!(fixture.interest(token).is_none());

    let responses = decode_response_lines(&read_to_eof(&stream, deadline));
    assert_eq!(responses.len(), 1);
    assert_server_hello(&responses[0]);
}

#[test]
fn matching_hello_and_subscribe_survive_clean_read_half_eof() {
    let deadline = TestDeadline::new();
    let mut fixture = Fixture::new();
    let (stream, token) = fixture.connect_raw(deadline);
    write_all_nonblocking(
        &stream,
        &encode_requests(&[
            Request::Hello {
                version: PROTOCOL_VERSION,
                client: "half-close-subscriber".into(),
            },
            Request::Subscribe,
        ]),
        deadline,
    );
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close subscriber input");

    assert!(fixture.receive_pipeline(token).is_none());
    let connection = expect_request(fixture.drain_writes(token, deadline), Request::Subscribe);
    let initial = RealmState::default();
    fixture
        .server
        .complete_subscribe(Instant::now(), connection, initial.clone())
        .expect("complete the retained Subscribe request");
    assert!(fixture.drain_writes(token, deadline).is_none());
    assert!(fixture.service(token, true, false).is_none());
    assert_eq!(fixture.interest(token), Some((false, false)));

    let mut changed = initial.clone();
    changed.revision = 7;
    fixture
        .server
        .publish_state(Instant::now(), changed.clone())
        .expect("publish after the subscriber read half closed");
    assert!(fixture.drain_writes(token, deadline).is_none());
    fixture.server.begin_shutdown(Instant::now());
    assert!(fixture.drain_writes(token, deadline).is_none());

    let output = read_to_eof(&stream, deadline);
    let text = std::str::from_utf8(&output).expect("subscriber output is UTF-8");
    let mut lines = text.lines();
    let hello: Response = ipc::decode(lines.next().expect("server Hello frame")).unwrap();
    assert_server_hello(&hello);
    let events: Vec<Event> = lines.map(|line| ipc::decode(line).unwrap()).collect();
    assert_eq!(
        events,
        vec![
            Event::State(Box::new(initial)),
            Event::State(Box::new(changed)),
            Event::Shutdown,
        ]
    );
}

#[test]
fn a_full_send_buffer_returns_from_one_nonblocking_quantum() {
    let deadline = TestDeadline::new();
    let mut fixture = Fixture::new();
    let (stream, token) = fixture.connect_raw(deadline);
    set_socket_recv_buffer_size(stream.as_fd(), 4 * 1024)
        .expect("shrink the receiving peer buffer for deterministic backpressure");
    write_all_nonblocking(
        &stream,
        &encode_requests(&[
            Request::Hello {
                version: PROTOCOL_VERSION,
                client: "backpressure".into(),
            },
            Request::GetState,
        ]),
        deadline,
    );

    assert!(fixture.receive_pipeline(token).is_none());
    let first_connection = expect_request(fixture.drain_writes(token, deadline), Request::GetState);
    let large_response = Response::Error {
        message: "x".repeat(60_000),
    };
    assert!(ipc::encode(&large_response).unwrap().len() < ipc::MAX_FRAME_BYTES);
    fixture
        .server
        .complete_request(Instant::now(), first_connection, large_response.clone())
        .expect("queue the first large response");

    for request_index in 0..64 {
        deadline.check("fill the real unix send buffer");
        while fixture
            .interest(token)
            .is_some_and(|(_, writable)| writable)
        {
            let before = ioctl_fionread(stream.as_fd()).expect("measure queued peer input");
            let quantum_started = Instant::now();
            assert!(fixture.service(token, false, true).is_none());
            let quantum_elapsed = quantum_started.elapsed();
            let after = ioctl_fionread(stream.as_fd()).expect("remeasure queued peer input");
            if before == after {
                assert_eq!(fixture.interest(token), Some((false, true)));
                assert!(
                    quantum_elapsed < Duration::from_millis(100),
                    "one EAGAIN quantum blocked for {quantum_elapsed:?}"
                );
                return;
            }
        }

        write_all_nonblocking(&stream, &encode_requests(&[Request::GetState]), deadline);
        let connection = expect_request(fixture.receive_pipeline(token), Request::GetState);
        fixture
            .server
            .complete_request(Instant::now(), connection, large_response.clone())
            .unwrap_or_else(|error| panic!("queue large response {request_index}: {error}"));
    }
    panic!("the real unix send buffer never reached EAGAIN");
}

#[test]
fn a_healthy_peer_completes_beside_a_stalled_partial_frame() {
    let deadline = TestDeadline::new();
    let mut fixture = Fixture::new();
    let (stalled, stalled_token) = fixture.connect_raw(deadline);
    write_all_nonblocking(&stalled, b"{", deadline);
    assert!(fixture.receive_pipeline(stalled_token).is_none());

    let (healthy, healthy_token) = fixture.connect_raw(deadline);
    write_all_nonblocking(
        &healthy,
        &encode_requests(&[
            Request::Hello {
                version: PROTOCOL_VERSION,
                client: "healthy-neighbour".into(),
            },
            Request::GetState,
        ]),
        deadline,
    );
    healthy
        .shutdown(std::net::Shutdown::Write)
        .expect("half-close healthy input");
    assert!(fixture.receive_pipeline(healthy_token).is_none());
    let connection = expect_request(
        fixture.drain_writes(healthy_token, deadline),
        Request::GetState,
    );
    fixture
        .server
        .complete_request(Instant::now(), connection, Response::State(Box::default()))
        .expect("complete healthy GetState");
    assert!(fixture.drain_writes(healthy_token, deadline).is_none());
    assert!(fixture.service(healthy_token, true, false).is_none());
    let responses = decode_response_lines(&read_to_eof(&healthy, deadline));
    assert_eq!(responses.len(), 2);
    assert_server_hello(&responses[0]);
    assert!(matches!(responses[1], Response::State(_)));
    assert!(fixture.interest(stalled_token).is_some());

    fixture
        .server
        .expire(Instant::now() + Duration::from_secs(2))
        .expect("expire the stalled peer without socket I/O");
    assert!(fixture.interest(stalled_token).is_none());
}

#[test]
fn public_client_and_server_interoperate_with_actual_same_euid_admission() {
    let deadline = TestDeadline::new();
    let fixture = Fixture::new();
    let client_endpoint = fixture.client_endpoint();
    let Fixture {
        runtime,
        server,
        socket_path: _,
        listener_token: _,
    } = fixture;
    let server_thread = ServerThread::spawn(server, deadline);

    let mut client = client_endpoint
        .connect("real-client")
        .expect("real public client completes Hello through real SO_PEERCRED admission");
    let response = client
        .request(Request::GetState)
        .expect("real public client receives the test-owned application response");
    assert_eq!(response, Response::State(Box::default()));

    drop(client);
    server_thread.finish();
    drop(runtime);
}

#[test]
#[ignore = "CI installs socat for this external-client interoperability acceptance"]
fn shell_two_frame_interoperability_is_ordered() {
    let deadline = TestDeadline::new();
    let fixture = Fixture::new();
    let socket_path = fixture.socket_path.clone();
    let Fixture {
        runtime,
        server,
        socket_path: _,
        listener_token: _,
    } = fixture;
    let server_thread = ServerThread::spawn(server, deadline);

    let mut command = Command::new("socat");
    command
        .arg("-")
        .arg(format!("UNIX-CONNECT:{}", socket_path.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::spawn(&mut command);
    child
        .child_mut()
        .stdin
        .take()
        .expect("pipe literal frames to socat")
        .write_all(
            b"{\"cmd\":\"hello\",\"arg\":{\"version\":1,\"client\":\"socat\"}}\n{\"cmd\":\"get-state\"}\n",
        )
        .expect("write the two literal LF-terminated ADR frames");

    let (status, stdout, stderr) = child.wait(deadline);
    assert!(
        status.success(),
        "socat failed: {}",
        String::from_utf8_lossy(&stderr)
    );
    let responses = decode_response_lines(&stdout);
    assert_eq!(responses.len(), 2);
    assert_server_hello(&responses[0]);
    assert_eq!(responses[1], Response::State(Box::default()));

    server_thread.finish();
    drop(runtime);
}
