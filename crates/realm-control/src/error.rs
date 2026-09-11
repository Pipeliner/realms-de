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
