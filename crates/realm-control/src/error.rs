use std::fmt;

use rustix::io::Errno;

/// Failure while resolving or validating a control-endpoint filesystem path.
#[derive(Debug)]
pub enum IpcPathError {
    MissingRuntimeDir,
    UnsafeRuntimeDir,
    UnsafeRealmDirectory,
    UnsafeSocketEntry,
    EndpointInUse,
    Io(std::io::Error),
}

/// Failure while servicing the bounded control server.
#[derive(Debug)]
pub enum ControlError {
    StaleConnection {
        connection: crate::ConnectionId,
    },
    ResponseSequenceExhausted {
        connection: crate::ConnectionId,
    },
    ResponseBarrierConflict {
        active: crate::ResponseReceipt,
        requested: crate::ResponseReceipt,
    },
    ShuttingDown,
    OutboundFrameTooLarge {
        connections: Vec<crate::ConnectionId>,
    },
    PeerIo {
        connection: crate::ConnectionId,
        source: std::io::Error,
    },
    ListenerIo(std::io::Error),
    ListenerTerminal,
    ResourceExhausted(std::io::Error),
}

/// The client operation whose bounded work failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientPhase {
    Connect,
    HelloWrite,
    HelloRead,
    RequestWrite,
    ResponseRead,
    SubscribeWrite,
    InitialState,
    SubscriptionEvent,
}

/// Failure while making one control-client attempt or exchanging one frame.
#[derive(Debug)]
pub enum ClientError {
    MissingRealm,
    Refused,
    Path(IpcPathError),
    VersionMismatch {
        client: u32,
        server: u32,
    },
    Timeout {
        phase: ClientPhase,
    },
    FrameTooLarge {
        phase: ClientPhase,
    },
    InvalidRequest,
    UnexpectedResponse {
        phase: ClientPhase,
    },
    MalformedResponse {
        phase: ClientPhase,
    },
    Eof {
        phase: ClientPhase,
    },
    Io {
        phase: ClientPhase,
        source: std::io::Error,
    },
}

impl ClientError {
    /// Whether the realmctl startup driver may make its next scheduled attempt.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::MissingRealm | Self::Refused)
    }
}

impl fmt::Display for IpcPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRuntimeDir => {
                formatter.write_str("runtime directory is missing or invalid")
            }
            Self::UnsafeRuntimeDir => formatter.write_str("runtime directory is unsafe"),
            Self::UnsafeRealmDirectory => formatter.write_str("realm directory is unsafe"),
            Self::UnsafeSocketEntry => formatter.write_str("control socket entry is unsafe"),
            Self::EndpointInUse => formatter.write_str("control endpoint is already in use"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for IpcPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Errno> for IpcPathError {
    fn from(error: Errno) -> Self {
        Self::Io(std::io::Error::from(error))
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleConnection { connection } => {
                write!(formatter, "stale control connection {connection:?}")
            }
            Self::ResponseSequenceExhausted { connection } => write!(
                formatter,
                "control response sequence exhausted for {connection:?}"
            ),
            Self::ResponseBarrierConflict { active, requested } => write!(
                formatter,
                "control response barrier conflict: active {active:?}, requested {requested:?}"
            ),
            Self::ShuttingDown => formatter.write_str("control server is shutting down"),
            Self::OutboundFrameTooLarge { connections } => write!(
                formatter,
                "outbound control frame exceeded the bound for {connections:?}"
            ),
            Self::PeerIo { connection, source } => {
                write!(
                    formatter,
                    "control peer {connection:?} I/O failed: {source}"
                )
            }
            Self::ListenerIo(source) => write!(formatter, "control listener I/O failed: {source}"),
            Self::ListenerTerminal => formatter.write_str("control listener became terminal"),
            Self::ResourceExhausted(source) => {
                write!(formatter, "control listener resource exhaustion: {source}")
            }
        }
    }
}

impl std::error::Error for ControlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::PeerIo { source, .. }
            | Self::ListenerIo(source)
            | Self::ResourceExhausted(source) => Some(source),
            Self::StaleConnection { .. }
            | Self::ResponseSequenceExhausted { .. }
            | Self::ResponseBarrierConflict { .. }
            | Self::ShuttingDown
            | Self::OutboundFrameTooLarge { .. }
            | Self::ListenerTerminal => None,
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRealm => formatter.write_str("realm runtime directory is missing"),
            Self::Refused => formatter.write_str("control endpoint refused the connection"),
            Self::Path(error) => write!(formatter, "control endpoint path failed: {error}"),
            Self::VersionMismatch { client, server } => write!(
                formatter,
                "control protocol version mismatch: client {client}, server {server}"
            ),
            Self::Timeout { phase } => {
                write!(formatter, "control client timed out during {phase:?}")
            }
            Self::FrameTooLarge { phase } => {
                write!(formatter, "control frame was too large during {phase:?}")
            }
            Self::InvalidRequest => formatter.write_str("invalid control client request"),
            Self::UnexpectedResponse { phase } => {
                write!(formatter, "unexpected control response during {phase:?}")
            }
            Self::MalformedResponse { phase } => {
                write!(formatter, "malformed control response during {phase:?}")
            }
            Self::Eof { phase } => {
                write!(formatter, "control peer closed during {phase:?}")
            }
            Self::Io { phase, source } => {
                write!(
                    formatter,
                    "control client I/O failed during {phase:?}: {source}"
                )
            }
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Path(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
