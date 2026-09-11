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
    ShuttingDown,
    OutboundFrameTooLarge {
        connections: Vec<crate::ConnectionId>,
    },
    PeerIo {
        connection: crate::ConnectionId,
        source: std::io::Error,
    },
    ListenerIo(std::io::Error),
    ResourceExhausted(std::io::Error),
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
            | Self::ShuttingDown
            | Self::OutboundFrameTooLarge { .. } => None,
        }
    }
}
