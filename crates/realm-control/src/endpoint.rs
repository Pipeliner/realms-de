use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};

use rustix::event::{poll, PollFd, PollFlags, Timespec};
use rustix::fs::{
    flock, fstat, openat2, statat, unlinkat, AtFlags, FileType, FlockOperation, Mode, OFlags,
    ResolveFlags, Stat,
};
use rustix::io::{fcntl_getfd, Errno, FdFlags};
use rustix::net::sockopt::socket_error;
use rustix::net::{connect, socket_with, AddressFamily, SocketAddrUnix, SocketFlags, SocketType};

use crate::sys::CONTROL_SOCKET;
use crate::{IpcPathError, RealmDir};

const REQUIRED_REALM_MODE: u32 = 0o700;
const REQUIRED_SOCKET_MODE: u32 = 0o600;
const LOCK_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);
const LOCK_RESOLVE_FLAGS: ResolveFlags =
    ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_MAGICLINKS);
const PROBE_SOCKET_FLAGS: SocketFlags = SocketFlags::NONBLOCK.union(SocketFlags::CLOEXEC);
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
#[allow(dead_code)]
pub(crate) struct EndpointLock {
    fd: OwnedFd,
}

/// The exact fixed control-socket descendant and its retained realm capability.
pub struct SocketEndpoint {
    path: PathBuf,
    realm_dir: RealmDir,
    #[allow(dead_code)]
    bind_address: SocketAddrUnix,
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
        self.reclaim_stale_entry(retained_euid, before_recheck)?;
        Ok(endpoint_lock)
    }

    fn reclaim_stale_entry<F>(
        &self,
        retained_euid: u32,
        before_recheck: F,
    ) -> Result<(), IpcPathError>
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
        let initial_identity = validate_socket_stat(&initial_stat, retained_euid)?;

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
        let current_identity = match validate_socket_stat(&current_stat, retained_euid) {
            Ok(identity) => identity,
            Err(_) => return Err(IpcPathError::EndpointInUse),
        };
        if current_identity != initial_identity {
            return Err(IpcPathError::EndpointInUse);
        }

        unlinkat(self.realm_dir.as_fd(), CONTROL_SOCKET, AtFlags::empty())
            .map_err(IpcPathError::from)
    }

    #[cfg(test)]
    pub(crate) fn set_retained_euid_for_test(&mut self, euid: u32) {
        self.realm_dir.set_retained_euid_for_test(euid);
    }

    #[cfg(test)]
    pub(crate) fn validate_socket_stat_for_test(
        &self,
        stat: &Stat,
    ) -> Result<SocketIdentity, IpcPathError> {
        validate_socket_stat(stat, self.realm_dir.retained_euid())
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
            Ok(()) => Ok(Self { fd }),
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
    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        PROBE_SOCKET_FLAGS,
        None,
    )
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
