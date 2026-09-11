use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::{
    fcntl_getfl, flock, fstat, openat2, statat, unlinkat, AtFlags, FileType, FlockOperation, Mode,
    OFlags, ResolveFlags, Stat,
};
use rustix::io::{fcntl_getfd, Errno, FdFlags};
use rustix::net::sockopt::{socket_acceptconn, socket_error};
use rustix::net::{
    bind, connect, getsockname, listen, socket_with, AddressFamily, SocketAddrUnix, SocketFlags,
    SocketType,
};

use crate::sys::{ScopedUmask, CONTROL_SOCKET};
use crate::{IpcPathError, RealmDir};

const REQUIRED_REALM_MODE: u32 = 0o700;
const REQUIRED_SOCKET_MODE: u32 = 0o600;
const LOCK_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);
const LOCK_RESOLVE_FLAGS: ResolveFlags =
    ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_MAGICLINKS);
const SOCKET_FLAGS: SocketFlags = SocketFlags::NONBLOCK.union(SocketFlags::CLOEXEC);
const STALE_POLL_TIMEOUT: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 100_000_000,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeDecision {
    Stale,
    Poll,
    Preserve,
}

#[derive(Debug)]
pub(crate) enum PollCompletion {
    Timeout,
    PollFailure,
    Ready(rustix::io::Result<rustix::io::Result<()>>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SocketIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    raw_mode: u32,
}

/// A private open-file description retaining singleton endpoint ownership.
pub(crate) struct EndpointLock {
    _fd: OwnedFd,
}

pub(crate) trait BindOperations {
    fn bind(&self, socket: BorrowedFd<'_>, address: &SocketAddrUnix) -> rustix::io::Result<()>;
    fn post_bind_stat(&self, realm_dir: BorrowedFd<'_>) -> rustix::io::Result<Stat>;
    fn getsockname(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<SocketAddrUnix>;
    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool>;
}

pub(crate) trait ActivationOperations {
    fn listen(&self, socket: BorrowedFd<'_>, backlog: i32) -> rustix::io::Result<()>;
    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool>;
}

struct OsBindOperations;
struct OsActivationOperations;

struct EndpointOwnership {
    endpoint: SocketEndpoint,
    socket: Option<OwnedFd>,
    identity: SocketIdentity,
    _lock: EndpointLock,
}

/// The exact fixed control-socket descendant and its retained realm capability.
pub struct SocketEndpoint {
    path: PathBuf,
    realm_dir: RealmDir,
    #[allow(dead_code)]
    bind_address: SocketAddrUnix,
}

/// A retained, verified control endpoint which has not started listening.
pub struct BoundControlEndpoint {
    ownership: EndpointOwnership,
}

/// A retained, verified control endpoint which is accepting connections.
pub struct ActiveControlListener {
    ownership: EndpointOwnership,
}

impl SocketEndpoint {
    pub(crate) fn new(path: PathBuf, realm_dir: RealmDir, bind_address: SocketAddrUnix) -> Self {
        Self {
            path,
            realm_dir,
            bind_address,
        }
    }

    /// Returns the canonical display path for external tools.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Borrows the retained, validated realm directory capability.
    pub fn realm_dir(&self) -> &RealmDir {
        &self.realm_dir
    }

    /// Binds and retains the fixed non-listening control endpoint.
    ///
    /// # Single-threaded startup requirement
    ///
    /// This operation temporarily changes the process-wide umask. The complete
    /// interval from [`RuntimeDir::prepare_server_endpoint`] through this bind
    /// and its post-bind verification must run during single-threaded startup.
    /// Do not start another thread until this method returns.
    ///
    /// [`RuntimeDir::prepare_server_endpoint`]: crate::RuntimeDir::prepare_server_endpoint
    pub fn bind(self) -> Result<BoundControlEndpoint, IpcPathError> {
        self.bind_using(&OsBindOperations)
    }

    #[cfg(test)]
    pub(crate) fn bind_with<O: BindOperations>(
        self,
        operations: &O,
    ) -> Result<BoundControlEndpoint, IpcPathError> {
        self.bind_using(operations)
    }

    fn bind_using<O: BindOperations>(
        self,
        operations: &O,
    ) -> Result<BoundControlEndpoint, IpcPathError> {
        let endpoint_lock = self.acquire_lock_and_reclaim()?;
        let socket = socket_with(AddressFamily::UNIX, SocketType::STREAM, SOCKET_FLAGS, None)
            .map_err(IpcPathError::from)?;

        {
            let _umask = ScopedUmask::new(Mode::from_raw_mode(0o177));
            operations
                .bind(socket.as_fd(), &self.bind_address)
                .map_err(IpcPathError::from)?;
        }

        let path_stat = operations
            .post_bind_stat(self.realm_dir.as_fd())
            .map_err(IpcPathError::from)?;
        let identity = self.validate_socket_stat(&path_stat)?;
        let ownership = EndpointOwnership {
            endpoint: self,
            socket: Some(socket),
            identity,
            _lock: endpoint_lock,
        };

        let socket = ownership.socket_fd();
        let status_flags = fcntl_getfl(socket).map_err(IpcPathError::from)?;
        let descriptor_flags = fcntl_getfd(socket).map_err(IpcPathError::from)?;
        if !status_flags.contains(OFlags::NONBLOCK) || !descriptor_flags.contains(FdFlags::CLOEXEC)
        {
            return Err(IpcPathError::UnsafeSocketEntry);
        }
        let actual_address = operations.getsockname(socket).map_err(IpcPathError::from)?;
        if actual_address != ownership.endpoint.bind_address {
            return Err(IpcPathError::UnsafeSocketEntry);
        }
        let accepting = operations
            .socket_acceptconn(socket)
            .map_err(IpcPathError::from)?;
        if accepting {
            return Err(IpcPathError::UnsafeSocketEntry);
        }

        Ok(BoundControlEndpoint { ownership })
    }

    #[allow(dead_code)]
    pub(crate) fn acquire_lock_and_reclaim(&self) -> Result<EndpointLock, IpcPathError> {
        self.acquire_lock_and_reclaim_after(|| {})
    }

    #[cfg(test)]
    pub(crate) fn acquire_lock_and_reclaim_with<F>(
        &self,
        before_recheck: F,
    ) -> Result<EndpointLock, IpcPathError>
    where
        F: FnOnce(),
    {
        self.acquire_lock_and_reclaim_after(before_recheck)
    }

    fn acquire_lock_and_reclaim_after<F>(
        &self,
        before_recheck: F,
    ) -> Result<EndpointLock, IpcPathError>
    where
        F: FnOnce(),
    {
        let retained_euid = self.realm_dir.retained_euid();
        let endpoint_lock = EndpointLock::acquire(&self.realm_dir, retained_euid)?;
        self.reclaim_stale_entry(before_recheck)?;
        Ok(endpoint_lock)
    }

    fn reclaim_stale_entry<F>(&self, before_recheck: F) -> Result<(), IpcPathError>
    where
        F: FnOnce(),
    {
        let initial_stat = match statat(
            self.realm_dir.as_fd(),
            CONTROL_SOCKET,
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(stat) => stat,
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(IpcPathError::from(error)),
        };
        let initial_identity = self.validate_socket_stat(&initial_stat)?;

        if probe_stale(&self.bind_address)? != ProbeDecision::Stale {
            return Err(IpcPathError::EndpointInUse);
        }

        before_recheck();
        let current_stat = statat(
            self.realm_dir.as_fd(),
            CONTROL_SOCKET,
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(IpcPathError::from)?;
        let current_identity = match self.validate_socket_stat(&current_stat) {
            Ok(identity) => identity,
            Err(_) => return Err(IpcPathError::EndpointInUse),
        };
        if current_identity != initial_identity {
            return Err(IpcPathError::EndpointInUse);
        }

        unlinkat(self.realm_dir.as_fd(), CONTROL_SOCKET, AtFlags::empty())
            .map_err(IpcPathError::from)
    }

    pub(crate) fn validate_socket_stat(&self, stat: &Stat) -> Result<SocketIdentity, IpcPathError> {
        validate_socket_stat(stat, self.realm_dir.retained_euid())
    }

    #[cfg(test)]
    pub(crate) fn set_retained_euid_for_test(&mut self, euid: u32) {
        self.realm_dir.set_retained_euid_for_test(euid);
    }
}

impl BoundControlEndpoint {
    /// Borrows the exact fixed endpoint retained by this capability.
    pub fn endpoint(&self) -> &SocketEndpoint {
        &self.ownership.endpoint
    }

    /// Borrows the retained, validated realm directory capability.
    pub fn realm_dir(&self) -> &RealmDir {
        self.endpoint().realm_dir()
    }

    /// Starts accepting connections on this endpoint exactly once.
    pub fn activate(self) -> Result<ActiveControlListener, IpcPathError> {
        self.activate_using(&OsActivationOperations)
    }

    #[cfg(test)]
    pub(crate) fn activate_with<O: ActivationOperations>(
        self,
        operations: &O,
    ) -> Result<ActiveControlListener, IpcPathError> {
        self.activate_using(operations)
    }

    fn activate_using<O: ActivationOperations>(
        self,
        operations: &O,
    ) -> Result<ActiveControlListener, IpcPathError> {
        let ownership = self.ownership;
        let socket = ownership.socket_fd();
        operations.listen(socket, 64).map_err(IpcPathError::from)?;
        if !operations
            .socket_acceptconn(socket)
            .map_err(IpcPathError::from)?
        {
            return Err(IpcPathError::UnsafeSocketEntry);
        }
        Ok(ActiveControlListener { ownership })
    }

    #[cfg(test)]
    pub(crate) fn socket_fd_for_test(&self) -> BorrowedFd<'_> {
        self.ownership.socket_fd()
    }

    #[cfg(test)]
    pub(crate) fn lock_fd_for_test(&self) -> BorrowedFd<'_> {
        self.ownership._lock._fd.as_fd()
    }
}

impl ActiveControlListener {
    /// Borrows the exact fixed endpoint retained by this listener.
    pub fn endpoint(&self) -> &SocketEndpoint {
        &self.ownership.endpoint
    }

    /// Borrows the retained, validated realm directory capability.
    pub fn realm_dir(&self) -> &RealmDir {
        self.endpoint().realm_dir()
    }
}

impl AsFd for ActiveControlListener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.ownership.socket_fd()
    }
}

impl BindOperations for OsBindOperations {
    fn bind(&self, socket: BorrowedFd<'_>, address: &SocketAddrUnix) -> rustix::io::Result<()> {
        bind(socket, address)
    }

    fn post_bind_stat(&self, realm_dir: BorrowedFd<'_>) -> rustix::io::Result<Stat> {
        statat(realm_dir, CONTROL_SOCKET, AtFlags::SYMLINK_NOFOLLOW)
    }

    fn getsockname(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<SocketAddrUnix> {
        getsockname(socket)?.try_into()
    }

    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool> {
        socket_acceptconn(socket)
    }
}

impl ActivationOperations for OsActivationOperations {
    fn listen(&self, socket: BorrowedFd<'_>, backlog: i32) -> rustix::io::Result<()> {
        listen(socket, backlog)
    }

    fn socket_acceptconn(&self, socket: BorrowedFd<'_>) -> rustix::io::Result<bool> {
        socket_acceptconn(socket)
    }
}

impl EndpointOwnership {
    fn socket_fd(&self) -> BorrowedFd<'_> {
        self.socket
            .as_ref()
            .expect("endpoint socket is retained until ownership cleanup")
            .as_fd()
    }
}

impl Drop for EndpointOwnership {
    fn drop(&mut self) {
        drop(self.socket.take());

        let Ok(stat) = statat(
            self.endpoint.realm_dir.as_fd(),
            CONTROL_SOCKET,
            AtFlags::SYMLINK_NOFOLLOW,
        ) else {
            return;
        };
        let Ok(identity) = self.endpoint.validate_socket_stat(&stat) else {
            return;
        };
        if identity != self.identity {
            return;
        }
        let _ = unlinkat(
            self.endpoint.realm_dir.as_fd(),
            CONTROL_SOCKET,
            AtFlags::empty(),
        );
    }
}

impl EndpointLock {
    fn acquire(realm_dir: &RealmDir, retained_euid: u32) -> Result<Self, IpcPathError> {
        let fd = openat2(
            realm_dir.as_fd(),
            ".",
            LOCK_OPEN_FLAGS,
            Mode::empty(),
            LOCK_RESOLVE_FLAGS,
        )
        .map_err(IpcPathError::from)?;
        let expected = fstat(realm_dir.as_fd()).map_err(IpcPathError::from)?;
        let actual = fstat(fd.as_fd()).map_err(IpcPathError::from)?;
        let flags = fcntl_getfd(fd.as_fd()).map_err(IpcPathError::from)?;
        validate_lock_stat(&expected, &actual, retained_euid, flags)?;

        match flock(fd.as_fd(), FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Self { _fd: fd }),
            Err(Errno::AGAIN) => Err(IpcPathError::EndpointInUse),
            Err(error) => Err(IpcPathError::from(error)),
        }
    }
}

pub(crate) fn validate_lock_stat(
    expected: &Stat,
    actual: &Stat,
    euid: u32,
    fd_flags: FdFlags,
) -> Result<(), IpcPathError> {
    let valid_directory = |stat: &Stat| {
        FileType::from_raw_mode(stat.st_mode) == FileType::Directory
            && stat.st_uid == euid
            && Mode::from_raw_mode(stat.st_mode).bits() == REQUIRED_REALM_MODE
    };
    if !valid_directory(expected)
        || !valid_directory(actual)
        || actual.st_dev != expected.st_dev
        || actual.st_ino != expected.st_ino
        || !fd_flags.contains(FdFlags::CLOEXEC)
    {
        return Err(IpcPathError::UnsafeRealmDirectory);
    }
    Ok(())
}

pub(crate) fn validate_socket_stat(stat: &Stat, euid: u32) -> Result<SocketIdentity, IpcPathError> {
    if FileType::from_raw_mode(stat.st_mode) != FileType::Socket
        || stat.st_uid != euid
        || Mode::from_raw_mode(stat.st_mode).bits() != REQUIRED_SOCKET_MODE
    {
        return Err(IpcPathError::UnsafeSocketEntry);
    }
    Ok(SocketIdentity {
        device: stat.st_dev,
        inode: stat.st_ino,
        owner: stat.st_uid,
        raw_mode: stat.st_mode,
    })
}

pub(crate) fn classify_initial_probe(result: rustix::io::Result<()>) -> ProbeDecision {
    match result {
        Err(Errno::CONNREFUSED) => ProbeDecision::Stale,
        Err(Errno::AGAIN | Errno::INPROGRESS) => ProbeDecision::Poll,
        Ok(()) | Err(_) => ProbeDecision::Preserve,
    }
}

pub(crate) fn classify_poll_completion(completion: PollCompletion) -> ProbeDecision {
    match completion {
        PollCompletion::Ready(Ok(Err(Errno::CONNREFUSED))) => ProbeDecision::Stale,
        PollCompletion::Timeout
        | PollCompletion::PollFailure
        | PollCompletion::Ready(Ok(Ok(())) | Ok(Err(_)) | Err(_)) => ProbeDecision::Preserve,
    }
}

fn probe_stale(address: &SocketAddrUnix) -> Result<ProbeDecision, IpcPathError> {
    let socket = socket_with(AddressFamily::UNIX, SocketType::STREAM, SOCKET_FLAGS, None)
        .map_err(IpcPathError::from)?;
    match classify_initial_probe(connect(socket.as_fd(), address)) {
        ProbeDecision::Poll => {
            let mut poll_fds = [PollFd::new(&socket, PollFlags::OUT)];
            let completion = match poll(&mut poll_fds, Some(&STALE_POLL_TIMEOUT)) {
                Ok(0) => PollCompletion::Timeout,
                Ok(_) => PollCompletion::Ready(socket_error(socket.as_fd())),
                Err(_) => PollCompletion::PollFailure,
            };
            Ok(classify_poll_completion(completion))
        }
        decision => Ok(decision),
    }
}
