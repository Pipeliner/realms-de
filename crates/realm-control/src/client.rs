use std::fmt;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::time::{Duration, Instant};

use realm_core::ipc::{self, Event, Request, Response, MAX_FRAME_BYTES, PROTOCOL_VERSION};
use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::io::Errno;
use rustix::net::sockopt::socket_error;
use rustix::net::{
    connect, recv, send, socket_with, AddressFamily, RecvFlags, SendFlags, SocketAddrUnix,
    SocketFlags, SocketType,
};

use crate::sys::{procfd_socket_path, socket_addr_un};
use crate::{ClientError, ClientPhase, RuntimeDir};

const SOCKET_FLAGS: SocketFlags = SocketFlags::NONBLOCK.union(SocketFlags::CLOEXEC);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2);
const READ_CHUNK_BYTES: usize = 8 * 1024;

trait ClientOperations {
    fn now(&mut self) -> Instant;
    fn connect(&mut self, fd: BorrowedFd<'_>, address: &SocketAddrUnix) -> rustix::io::Result<()>;
    fn poll(
        &mut self,
        fd: BorrowedFd<'_>,
        flags: PollFlags,
        timeout: Option<Duration>,
    ) -> rustix::io::Result<usize>;
    fn socket_error(&mut self, fd: BorrowedFd<'_>) -> rustix::io::Result<rustix::io::Result<()>>;
    fn send(
        &mut self,
        fd: BorrowedFd<'_>,
        bytes: &[u8],
        flags: SendFlags,
    ) -> rustix::io::Result<usize>;
    fn receive(&mut self, fd: BorrowedFd<'_>, bytes: &mut [u8]) -> rustix::io::Result<usize>;
}

struct RealClientOperations;

impl ClientOperations for RealClientOperations {
    fn now(&mut self) -> Instant {
        Instant::now()
    }

    fn connect(&mut self, fd: BorrowedFd<'_>, address: &SocketAddrUnix) -> rustix::io::Result<()> {
        connect(fd, address)
    }

    fn poll(
        &mut self,
        fd: BorrowedFd<'_>,
        flags: PollFlags,
        timeout: Option<Duration>,
    ) -> rustix::io::Result<usize> {
        let mut poll_fds = [PollFd::from_borrowed_fd(fd, flags)];
        let timeout = timeout.map(duration_timespec);
        poll(&mut poll_fds, timeout.as_ref())
    }

    fn socket_error(&mut self, fd: BorrowedFd<'_>) -> rustix::io::Result<rustix::io::Result<()>> {
        socket_error(fd)
    }

    fn send(
        &mut self,
        fd: BorrowedFd<'_>,
        bytes: &[u8],
        flags: SendFlags,
    ) -> rustix::io::Result<usize> {
        send(fd, bytes, flags)
    }

    fn receive(&mut self, fd: BorrowedFd<'_>, bytes: &mut [u8]) -> rustix::io::Result<usize> {
        let (count, _) = recv(fd, bytes, RecvFlags::empty())?;
        Ok(count)
    }
}

fn duration_timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: duration
            .as_secs()
            .try_into()
            .expect("client deadlines fit in Timespec"),
        tv_nsec: duration.subsec_nanos().into(),
    }
}

/// A reusable client endpoint retaining the originally validated runtime fd.
pub struct ClientEndpoint {
    runtime: RuntimeDir,
}

struct FramedTransport {
    fd: OwnedFd,
    input: Vec<u8>,
    partial_deadline: Option<Instant>,
    operations: Box<dyn ClientOperations>,
}

/// One connected control transport.
pub struct Client {
    transport: FramedTransport,
}

/// A consuming stream of state and shutdown events.
pub struct Subscription {
    transport: FramedTransport,
    pending: Option<Event>,
    finished: bool,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Client").finish_non_exhaustive()
    }
}

impl ClientEndpoint {
    pub(crate) fn new(runtime: RuntimeDir) -> Self {
        Self { runtime }
    }

    /// Makes exactly one descriptor-relative connection attempt and completes Hello.
    pub fn connect(&self, client: &str) -> Result<Client, ClientError> {
        self.connect_with_operations(client, Box::new(RealClientOperations))
    }

    fn connect_with_operations(
        &self,
        client: &str,
        mut operations: Box<dyn ClientOperations>,
    ) -> Result<Client, ClientError> {
        let realm = self.runtime.open_client_realm_dir()?;
        let path = procfd_socket_path(realm.as_fd().as_raw_fd());
        let address = socket_addr_un(&path).map_err(ClientError::Path)?;
        let fd = socket_with(AddressFamily::UNIX, SocketType::STREAM, SOCKET_FLAGS, None)
            .map_err(|error| client_io(ClientPhase::Connect, error))?;

        let connect_started = operations.now();
        match operations.connect(fd.as_fd(), &address) {
            Ok(()) => {}
            Err(Errno::AGAIN | Errno::INPROGRESS) => {
                let deadline = connect_started + CONNECT_TIMEOUT;
                poll_until(
                    &mut *operations,
                    fd.as_fd(),
                    PollFlags::OUT,
                    Some(deadline),
                    ClientPhase::Connect,
                )?;
                match operations.socket_error(fd.as_fd()) {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => return Err(classify_connect_error(error)),
                    Err(error) => return Err(client_io(ClientPhase::Connect, error)),
                }
            }
            Err(error) => return Err(classify_connect_error(error)),
        }

        let mut transport = FramedTransport {
            fd,
            input: Vec::new(),
            partial_deadline: None,
            operations,
        };
        let deadline = transport.operations.now() + EXCHANGE_TIMEOUT;
        let hello = Request::Hello {
            version: PROTOCOL_VERSION,
            client: client.to_owned(),
        };
        transport.send_request(&hello, deadline, ClientPhase::HelloWrite)?;
        match transport.read_response(ReadBound::Deadline(deadline), ClientPhase::HelloRead)? {
            Response::Hello { version, .. } if version == PROTOCOL_VERSION => {
                Ok(Client { transport })
            }
            Response::Hello { version, .. } => Err(ClientError::VersionMismatch {
                client: PROTOCOL_VERSION,
                server: version,
            }),
            _ => Err(ClientError::UnexpectedResponse {
                phase: ClientPhase::HelloRead,
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_with_test_operations(
        &self,
        client: &str,
        operations: TestClientOperations,
    ) -> Result<Client, ClientError> {
        self.connect_with_operations(client, Box::new(operations))
    }
}

impl Client {
    /// Sends one ordinary request and receives one response within one deadline.
    pub fn request(&mut self, request: Request) -> Result<Response, ClientError> {
        if matches!(request, Request::Hello { .. } | Request::Subscribe) {
            return Err(ClientError::InvalidRequest);
        }
        let deadline = self.transport.operations.now() + EXCHANGE_TIMEOUT;
        self.transport
            .send_request(&request, deadline, ClientPhase::RequestWrite)?;
        self.transport
            .read_response(ReadBound::Deadline(deadline), ClientPhase::ResponseRead)
    }

    /// Consumes the client, sends Subscribe, and waits for the initial State.
    pub fn subscribe(mut self) -> Result<Subscription, ClientError> {
        let deadline = self.transport.operations.now() + EXCHANGE_TIMEOUT;
        self.transport
            .send_request(&Request::Subscribe, deadline, ClientPhase::SubscribeWrite)?;
        let initial = self
            .transport
            .read_event(ReadBound::Deadline(deadline), ClientPhase::InitialState)?;
        if !matches!(initial, Event::State(_)) {
            return Err(ClientError::UnexpectedResponse {
                phase: ClientPhase::InitialState,
            });
        }
        Ok(Subscription {
            transport: self.transport,
            pending: Some(initial),
            finished: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn fd_for_test(&self) -> BorrowedFd<'_> {
        self.transport.fd.as_fd()
    }
}

impl Iterator for Subscription {
    type Item = Result<Event, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let result = match self.pending.take() {
            Some(event) => Ok(event),
            None => self
                .transport
                .read_event(ReadBound::Subscription, ClientPhase::SubscriptionEvent),
        };
        if matches!(result, Ok(Event::Shutdown) | Err(_)) {
            self.finished = true;
        }
        Some(result)
    }
}

#[derive(Clone, Copy)]
enum ReadBound {
    Deadline(Instant),
    Subscription,
}

impl FramedTransport {
    fn send_request(
        &mut self,
        request: &Request,
        deadline: Instant,
        phase: ClientPhase,
    ) -> Result<(), ClientError> {
        let frame = ipc::encode(request).map_err(|error| match error {
            realm_core::Error::IpcFrameTooLarge { .. } => ClientError::FrameTooLarge { phase },
            other => ClientError::Io {
                phase,
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, other),
            },
        })?;
        let mut offset = 0;
        while offset < frame.len() {
            ensure_before(&mut *self.operations, deadline, phase)?;
            match self.operations.send(
                self.fd.as_fd(),
                &frame.as_bytes()[offset..],
                SendFlags::NOSIGNAL,
            ) {
                Ok(0) => {
                    return Err(ClientError::Io {
                        phase,
                        source: std::io::Error::from(std::io::ErrorKind::WriteZero),
                    });
                }
                Ok(count) if count <= frame.len() - offset => offset += count,
                Ok(_) => {
                    return Err(ClientError::Io {
                        phase,
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "send returned more bytes than supplied",
                        ),
                    });
                }
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => poll_until(
                    &mut *self.operations,
                    self.fd.as_fd(),
                    PollFlags::OUT,
                    Some(deadline),
                    phase,
                )?,
                Err(error) => return Err(client_io(phase, error)),
            }
        }
        Ok(())
    }

    fn read_response(
        &mut self,
        bound: ReadBound,
        phase: ClientPhase,
    ) -> Result<Response, ClientError> {
        let frame = self.read_frame(bound, phase)?;
        decode_response(&frame, phase)
    }

    fn read_event(&mut self, bound: ReadBound, phase: ClientPhase) -> Result<Event, ClientError> {
        let frame = self.read_frame(bound, phase)?;
        decode_event(&frame, phase)
    }

    fn read_frame(&mut self, bound: ReadBound, phase: ClientPhase) -> Result<Vec<u8>, ClientError> {
        loop {
            if let Some(newline) = self.input.iter().position(|byte| *byte == b'\n') {
                let trailing = self.input.split_off(newline + 1);
                let frame = std::mem::replace(&mut self.input, trailing);
                self.partial_deadline =
                    (!self.input.is_empty()).then(|| self.operations.now() + EXCHANGE_TIMEOUT);
                return Ok(frame);
            }
            if self.input.len() >= MAX_FRAME_BYTES {
                return Err(ClientError::FrameTooLarge { phase });
            }

            let active_deadline = match bound {
                ReadBound::Deadline(deadline) => Some(deadline),
                ReadBound::Subscription => self.partial_deadline,
            };
            if let Some(deadline) = active_deadline {
                ensure_before(&mut *self.operations, deadline, phase)?;
            }

            let mut bytes = [0_u8; READ_CHUNK_BYTES];
            let capacity = (MAX_FRAME_BYTES - self.input.len()).min(bytes.len());
            match self
                .operations
                .receive(self.fd.as_fd(), &mut bytes[..capacity])
            {
                Ok(0) => return Err(ClientError::Eof { phase }),
                Ok(count) if count <= capacity => {
                    if self.input.is_empty() {
                        self.partial_deadline = Some(self.operations.now() + EXCHANGE_TIMEOUT);
                    }
                    self.input.extend_from_slice(&bytes[..count]);
                }
                Ok(_) => {
                    return Err(ClientError::Io {
                        phase,
                        source: std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "receive returned more bytes than supplied",
                        ),
                    });
                }
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => poll_until(
                    &mut *self.operations,
                    self.fd.as_fd(),
                    PollFlags::IN,
                    active_deadline,
                    phase,
                )?,
                Err(error) => return Err(client_io(phase, error)),
            }
        }
    }
}

fn decode_response(frame: &[u8], phase: ClientPhase) -> Result<Response, ClientError> {
    let text = std::str::from_utf8(frame).map_err(|_| ClientError::MalformedResponse { phase })?;
    ipc::decode(text).map_err(|_| ClientError::MalformedResponse { phase })
}

fn decode_event(frame: &[u8], phase: ClientPhase) -> Result<Event, ClientError> {
    let text = std::str::from_utf8(frame).map_err(|_| ClientError::MalformedResponse { phase })?;
    ipc::decode(text).map_err(|_| ClientError::MalformedResponse { phase })
}

fn ensure_before(
    operations: &mut dyn ClientOperations,
    deadline: Instant,
    phase: ClientPhase,
) -> Result<Duration, ClientError> {
    deadline
        .checked_duration_since(operations.now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or(ClientError::Timeout { phase })
}

fn poll_until(
    operations: &mut dyn ClientOperations,
    fd: BorrowedFd<'_>,
    flags: PollFlags,
    deadline: Option<Instant>,
    phase: ClientPhase,
) -> Result<(), ClientError> {
    loop {
        let timeout = match deadline {
            Some(deadline) => Some(ensure_before(operations, deadline, phase)?),
            None => None,
        };
        match operations.poll(fd, flags, timeout) {
            Ok(0) => return Err(ClientError::Timeout { phase }),
            Ok(_) => {
                if let Some(deadline) = deadline {
                    ensure_before(operations, deadline, phase)?;
                }
                return Ok(());
            }
            Err(Errno::INTR) => {}
            Err(error) => return Err(client_io(phase, error)),
        }
    }
}

fn classify_connect_error(error: Errno) -> ClientError {
    match error {
        Errno::NOENT | Errno::NOTDIR => ClientError::MissingRealm,
        Errno::CONNREFUSED => ClientError::Refused,
        other => client_io(ClientPhase::Connect, other),
    }
}

fn client_io(phase: ClientPhase, error: Errno) -> ClientError {
    ClientError::Io {
        phase,
        source: error.into(),
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum TestClientConnect {
    Success,
    Error(Errno),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum TestClientPoll {
    ReadyAfter(Duration),
    TimeoutAfter(Duration),
    ErrorAfter(Errno, Duration),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum TestClientSocketError {
    Connected,
    Socket(Errno),
    Lookup(Errno),
}

#[cfg(test)]
#[derive(Debug)]
pub enum TestClientSend {
    CountAfter(usize, Duration),
    ErrorAfter(Errno, Duration),
}

#[cfg(test)]
#[derive(Debug)]
pub enum TestClientReceive {
    BytesAfter(Vec<u8>, Duration),
    EofAfter(Duration),
    ErrorAfter(Errno, Duration),
}

#[cfg(test)]
#[derive(Default)]
struct TestClientState {
    now: Option<Instant>,
    connects: std::collections::VecDeque<TestClientConnect>,
    polls: std::collections::VecDeque<TestClientPoll>,
    socket_errors: std::collections::VecDeque<TestClientSocketError>,
    sends: std::collections::VecDeque<TestClientSend>,
    receives: std::collections::VecDeque<TestClientReceive>,
    connect_calls: usize,
    poll_timeouts: Vec<Option<Duration>>,
    send_flags: Vec<SendFlags>,
    sent_bytes: Vec<u8>,
}

#[cfg(test)]
#[derive(Clone)]
pub struct TestClientOperations(std::rc::Rc<std::cell::RefCell<TestClientState>>);

#[cfg(test)]
impl TestClientOperations {
    pub fn new(now: Instant) -> Self {
        let state = TestClientState {
            now: Some(now),
            ..TestClientState::default()
        };
        Self(std::rc::Rc::new(std::cell::RefCell::new(state)))
    }

    pub fn push_connect(&self, result: TestClientConnect) {
        self.0.borrow_mut().connects.push_back(result);
    }

    pub fn push_poll(&self, result: TestClientPoll) {
        self.0.borrow_mut().polls.push_back(result);
    }

    pub fn push_socket_error(&self, result: TestClientSocketError) {
        self.0.borrow_mut().socket_errors.push_back(result);
    }

    pub fn push_send(&self, result: TestClientSend) {
        self.0.borrow_mut().sends.push_back(result);
    }

    pub fn push_receive(&self, result: TestClientReceive) {
        self.0.borrow_mut().receives.push_back(result);
    }

    pub fn connect_calls(&self) -> usize {
        self.0.borrow().connect_calls
    }

    pub fn poll_timeouts(&self) -> Vec<Option<Duration>> {
        self.0.borrow().poll_timeouts.clone()
    }

    pub fn send_flags(&self) -> Vec<SendFlags> {
        self.0.borrow().send_flags.clone()
    }

    pub fn sent_bytes(&self) -> Vec<u8> {
        self.0.borrow().sent_bytes.clone()
    }

    fn advance(state: &mut TestClientState, duration: Duration) {
        state.now = Some(state.now.expect("test clock initialized") + duration);
    }
}

#[cfg(test)]
impl ClientOperations for TestClientOperations {
    fn now(&mut self) -> Instant {
        self.0.borrow().now.expect("test clock initialized")
    }

    fn connect(
        &mut self,
        _fd: BorrowedFd<'_>,
        _address: &SocketAddrUnix,
    ) -> rustix::io::Result<()> {
        let mut state = self.0.borrow_mut();
        state.connect_calls += 1;
        match state
            .connects
            .pop_front()
            .unwrap_or(TestClientConnect::Success)
        {
            TestClientConnect::Success => Ok(()),
            TestClientConnect::Error(error) => Err(error),
        }
    }

    fn poll(
        &mut self,
        _fd: BorrowedFd<'_>,
        _flags: PollFlags,
        timeout: Option<Duration>,
    ) -> rustix::io::Result<usize> {
        let mut state = self.0.borrow_mut();
        state.poll_timeouts.push(timeout);
        match state
            .polls
            .pop_front()
            .unwrap_or(TestClientPoll::ReadyAfter(Duration::ZERO))
        {
            TestClientPoll::ReadyAfter(elapsed) => {
                Self::advance(&mut state, elapsed);
                Ok(1)
            }
            TestClientPoll::TimeoutAfter(elapsed) => {
                Self::advance(&mut state, elapsed);
                Ok(0)
            }
            TestClientPoll::ErrorAfter(error, elapsed) => {
                Self::advance(&mut state, elapsed);
                Err(error)
            }
        }
    }

    fn socket_error(&mut self, _fd: BorrowedFd<'_>) -> rustix::io::Result<rustix::io::Result<()>> {
        match self
            .0
            .borrow_mut()
            .socket_errors
            .pop_front()
            .unwrap_or(TestClientSocketError::Connected)
        {
            TestClientSocketError::Connected => Ok(Ok(())),
            TestClientSocketError::Socket(error) => Ok(Err(error)),
            TestClientSocketError::Lookup(error) => Err(error),
        }
    }

    fn send(
        &mut self,
        _fd: BorrowedFd<'_>,
        bytes: &[u8],
        flags: SendFlags,
    ) -> rustix::io::Result<usize> {
        let mut state = self.0.borrow_mut();
        state.send_flags.push(flags);
        match state.sends.pop_front() {
            Some(TestClientSend::CountAfter(count, elapsed)) => {
                Self::advance(&mut state, elapsed);
                let count = count.min(bytes.len());
                state.sent_bytes.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            Some(TestClientSend::ErrorAfter(error, elapsed)) => {
                Self::advance(&mut state, elapsed);
                Err(error)
            }
            None => {
                state.sent_bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
        }
    }

    fn receive(&mut self, _fd: BorrowedFd<'_>, bytes: &mut [u8]) -> rustix::io::Result<usize> {
        let mut state = self.0.borrow_mut();
        match state
            .receives
            .pop_front()
            .unwrap_or(TestClientReceive::ErrorAfter(Errno::AGAIN, Duration::ZERO))
        {
            TestClientReceive::BytesAfter(mut received, elapsed) => {
                Self::advance(&mut state, elapsed);
                let count = received.len().min(bytes.len());
                let trailing = received.split_off(count);
                bytes[..count].copy_from_slice(&received);
                if !trailing.is_empty() {
                    state
                        .receives
                        .push_front(TestClientReceive::BytesAfter(trailing, Duration::ZERO));
                }
                Ok(count)
            }
            TestClientReceive::EofAfter(elapsed) => {
                Self::advance(&mut state, elapsed);
                Ok(0)
            }
            TestClientReceive::ErrorAfter(error, elapsed) => {
                Self::advance(&mut state, elapsed);
                Err(error)
            }
        }
    }
}
