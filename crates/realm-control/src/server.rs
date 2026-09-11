use std::collections::BTreeMap;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::time::Instant;

use realm_core::ipc::{self, Event, Request, Response, MAX_FRAME_BYTES};
use realm_core::state::RealmState;
use rustix::io::Errno;
use rustix::net::sockopt::socket_peercred;
use rustix::net::{accept_with, recv, send, RecvFlags, SendFlags, SocketFlags};

use crate::protocol::{ConnectionMachine, ConnectionPhase, MachineAction};
use crate::{ActiveControlListener, ControlError};

const CONNECTION_LIMIT: usize = 64;
const ACCEPT_FLAGS: SocketFlags = SocketFlags::NONBLOCK.union(SocketFlags::CLOEXEC);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlToken(u64);

pub struct PollInterest<'a> {
    pub token: ControlToken,
    pub fd: BorrowedFd<'a>,
    pub readable: bool,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyEvent {
    pub token: ControlToken,
    pub readable: bool,
    pub writable: bool,
}

#[derive(Debug, PartialEq)]
pub enum ControlAction {
    Request {
        connection: ConnectionId,
        request: Request,
    },
}

struct Connection {
    fd: OwnedFd,
    token: ControlToken,
    machine: ConnectionMachine,
}

pub struct ControlServer {
    listener: ActiveControlListener,
    listener_token: ControlToken,
    connections: BTreeMap<ConnectionId, Connection>,
    next_connection_id: u64,
    next_token: u64,
    shutting_down: bool,
    #[cfg(test)]
    test_credentials: std::collections::VecDeque<TestPeerCredential>,
    #[cfg(test)]
    receive_calls: usize,
    #[cfg(test)]
    accept_calls: usize,
    #[cfg(test)]
    send_calls: usize,
    #[cfg(test)]
    accept_errors: std::collections::VecDeque<Errno>,
    #[cfg(test)]
    test_receives: std::collections::VecDeque<TestReceive>,
    #[cfg(test)]
    test_sends: std::collections::VecDeque<TestSend>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum TestPeerCredential {
    Missing,
    Uid(u32),
    Error(Errno),
}

#[cfg(test)]
#[derive(Debug)]
pub enum TestReceive {
    Bytes(Vec<u8>),
    Eof,
    Error(Errno),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub enum TestSend {
    Count(usize),
    Error(Errno),
}

impl ControlServer {
    pub(crate) fn new(listener: ActiveControlListener, _now: Instant) -> Self {
        Self {
            listener,
            listener_token: ControlToken(0),
            connections: BTreeMap::new(),
            next_connection_id: 1,
            next_token: 1,
            shutting_down: false,
            #[cfg(test)]
            test_credentials: std::collections::VecDeque::new(),
            #[cfg(test)]
            receive_calls: 0,
            #[cfg(test)]
            accept_calls: 0,
            #[cfg(test)]
            send_calls: 0,
            #[cfg(test)]
            accept_errors: std::collections::VecDeque::new(),
            #[cfg(test)]
            test_receives: std::collections::VecDeque::new(),
            #[cfg(test)]
            test_sends: std::collections::VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_test_credentials(
        listener: ActiveControlListener,
        now: Instant,
        credentials: impl IntoIterator<Item = TestPeerCredential>,
    ) -> Self {
        let mut server = Self::new(listener, now);
        server.test_credentials = credentials.into_iter().collect();
        server
    }

    pub fn poll_interests(&self) -> impl Iterator<Item = PollInterest<'_>> {
        let listener = (!self.shutting_down).then(|| PollInterest {
            token: self.listener_token,
            fd: self.listener.socket_fd(),
            readable: true,
            writable: false,
        });
        let peers = self.connections.values().map(|connection| PollInterest {
            token: connection.token,
            fd: connection.fd.as_fd(),
            readable: connection.machine.input_enabled(),
            writable: connection.machine.output().is_some(),
        });
        listener.into_iter().chain(peers)
    }

    pub fn service_one(
        &mut self,
        now: Instant,
        ready: ReadyEvent,
    ) -> Result<Option<ControlAction>, ControlError> {
        let known_peer = self.connection_id_for_token(ready.token);
        self.expire(now)?;

        if ready.token == self.listener_token {
            if self.shutting_down || !ready.readable {
                return Ok(None);
            }
            return self.accept_one(now).map(|()| None);
        }
        let Some(connection_id) = known_peer else {
            return Ok(None);
        };
        let Some(connection) = self.connections.get(&connection_id) else {
            return Ok(None);
        };
        let read_enabled = connection.machine.input_enabled();
        let write_enabled = connection.machine.output().is_some();
        if ready.readable && read_enabled {
            self.receive_one(now, connection_id)
        } else if ready.writable && write_enabled {
            self.send_one(now, connection_id)
        } else {
            Ok(None)
        }
    }

    fn accept_one(&mut self, now: Instant) -> Result<(), ControlError> {
        let accepted = match self.accept_socket() {
            Ok(fd) => fd,
            Err(Errno::INTR | Errno::AGAIN | Errno::CONNABORTED) => return Ok(()),
            Err(error @ (Errno::MFILE | Errno::NFILE | Errno::NOBUFS | Errno::NOMEM)) => {
                return Err(ControlError::ResourceExhausted(error.into()));
            }
            Err(error) => return Err(ControlError::ListenerIo(error.into())),
        };

        let admitted_uid = self.peer_uid(accepted.as_fd());
        if admitted_uid != Some(self.listener.realm_dir().retained_euid()) {
            return Ok(());
        }
        if self.connections.len() == CONNECTION_LIMIT {
            return Ok(());
        }

        let connection_id = ConnectionId(self.next_connection_id);
        let token = ControlToken(self.next_token);
        self.next_connection_id += 1;
        self.next_token += 1;
        self.connections.insert(
            connection_id,
            Connection {
                fd: accepted,
                token,
                machine: ConnectionMachine::new(now, env!("CARGO_PKG_VERSION")),
            },
        );
        Ok(())
    }

    fn accept_socket(&mut self) -> rustix::io::Result<OwnedFd> {
        #[cfg(test)]
        {
            self.accept_calls += 1;
            if let Some(error) = self.accept_errors.pop_front() {
                return Err(error);
            }
        }
        accept_with(self.listener.socket_fd(), ACCEPT_FLAGS)
    }

    fn peer_uid(&mut self, fd: BorrowedFd<'_>) -> Option<u32> {
        #[cfg(test)]
        if let Some(outcome) = self.test_credentials.pop_front() {
            return match outcome {
                TestPeerCredential::Missing | TestPeerCredential::Error(_) => None,
                TestPeerCredential::Uid(uid) => Some(uid),
            };
        }
        socket_peercred(fd)
            .ok()
            .map(|credentials| credentials.uid.as_raw())
    }

    fn connection_id_for_token(&self, token: ControlToken) -> Option<ConnectionId> {
        self.connections
            .iter()
            .find_map(|(id, connection)| (connection.token == token).then_some(*id))
    }

    fn receive_one(
        &mut self,
        now: Instant,
        connection_id: ConnectionId,
    ) -> Result<Option<ControlAction>, ControlError> {
        let mut buffer = [0_u8; MAX_FRAME_BYTES];
        let result = self.receive_socket(connection_id, &mut buffer);
        let action = match result {
            Ok(0) => self
                .connections
                .get_mut(&connection_id)
                .expect("selected connection remains present")
                .machine
                .read_eof(now),
            Ok(count) => self
                .connections
                .get_mut(&connection_id)
                .expect("selected connection remains present")
                .machine
                .ingest(now, &buffer[..count]),
            Err(Errno::INTR | Errno::AGAIN) => return Ok(None),
            Err(Errno::CONNRESET) => {
                self.connections.remove(&connection_id);
                return Ok(None);
            }
            Err(error) => {
                self.connections.remove(&connection_id);
                return Err(ControlError::PeerIo {
                    connection: connection_id,
                    source: error.into(),
                });
            }
        };
        Ok(self.apply_machine_action(connection_id, action))
    }

    fn receive_socket(
        &mut self,
        connection_id: ConnectionId,
        buffer: &mut [u8],
    ) -> rustix::io::Result<usize> {
        #[cfg(test)]
        {
            self.receive_calls += 1;
            if let Some(outcome) = self.test_receives.pop_front() {
                return match outcome {
                    TestReceive::Bytes(bytes) => {
                        assert!(bytes.len() <= buffer.len());
                        buffer[..bytes.len()].copy_from_slice(&bytes);
                        Ok(bytes.len())
                    }
                    TestReceive::Eof => Ok(0),
                    TestReceive::Error(error) => Err(error),
                };
            }
        }
        let connection = self
            .connections
            .get(&connection_id)
            .expect("selected connection remains present");
        recv(connection.fd.as_fd(), buffer, RecvFlags::empty()).map(|(_, count)| count)
    }

    fn send_one(
        &mut self,
        now: Instant,
        connection_id: ConnectionId,
    ) -> Result<Option<ControlAction>, ControlError> {
        let result = self.send_socket(connection_id);
        let count = match result {
            Ok(count) => count,
            Err(Errno::INTR | Errno::AGAIN) => return Ok(None),
            Err(Errno::CONNRESET | Errno::PIPE) => {
                self.connections.remove(&connection_id);
                return Ok(None);
            }
            Err(error) => {
                self.connections.remove(&connection_id);
                return Err(ControlError::PeerIo {
                    connection: connection_id,
                    source: error.into(),
                });
            }
        };
        let action = self
            .connections
            .get_mut(&connection_id)
            .expect("selected connection remains present")
            .machine
            .advance_output(now, count);
        Ok(self.apply_machine_action(connection_id, action))
    }

    fn send_socket(&mut self, connection_id: ConnectionId) -> rustix::io::Result<usize> {
        #[cfg(test)]
        {
            self.send_calls += 1;
            if let Some(outcome) = self.test_sends.pop_front() {
                return match outcome {
                    TestSend::Count(count) => Ok(count),
                    TestSend::Error(error) => Err(error),
                };
            }
        }
        let connection = self
            .connections
            .get(&connection_id)
            .expect("selected connection remains present");
        send(
            connection.fd.as_fd(),
            connection
                .machine
                .output()
                .expect("write selected only with output"),
            SendFlags::NOSIGNAL,
        )
    }

    fn apply_machine_action(
        &mut self,
        connection_id: ConnectionId,
        action: MachineAction,
    ) -> Option<ControlAction> {
        match action {
            MachineAction::None => None,
            MachineAction::Request(request) => Some(ControlAction::Request {
                connection: connection_id,
                request,
            }),
            MachineAction::Close(_) => {
                self.connections.remove(&connection_id);
                None
            }
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.connections
            .values()
            .filter_map(|connection| connection.machine.next_deadline())
            .min()
    }

    pub fn expire(&mut self, now: Instant) -> Result<(), ControlError> {
        let due: Vec<_> = self
            .connections
            .iter_mut()
            .filter_map(|(id, connection)| {
                matches!(connection.machine.expire(now), MachineAction::Close(_)).then_some(*id)
            })
            .collect();
        for connection in due {
            self.connections.remove(&connection);
        }
        Ok(())
    }

    pub fn complete_request(
        &mut self,
        now: Instant,
        connection: ConnectionId,
        response: Response,
    ) -> Result<(), ControlError> {
        if self.shutting_down {
            return Err(ControlError::ShuttingDown);
        }
        let Some(record) = self.connections.get_mut(&connection) else {
            return Err(ControlError::StaleConnection { connection });
        };
        let action = record.machine.complete_request(now, &response);
        if matches!(action, MachineAction::Close(_)) {
            self.connections.remove(&connection);
            return Err(ControlError::OutboundFrameTooLarge {
                connections: vec![connection],
            });
        }
        Ok(())
    }

    pub fn complete_subscribe(
        &mut self,
        now: Instant,
        connection: ConnectionId,
        state: RealmState,
    ) -> Result<(), ControlError> {
        if self.shutting_down {
            return Err(ControlError::ShuttingDown);
        }
        let Some(record) = self.connections.get_mut(&connection) else {
            return Err(ControlError::StaleConnection { connection });
        };
        let action = record.machine.complete_subscribe(now, &state);
        if matches!(action, MachineAction::Close(_)) {
            self.connections.remove(&connection);
            return Err(ControlError::OutboundFrameTooLarge {
                connections: vec![connection],
            });
        }
        Ok(())
    }

    pub fn publish_state(&mut self, now: Instant, state: RealmState) -> Result<(), ControlError> {
        if self.shutting_down {
            return Err(ControlError::ShuttingDown);
        }
        let subscribers: Vec<_> = self
            .connections
            .iter()
            .filter_map(|(id, connection)| {
                (connection.machine.phase() == ConnectionPhase::Subscriber).then_some(*id)
            })
            .collect();
        if subscribers.is_empty() {
            return Ok(());
        }
        let frame = match ipc::encode(&Event::State(Box::new(state))) {
            Ok(frame) => frame,
            Err(_) => {
                for connection in &subscribers {
                    self.connections.remove(connection);
                }
                return Err(ControlError::OutboundFrameTooLarge {
                    connections: subscribers,
                });
            }
        };
        for connection in subscribers {
            let action = self
                .connections
                .get_mut(&connection)
                .expect("subscriber set is stable during publication")
                .machine
                .publish_state(now, frame.as_bytes());
            if matches!(action, MachineAction::Close(_)) {
                self.connections.remove(&connection);
            }
        }
        Ok(())
    }

    pub fn begin_shutdown(&mut self, now: Instant) {
        if self.shutting_down {
            return;
        }
        self.shutting_down = true;
        let connections: Vec<_> = self.connections.keys().copied().collect();
        for connection in connections {
            let action = self
                .connections
                .get_mut(&connection)
                .expect("shutdown snapshot contains live connections")
                .machine
                .begin_shutdown(now);
            if matches!(action, MachineAction::Close(_)) {
                self.connections.remove(&connection);
            }
        }
    }

    pub fn is_shutdown_complete(&self) -> bool {
        self.shutting_down && self.connections.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn connection_count_for_test(&self) -> usize {
        self.connections.len()
    }

    #[cfg(test)]
    pub(crate) fn receive_calls_for_test(&self) -> usize {
        self.receive_calls
    }

    #[cfg(test)]
    pub(crate) fn accept_calls_for_test(&self) -> usize {
        self.accept_calls
    }

    #[cfg(test)]
    pub(crate) fn send_calls_for_test(&self) -> usize {
        self.send_calls
    }

    #[cfg(test)]
    pub(crate) fn socket_calls_for_test(&self) -> usize {
        self.accept_calls + self.receive_calls + self.send_calls
    }

    #[cfg(test)]
    pub(crate) fn inject_accept_error_for_test(&mut self, error: Errno) {
        self.accept_errors.push_back(error);
    }

    #[cfg(test)]
    pub(crate) fn inject_receive_for_test(&mut self, outcome: TestReceive) {
        self.test_receives.push_back(outcome);
    }

    #[cfg(test)]
    pub(crate) fn inject_send_for_test(&mut self, outcome: TestSend) {
        self.test_sends.push_back(outcome);
    }

    #[cfg(test)]
    pub(crate) fn connection_id_for_token_for_test(
        &self,
        token: ControlToken,
    ) -> Option<ConnectionId> {
        self.connection_id_for_token(token)
    }

    #[cfg(test)]
    pub(crate) fn interest_for_test(&self, token: ControlToken) -> Option<(bool, bool)> {
        self.connection_id_for_token(token).and_then(|id| {
            self.connections.get(&id).map(|connection| {
                (
                    connection.machine.input_enabled(),
                    connection.machine.output().is_some(),
                )
            })
        })
    }

    #[cfg(test)]
    pub(crate) fn output_len_for_test(&self, connection: ConnectionId) -> Option<usize> {
        self.connections
            .get(&connection)
            .and_then(|record| record.machine.output().map(<[u8]>::len))
    }

    #[cfg(test)]
    pub(crate) fn has_connection_for_test(&self, connection: ConnectionId) -> bool {
        self.connections.contains_key(&connection)
    }

    #[cfg(test)]
    pub(crate) fn connection_ids_for_test(&self) -> Vec<ConnectionId> {
        self.connections.keys().copied().collect()
    }

    #[cfg(test)]
    pub(crate) fn credential_outcomes_for_test(&self) -> usize {
        self.test_credentials.len()
    }
}
