use std::os::fd::{BorrowedFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{fstat, openat2, Mode, OFlags, ResolveFlags, Stat, CWD};
use rustix::net::SocketAddrUnix;
use rustix::process::umask;

use crate::IpcPathError;

pub(crate) const REALM_DIRECTORY: &str = "realm";
pub(crate) const CONTROL_SOCKET: &str = "ctl.sock";
pub(crate) const SUN_PATH_CAPACITY: usize = 108;

const DIRECTORY_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

pub(crate) trait RuntimeBridge {
    fn open(&self, path: &Path) -> rustix::io::Result<OwnedFd>;
    fn stat(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<Stat>;
}

pub(crate) struct ProcFdRuntimeBridge;

impl RuntimeBridge for ProcFdRuntimeBridge {
    fn open(&self, path: &Path) -> rustix::io::Result<OwnedFd> {
        openat2(
            CWD,
            path,
            DIRECTORY_OPEN_FLAGS,
            Mode::empty(),
            ResolveFlags::empty(),
        )
    }

    fn stat(&self, fd: BorrowedFd<'_>) -> rustix::io::Result<Stat> {
        fstat(fd)
    }
}

pub(crate) struct ScopedUmask {
    previous: Mode,
}

impl ScopedUmask {
    pub(crate) fn new(mask: Mode) -> Self {
        Self {
            previous: umask(mask),
        }
    }
}

impl Drop for ScopedUmask {
    fn drop(&mut self) {
        umask(self.previous);
    }
}

pub(crate) fn runtime_procfd_path(fd: RawFd) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{fd}"))
}

pub(crate) fn procfd_socket_path(fd: RawFd) -> PathBuf {
    runtime_procfd_path(fd).join(CONTROL_SOCKET)
}

pub(crate) fn canonical_socket_path(runtime_path: &Path) -> PathBuf {
    runtime_path.join(REALM_DIRECTORY).join(CONTROL_SOCKET)
}

pub(crate) fn validate_socket_path(path: &Path, capacity: usize) -> Result<(), IpcPathError> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.contains(&0) {
        return Err(IpcPathError::from(rustix::io::Errno::INVAL));
    }
    if bytes.len().checked_add(1).is_none_or(|len| len > capacity) {
        return Err(IpcPathError::from(rustix::io::Errno::NAMETOOLONG));
    }
    Ok(())
}

pub(crate) fn socket_addr_un(path: &Path) -> Result<SocketAddrUnix, IpcPathError> {
    validate_socket_path(path, SUN_PATH_CAPACITY)?;
    SocketAddrUnix::new(path).map_err(IpcPathError::from)
}
