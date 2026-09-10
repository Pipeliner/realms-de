use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};

use rustix::fs::{fstat, mkdirat, openat2, FileType, Mode, OFlags, ResolveFlags, Stat, CWD};
use rustix::io::Errno;
use rustix::process::geteuid;

use crate::sys::{
    canonical_socket_path, procfd_socket_path, runtime_procfd_path, socket_addr_un,
    validate_socket_path, ProcFdRuntimeBridge, RuntimeBridge, ScopedUmask, REALM_DIRECTORY,
    SUN_PATH_CAPACITY,
};
use crate::{IpcPathError, SocketEndpoint};

const REQUIRED_DIRECTORY_MODE: u32 = 0o700;
const SECURE_RESOLUTION: ResolveFlags =
    ResolveFlags::NO_SYMLINKS.union(ResolveFlags::NO_MAGICLINKS);
const DIRECTORY_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

/// Resolves the daemon runtime directory into a retained, validated capability.
pub trait RuntimeDirResolver {
    fn resolve(&self) -> Result<RuntimeDir, IpcPathError>;
}

/// A securely resolved runtime directory and the identity it was validated against.
#[allow(dead_code)]
pub struct RuntimeDir {
    path: PathBuf,
    fd: OwnedFd,
    euid: u32,
    device: u64,
    inode: u64,
}

/// A separately opened, validated `realm` directory capability.
#[allow(dead_code)]
pub struct RealmDir {
    fd: OwnedFd,
    euid: u32,
    device: u64,
    inode: u64,
}

/// Resolves the sole production runtime input, `XDG_RUNTIME_DIR`.
pub fn production_runtime_dir() -> Result<RuntimeDir, IpcPathError> {
    let path = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or(IpcPathError::MissingRuntimeDir)?;
    resolve_runtime(&path)
}

/// Resolves an explicit absolute test runtime directory under the production rules.
pub fn test_runtime_dir(path: &Path) -> Result<RuntimeDir, IpcPathError> {
    resolve_runtime(path)
}

impl RuntimeDir {
    /// The canonical display path supplied by the validated runtime input.
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub(crate) fn raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }

    /// Prepares the fixed `realm/ctl.sock` server endpoint.
    pub fn prepare_server_endpoint(self) -> Result<SocketEndpoint, IpcPathError> {
        self.prepare_server_endpoint_with(&ProcFdRuntimeBridge, SUN_PATH_CAPACITY)
    }

    pub(crate) fn prepare_server_endpoint_with<B: RuntimeBridge>(
        self,
        bridge: &B,
        sun_path_capacity: usize,
    ) -> Result<SocketEndpoint, IpcPathError> {
        let bridge_path = runtime_procfd_path(self.fd.as_raw_fd());
        let bridge_fd = bridge.open(&bridge_path).map_err(IpcPathError::from)?;
        let bridge_stat = bridge.stat(bridge_fd.as_fd()).map_err(IpcPathError::from)?;
        self.validate_bridge(&bridge_stat)?;
        drop(bridge_fd);

        let display_path = canonical_socket_path(&self.path);
        validate_socket_path(&display_path, sun_path_capacity)?;
        validate_socket_path(&procfd_socket_path(RawFd::MAX), sun_path_capacity)?;

        {
            let _umask = ScopedUmask::new(Mode::from_raw_mode(0o077));
            match mkdirat(self.fd.as_fd(), REALM_DIRECTORY, Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(IpcPathError::from(error)),
            }
        }

        let realm_dir = self.open_realm_dir()?;
        let bind_path = procfd_socket_path(realm_dir.as_fd().as_raw_fd());
        let bind_address = socket_addr_un(&bind_path)?;
        Ok(SocketEndpoint::new(display_path, realm_dir, bind_address))
    }

    fn validate_bridge(&self, stat: &Stat) -> Result<(), IpcPathError> {
        validate_directory(
            FileType::from_raw_mode(stat.st_mode),
            stat.st_uid,
            Mode::from_raw_mode(stat.st_mode),
            self.euid,
            IpcPathError::UnsafeRuntimeDir,
        )?;
        if stat.st_dev != self.device || stat.st_ino != self.inode {
            return Err(IpcPathError::UnsafeRuntimeDir);
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn open_realm_dir(&self) -> Result<RealmDir, IpcPathError> {
        let fd = openat2(
            self.fd.as_fd(),
            REALM_DIRECTORY,
            DIRECTORY_OPEN_FLAGS,
            Mode::empty(),
            SECURE_RESOLUTION,
        )
        .map_err(map_realm_open_error)?;
        realm_dir_from_fd(fd, self.euid)
    }
}

impl RealmDir {
    /// Borrows the retained descriptor for descriptor-relative realm data operations.
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

fn resolve_runtime(path: &Path) -> Result<RuntimeDir, IpcPathError> {
    resolve_runtime_with(path, |path| {
        openat2(
            CWD,
            path,
            DIRECTORY_OPEN_FLAGS,
            Mode::empty(),
            SECURE_RESOLUTION,
        )
    })
}

pub(crate) fn resolve_runtime_with<F>(path: &Path, opener: F) -> Result<RuntimeDir, IpcPathError>
where
    F: FnOnce(&Path) -> rustix::io::Result<OwnedFd>,
{
    resolve_runtime_from_opener(path, opener)
}

fn resolve_runtime_from_opener<F>(path: &Path, opener: F) -> Result<RuntimeDir, IpcPathError>
where
    F: FnOnce(&Path) -> rustix::io::Result<OwnedFd>,
{
    if !path.is_absolute() {
        return Err(IpcPathError::MissingRuntimeDir);
    }

    let euid = geteuid().as_raw();
    let fd = opener(path).map_err(map_runtime_open_error)?;
    runtime_dir_from_fd(path.to_path_buf(), fd, euid)
}

fn runtime_dir_from_fd(path: PathBuf, fd: OwnedFd, euid: u32) -> Result<RuntimeDir, IpcPathError> {
    let stat = fstat(fd.as_fd()).map_err(IpcPathError::from)?;
    validate_directory(
        FileType::from_raw_mode(stat.st_mode),
        stat.st_uid,
        Mode::from_raw_mode(stat.st_mode),
        euid,
        IpcPathError::UnsafeRuntimeDir,
    )?;
    Ok(RuntimeDir {
        path,
        fd,
        euid,
        device: stat.st_dev,
        inode: stat.st_ino,
    })
}

#[allow(dead_code)]
fn realm_dir_from_fd(fd: OwnedFd, euid: u32) -> Result<RealmDir, IpcPathError> {
    let stat = fstat(fd.as_fd()).map_err(IpcPathError::from)?;
    validate_realm_directory_properties(
        FileType::from_raw_mode(stat.st_mode),
        stat.st_uid,
        Mode::from_raw_mode(stat.st_mode),
        euid,
    )?;
    Ok(RealmDir {
        fd,
        euid,
        device: stat.st_dev,
        inode: stat.st_ino,
    })
}

#[cfg(test)]
pub(crate) fn validate_directory_properties(
    file_type: FileType,
    owner: u32,
    mode: Mode,
) -> Result<(), IpcPathError> {
    validate_directory(
        file_type,
        owner,
        mode,
        geteuid().as_raw(),
        IpcPathError::UnsafeRuntimeDir,
    )
}

#[allow(dead_code)]
fn validate_realm_directory_properties(
    file_type: FileType,
    owner: u32,
    mode: Mode,
    euid: u32,
) -> Result<(), IpcPathError> {
    validate_directory(
        file_type,
        owner,
        mode,
        euid,
        IpcPathError::UnsafeRealmDirectory,
    )
}

fn validate_directory(
    file_type: FileType,
    owner: u32,
    mode: Mode,
    euid: u32,
    unsafe_error: IpcPathError,
) -> Result<(), IpcPathError> {
    if file_type != FileType::Directory || owner != euid || mode.bits() != REQUIRED_DIRECTORY_MODE {
        return Err(unsafe_error);
    }
    Ok(())
}

fn map_runtime_open_error(error: Errno) -> IpcPathError {
    match error {
        Errno::NOENT | Errno::NOTDIR => IpcPathError::MissingRuntimeDir,
        Errno::LOOP | Errno::ACCESS | Errno::PERM => IpcPathError::UnsafeRuntimeDir,
        other => IpcPathError::from(other),
    }
}

#[allow(dead_code)]
fn map_realm_open_error(error: Errno) -> IpcPathError {
    match error {
        Errno::NOENT | Errno::NOTDIR | Errno::LOOP | Errno::ACCESS | Errno::PERM => {
            IpcPathError::UnsafeRealmDirectory
        }
        other => IpcPathError::from(other),
    }
}
