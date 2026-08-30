//! Immutable generation validation, publication and fail-closed recovery.

#[allow(dead_code)]
mod lifecycle;

#[allow(dead_code)]
mod desktop_exec;

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rustix::fs::{
    flock, fsync, linkat, mkdirat, openat, renameat, renameat_with, statat, unlinkat, AtFlags, Dir,
    FileType, FlockOperation, Mode, OFlags, RenameFlags, CWD,
};
use rustix::io::Errno;

const MAX_LEASE_INVENTORY_ENTRIES: usize = 4096;
const MAX_LEASE_INVENTORY_BYTES: usize = 16 * 1024 * 1024;

/// The held generation root used for every descriptor-relative read.
#[derive(Debug)]
struct GenerationRoot {
    fd: OwnedFd,
}

fn validate_owned_mode(
    name: &str,
    stat: &rustix::fs::Stat,
    expected_type: rustix::fs::FileType,
    expected_mode: u32,
    expected_uid: u32,
) -> std::result::Result<(), String> {
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != expected_type {
        return Err(format!("{name} has an unsafe file type"));
    }
    if stat.st_uid != expected_uid {
        return Err(format!("{name} is not owned by the current user"));
    }
    if rustix::fs::Mode::from_raw_mode(stat.st_mode).bits() != expected_mode {
        return Err(format!("{name} has unsafe permissions"));
    }
    Ok(())
}

fn validate_optional_control(
    root: &GenerationRoot,
    name: &str,
    expected_type: FileType,
    expected_mode: u32,
    expected_uid: u32,
) -> std::result::Result<bool, String> {
    match statat(&root.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            validate_owned_mode(name, &stat, expected_type, expected_mode, expected_uid)?;
            Ok(true)
        }
        Err(Errno::NOENT) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

/// The descriptor-owned root and persistent lock for generation operations.
#[derive(Debug)]
pub struct GenerationStore {
    root: GenerationRoot,
    generations: GenerationRoot,
    leases: GenerationRoot,
    lock: OwnedFd,
    intra_process_lock: Arc<Mutex<()>>,
}

/// Existing generation controls opened without creating or repairing state.
///
/// This capability is intentionally narrower than [`GenerationStore`]: the
/// supported theme diff can take the shared activation lock and compare a
/// validated current generation, but cannot reach any writer, recovery, lease,
/// or garbage-collection operation.
#[derive(Debug)]
pub(crate) struct GenerationReader {
    root: GenerationRoot,
    generations: GenerationRoot,
    lock: OwnedFd,
    intra_process_lock: Mutex<()>,
}

/// How one normalized candidate output differs from the current generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputDifference {
    Added,
    Removed,
    ByteDifferent,
}

/// One launcher selection pinned to a validated generation by an on-disk lease.
#[derive(Debug)]
pub struct GenerationSelection {
    generation: GenerationId,
    root: GenerationRoot,
    path: PathBuf,
    lease_directory: OwnedFd,
    lease_directory_identity: (u64, u64),
    lease_name: String,
    process_identity: LeaseRecord,
    // Private lifecycle transfer state consumed by the following M2 tranche.
    #[allow(dead_code)]
    manifest_seal_digest: String,
    #[allow(dead_code)]
    activation_lock: OwnedFd,
    activation_lock_identity: (u64, u64),
    #[allow(dead_code)]
    intra_process_lock: Arc<Mutex<()>>,
    released: bool,
}

/// The mutations performed by one conservative garbage-collection pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerationGcReport {
    /// Leases whose recorded process identity was provably stale.
    pub reclaimed_leases: usize,
    /// Valid, unleased generations older than the two retained candidates.
    pub reclaimed_generations: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LeaseRecord {
    generation: GenerationId,
    pid: u32,
    start_time: u64,
    boot_id: String,
    owner_uid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedLeaseRecord {
    Process(LeaseRecord),
    Lifecycle(LifecycleLeaseRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LifecycleLeaseRecord {
    generation: GenerationId,
    launch: GenerationId,
    pid: u32,
    start_time: u64,
    boot_id: String,
    owner_uid: u32,
    owner_kind: LifecycleOwnerKind,
    unit: String,
    unit_invocation: String,
    process_group: u32,
    cgroup: String,
    cgroup_device: u64,
    cgroup_inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LifecycleOwnerKind {
    SystemdScope,
    ProcessGroup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferCheckpoint {
    DuringStageWrite,
    BeforeReplace,
    BeforeExchange,
    AfterReplace,
    AfterLeaseDirectoryFsync,
    BeforeAdoptedRecord,
    BeforeSelectionDisarm,
}

trait LeaseFilesystem {
    fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> std::result::Result<(), String>;

    fn link_unnamed_empty_path(
        &mut self,
        source: &OwnedFd,
        directory: &OwnedFd,
        name: &str,
    ) -> std::result::Result<(), Errno> {
        linkat(source, "", directory, name, AtFlags::EMPTY_PATH)
    }

    fn proc_fd_source(&mut self, source: &OwnedFd) -> std::result::Result<PathBuf, String> {
        Ok(format!("/proc/self/fd/{}", source.as_raw_fd()).into())
    }

    fn link_unnamed_proc(
        &mut self,
        source: &Path,
        directory: &OwnedFd,
        name: &str,
    ) -> std::result::Result<(), Errno> {
        linkat(CWD, source, directory, name, AtFlags::SYMLINK_FOLLOW)
    }
}

struct RealLeaseFilesystem;

impl LeaseFilesystem for RealLeaseFilesystem {
    fn checkpoint(&mut self, _checkpoint: TransferCheckpoint) -> std::result::Result<(), String> {
        Ok(())
    }
}

#[derive(Debug)]
struct ValidatedLeaseInventoryEntry {
    record: ParsedLeaseRecord,
    descriptor: OwnedFd,
}

#[derive(Debug)]
struct GcLeaseInventoryPreflight {
    canonical_names: Vec<String>,
    has_transfer_stage: bool,
}

#[derive(Debug, Default)]
struct LeaseTransferRecoveryPlan {
    final_entries: BTreeMap<String, ValidatedLeaseInventoryEntry>,
    staging_entries: BTreeMap<String, ValidatedLeaseInventoryEntry>,
}

impl LeaseTransferRecoveryPlan {
    fn has_staging(&self) -> bool {
        !self.staging_entries.is_empty()
    }

    fn normalize(self, lease_directory: &OwnedFd) -> std::result::Result<(), String> {
        if self.staging_entries.is_empty() {
            return Ok(());
        }
        for (target_name, staging) in &self.staging_entries {
            let staging_name = format!(".lease-transfer-{target_name}");
            let target = self
                .final_entries
                .get(target_name)
                .expect("staging targets were classified");
            if !validated_lease_path_matches(
                lease_directory,
                staging_name.as_str(),
                &staging.descriptor,
            )? || !validated_lease_path_matches(
                lease_directory,
                target_name.as_str(),
                &target.descriptor,
            )? {
                return Err("generation lease transfer pair changed before recovery".into());
            }
        }

        fsync(lease_directory).map_err(|error| error.to_string())?;
        for target_name in self.staging_entries.keys() {
            unlinkat(
                lease_directory,
                format!(".lease-transfer-{target_name}").as_str(),
                AtFlags::empty(),
            )
            .map_err(|error| error.to_string())?;
        }
        fsync(lease_directory).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicationOrder {
    next_sequence: u64,
    sequences: BTreeMap<GenerationId, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseLiveness {
    Live,
    Stale,
    Uncertain,
}

/// One advisory generation-operation lock plus its same-object mutex guard.
#[derive(Debug)]
struct GenerationOperationLock<'operation> {
    lock: &'operation OwnedFd,
    _intra_process: MutexGuard<'operation, ()>,
}

/// A cloned selection lock descriptor held across destructor-safe cleanup.
#[derive(Debug)]
struct SelectionCleanupFileLock(OwnedFd);

#[derive(Debug)]
enum PointerJournalState {
    Clean,
    DiscardCurrent {
        name: String,
    },
    RollbackCurrent {
        name: String,
    },
    RollbackAbsent {
        name: String,
    },
    DiscardPristine {
        current_name: String,
        absent_name: String,
    },
    CleanupCommitted {
        name: String,
        previous: Option<GenerationId>,
    },
}

#[derive(Debug)]
struct PointerJournalInventory {
    current: Option<GenerationId>,
    state: PointerJournalState,
}

#[derive(Debug)]
struct PointerJournalEntry {
    name: String,
    suffix: GenerationId,
    content: Option<GenerationId>,
}

/// Captured inputs and rendered outputs for one immutable generation.
#[derive(Debug)]
pub struct GenerationPublication {
    input_digests: [String; 5],
    outputs: Vec<(String, Vec<u8>)>,
}

/// Publication failures distinguish a definite non-commit from a durable
/// pointer whose rollback-marker cleanup could not be made durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationPublicationError {
    /// The operation restored the prior selection, or never changed it.
    NotCommitted(String),
}

/// A non-error publication result, including explicit filesystem uncertainty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationPublicationOutcome {
    /// The pointer and journal cleanup are durable.
    Committed(GenerationId),
    /// The pointer is durable and a committed journal remains recoverable.
    CommittedWithCleanupPending {
        /// The durably selected generation.
        generation: GenerationId,
        /// Why committed-journal cleanup remains pending.
        cause: String,
    },
    /// Commit and rollback could not be distinguished after filesystem errors.
    OutcomeAmbiguous {
        /// The candidate which may be selected.
        candidate: GenerationId,
        /// The filesystem failures preventing a definite outcome.
        cause: String,
    },
}

impl GenerationPublicationOutcome {
    /// Return the candidate generation associated with this outcome.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Committed(generation) | Self::CommittedWithCleanupPending { generation, .. } => {
                generation.as_str()
            }
            Self::OutcomeAmbiguous { candidate, .. } => candidate.as_str(),
        }
    }
}

impl From<String> for GenerationPublicationError {
    fn from(error: String) -> Self {
        Self::NotCommitted(error)
    }
}

impl From<&str> for GenerationPublicationError {
    fn from(error: &str) -> Self {
        Self::NotCommitted(error.into())
    }
}

impl std::fmt::Display for GenerationPublicationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for GenerationPublicationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommitCheckpoint {
    InputsCaptured,
    PublicationSequenceReserved,
    OutputsSynced,
    ReceiptSynced,
    ManifestSynced,
    SealSynced,
    StagingDirectorySynced,
    PublicationOrderSynced,
    TreeCommitted,
    GenerationsDirectorySynced,
    PointerFileSynced,
    PointerCommitted,
    RootDirectorySynced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryCheckpoint {
    PristineCurrentJournaled,
    PristineAbsentRemoved,
}

trait PublicationFilesystem {
    fn sync(&mut self, fd: BorrowedFd<'_>) -> std::result::Result<(), String>;

    fn rename(
        &mut self,
        from_directory: BorrowedFd<'_>,
        from_name: &str,
        to_directory: BorrowedFd<'_>,
        to_name: &str,
    ) -> std::result::Result<(), String>;

    fn exchange(
        &mut self,
        left_directory: BorrowedFd<'_>,
        left_name: &str,
        right_directory: BorrowedFd<'_>,
        right_name: &str,
    ) -> std::result::Result<(), String>;

    fn unlink(&mut self, directory: BorrowedFd<'_>, name: &str) -> std::result::Result<(), String>;
}

struct RealPublicationFilesystem;

impl PublicationFilesystem for RealPublicationFilesystem {
    fn sync(&mut self, fd: BorrowedFd<'_>) -> std::result::Result<(), String> {
        fsync(fd).map_err(|error| error.to_string())
    }

    fn rename(
        &mut self,
        from_directory: BorrowedFd<'_>,
        from_name: &str,
        to_directory: BorrowedFd<'_>,
        to_name: &str,
    ) -> std::result::Result<(), String> {
        renameat(from_directory, from_name, to_directory, to_name)
            .map_err(|error| error.to_string())
    }

    fn exchange(
        &mut self,
        left_directory: BorrowedFd<'_>,
        left_name: &str,
        right_directory: BorrowedFd<'_>,
        right_name: &str,
    ) -> std::result::Result<(), String> {
        renameat_with(
            left_directory,
            left_name,
            right_directory,
            right_name,
            RenameFlags::EXCHANGE,
        )
        .map_err(|error| error.to_string())
    }

    fn unlink(&mut self, directory: BorrowedFd<'_>, name: &str) -> std::result::Result<(), String> {
        unlinkat(directory, name, AtFlags::empty()).map_err(|error| error.to_string())
    }
}

impl GenerationPublication {
    /// Validate captured generation inputs before touching the generation tree.
    pub fn new(
        input_digests: [String; 5],
        mut outputs: Vec<(String, Vec<u8>)>,
    ) -> std::result::Result<Self, String> {
        if input_digests.iter().any(|digest| !sha256_digest(digest)) {
            return Err("generation input digest must be lowercase SHA-256 hex".into());
        }
        outputs.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
        if outputs.is_empty() {
            return Err("generation publication requires an output".into());
        }
        let output_paths: BTreeSet<&str> = outputs.iter().map(|(path, _)| path.as_str()).collect();
        if output_paths.len() != outputs.len() {
            return Err("generation output paths collide".into());
        }
        for (path, _) in &outputs {
            if !safe_output_path(path) {
                return Err("unsafe generation output path".into());
            }
            let mut prefix = String::new();
            let mut components = path.split('/').peekable();
            while let Some(component) = components.next() {
                if components.peek().is_none() {
                    break;
                }
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                if output_paths.contains(prefix.as_str()) {
                    return Err("generation output paths collide".into());
                }
            }
        }
        Ok(Self {
            input_digests,
            outputs,
        })
    }

    pub(crate) fn outputs(&self) -> &[(String, Vec<u8>)] {
        &self.outputs
    }
}

impl GenerationReader {
    /// Open every pre-existing control needed by diff without initializing any
    /// missing inode or parsing publication order, which selection ignores.
    pub(crate) fn open_from_fd(fd: OwnedFd) -> std::result::Result<Self, String> {
        let root = GenerationRoot { fd };
        let current_uid = rustix::process::getuid().as_raw();
        let root_stat = rustix::fs::fstat(&root.fd).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "generated root",
            &root_stat,
            FileType::Directory,
            0o700,
            current_uid,
        )?;

        let lock = openat(
            &root.fd,
            "activation.lock",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| format!("activation lock: {error}"))?;
        let lock_stat = rustix::fs::fstat(&lock).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "activation lock",
            &lock_stat,
            FileType::RegularFile,
            0o600,
            current_uid,
        )?;

        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let generations = GenerationRoot {
            fd: openat(&root.fd, "generations", directory_flags, Mode::empty())
                .map_err(|error| format!("generations directory: {error}"))?,
        };
        let generations_stat =
            rustix::fs::fstat(&generations.fd).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "generations directory",
            &generations_stat,
            FileType::Directory,
            0o700,
            current_uid,
        )?;

        Ok(Self {
            root,
            generations,
            lock,
            intra_process_lock: Mutex::new(()),
        })
    }

    fn lock_shared(&self) -> std::result::Result<GenerationOperationLock<'_>, String> {
        let intra_process = self
            .intra_process_lock
            .lock()
            .map_err(|_| "generation reader lock is poisoned")?;
        flock(&self.lock, FlockOperation::LockShared).map_err(|error| error.to_string())?;
        Ok(GenerationOperationLock {
            lock: &self.lock,
            _intra_process: intra_process,
        })
    }

    /// Fully validate current and compare exact output bytes while the shared
    /// activation lock remains held.
    pub(crate) fn diff_current(
        &self,
        candidate: &[(String, Vec<u8>)],
    ) -> std::result::Result<Vec<(String, OutputDifference)>, String> {
        self.diff_current_inner(candidate, || {}, |_| {}, || {})
    }

    #[cfg(test)]
    fn diff_current_with_interval_checkpoints<L, R, C>(
        &self,
        candidate: &[(String, Vec<u8>)],
        lock_acquired: L,
        before_output_read: R,
        result_constructed: C,
    ) -> std::result::Result<Vec<(String, OutputDifference)>, String>
    where
        L: FnOnce(),
        R: FnMut(&str),
        C: FnOnce(),
    {
        self.diff_current_inner(
            candidate,
            lock_acquired,
            before_output_read,
            result_constructed,
        )
    }

    fn diff_current_inner<L, R, C>(
        &self,
        candidate: &[(String, Vec<u8>)],
        lock_acquired: L,
        mut before_output_read: R,
        result_constructed: C,
    ) -> std::result::Result<Vec<(String, OutputDifference)>, String>
    where
        L: FnOnce(),
        R: FnMut(&str),
        C: FnOnce(),
    {
        let _lock = self.lock_shared()?;
        lock_acquired();
        let inventory =
            GenerationStore::inspect_pointer_journals_from(&self.root, &self.generations)?;
        let generation = match inventory.state {
            PointerJournalState::Clean | PointerJournalState::CleanupCommitted { .. } => inventory
                .current
                .ok_or_else(|| "current generation is absent".to_owned()),
            _ => Err("pointer transaction requires exclusive recovery".into()),
        }?;
        let (current_root, manifest) =
            GenerationStore::open_validated_generation_with_manifest_from(
                &self.generations,
                &generation,
            )?;

        let candidate: BTreeMap<&str, &[u8]> = candidate
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect();
        let current: BTreeMap<&str, Vec<u8>> = manifest
            .entries
            .iter()
            .map(|(path, _)| {
                before_output_read(path);
                let bytes = read_regular_bytes(&current_root, Path::new(path))?;
                Ok::<_, String>((path.as_str(), bytes))
            })
            .collect::<std::result::Result<_, _>>()?;
        let paths: BTreeSet<&str> = candidate.keys().chain(current.keys()).copied().collect();
        let mut changes = Vec::new();
        for path in paths {
            let difference = match (candidate.get(path), current.get(path)) {
                (Some(_), None) => Some(OutputDifference::Added),
                (None, Some(_)) => Some(OutputDifference::Removed),
                (Some(candidate), Some(current)) if *candidate != current.as_slice() => {
                    Some(OutputDifference::ByteDifferent)
                }
                (Some(_), Some(_)) => None,
                (None, None) => unreachable!("path came from the map union"),
            };
            if let Some(difference) = difference {
                changes.push((path.to_owned(), difference));
            }
        }
        result_constructed();
        Ok(changes)
    }
}

impl GenerationStore {
    /// Open an existing generated root and create its one persistent lock inode.
    pub fn open(path: &Path) -> std::result::Result<Self, String> {
        Self::open_with_root_and_preflight(GenerationRoot::open(path)?, || {})
    }

    /// Open a generated root already reached through a descriptor-safe parent
    /// chain. The descriptor must name the generated root itself.
    pub fn open_from_fd(fd: OwnedFd) -> std::result::Result<Self, String> {
        Self::open_with_root_and_preflight(GenerationRoot { fd }, || {})
    }

    #[cfg(test)]
    fn open_with_preflight_checkpoint<H>(
        path: &Path,
        preflight_checkpoint: H,
    ) -> std::result::Result<Self, String>
    where
        H: FnOnce(),
    {
        let root = GenerationRoot::open(path)?;
        Self::open_with_root_and_preflight(root, preflight_checkpoint)
    }

    fn open_with_root_and_preflight<H>(
        root: GenerationRoot,
        preflight_checkpoint: H,
    ) -> std::result::Result<Self, String>
    where
        H: FnOnce(),
    {
        let current_uid = rustix::process::getuid().as_raw();
        let root_stat = rustix::fs::fstat(&root.fd).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "generated root",
            &root_stat,
            FileType::Directory,
            0o700,
            current_uid,
        )?;

        let lock_exists = match statat(&root.fd, "activation.lock", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                validate_owned_mode(
                    "activation lock",
                    &stat,
                    FileType::RegularFile,
                    0o600,
                    current_uid,
                )?;
                true
            }
            Err(Errno::NOENT) => false,
            Err(error) => return Err(error.to_string()),
        };
        let generations_exists = match statat(&root.fd, "generations", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                validate_owned_mode(
                    "generations directory",
                    &stat,
                    FileType::Directory,
                    0o700,
                    current_uid,
                )?;
                true
            }
            Err(Errno::NOENT) => false,
            Err(error) => return Err(error.to_string()),
        };
        let leases_exists = match statat(&root.fd, "leases", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                validate_owned_mode(
                    "leases directory",
                    &stat,
                    FileType::Directory,
                    0o700,
                    current_uid,
                )?;
                true
            }
            Err(Errno::NOENT) => false,
            Err(error) => return Err(error.to_string()),
        };
        preflight_checkpoint();

        let lock_flags = OFlags::RDWR
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | if lock_exists {
                OFlags::empty()
            } else {
                OFlags::CREATE | OFlags::EXCL
            };
        let lock = match openat(
            &root.fd,
            "activation.lock",
            lock_flags,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(lock) => lock,
            Err(Errno::EXIST) if !lock_exists => openat(
                &root.fd,
                "activation.lock",
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?,
            Err(error) => return Err(error.to_string()),
        };
        let lock_stat = rustix::fs::fstat(&lock).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "activation lock",
            &lock_stat,
            FileType::RegularFile,
            0o600,
            current_uid,
        )?;
        if !generations_exists {
            match mkdirat(
                &root.fd,
                "generations",
                Mode::RUSR | Mode::WUSR | Mode::XUSR,
            ) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        if !leases_exists {
            match mkdirat(&root.fd, "leases", Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let generations = GenerationRoot {
            fd: openat(
                &root.fd,
                "generations",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?,
        };
        let generations_stat =
            rustix::fs::fstat(&generations.fd).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "generations directory",
            &generations_stat,
            FileType::Directory,
            0o700,
            current_uid,
        )?;
        let leases = GenerationRoot {
            fd: openat(
                &root.fd,
                "leases",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?,
        };
        let leases_stat = rustix::fs::fstat(&leases.fd).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "leases directory",
            &leases_stat,
            FileType::Directory,
            0o700,
            current_uid,
        )?;
        flock(&lock, FlockOperation::LockExclusive).map_err(|error| error.to_string())?;
        let initialize_order = (|| -> std::result::Result<(), String> {
            match statat(&root.fd, "publication-order", AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => return Ok(()),
                Err(Errno::NOENT) => {}
                Err(error) => return Err(error.to_string()),
            }
            let fd = match openat(
                &root.fd,
                "publication-order",
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => fd,
                Err(Errno::EXIST) => return Ok(()),
                Err(error) => return Err(error.to_string()),
            };
            let mut file = std::fs::File::from(fd);
            file.write_all(&PublicationOrder::empty().encode())
                .map_err(|error| error.to_string())?;
            fsync(&file).map_err(|error| error.to_string())?;
            fsync(&root.fd).map_err(|error| error.to_string())?;
            Ok(())
        })();
        let unlock = flock(&lock, FlockOperation::Unlock).map_err(|error| error.to_string());
        initialize_order?;
        unlock?;
        if !lock_exists || !generations_exists || !leases_exists {
            fsync(&root.fd).map_err(|error| error.to_string())?;
        }
        Ok(Self {
            root,
            generations,
            leases,
            lock,
            intra_process_lock: Arc::new(Mutex::new(())),
        })
    }

    fn lock_exclusive(&self) -> std::result::Result<GenerationOperationLock<'_>, String> {
        let intra_process = self
            .intra_process_lock
            .lock()
            .map_err(|_| "generation store lock is poisoned")?;
        flock(&self.lock, FlockOperation::LockExclusive).map_err(|error| error.to_string())?;
        Ok(GenerationOperationLock {
            lock: &self.lock,
            _intra_process: intra_process,
        })
    }

    fn lock_shared(&self) -> std::result::Result<GenerationOperationLock<'_>, String> {
        let intra_process = self
            .intra_process_lock
            .lock()
            .map_err(|_| "generation store lock is poisoned")?;
        flock(&self.lock, FlockOperation::LockShared).map_err(|error| error.to_string())?;
        Ok(GenerationOperationLock {
            lock: &self.lock,
            _intra_process: intra_process,
        })
    }

    /// Resolve and validate current while holding the persistent shared lock.
    pub fn select_current(&self) -> std::result::Result<GenerationSelection, String> {
        self.select_current_with_checkpoint(std::process::id(), || {})
    }

    /// Resolve current and lease it to an already-created, not-yet-executing target.
    pub fn select_current_for_process(
        &self,
        pid: u32,
    ) -> std::result::Result<GenerationSelection, String> {
        self.select_current_with_checkpoint(pid, || {})
    }

    fn select_current_with_checkpoint<H>(
        &self,
        pid: u32,
        lease_synced: H,
    ) -> std::result::Result<GenerationSelection, String>
    where
        H: FnOnce(),
    {
        let _lock = self.lock_shared()?;
        let inventory = self.inspect_pointer_journals()?;
        let generation = match inventory.state {
            PointerJournalState::Clean | PointerJournalState::CleanupCommitted { .. } => inventory
                .current
                .ok_or_else(|| "current generation is absent".to_owned()),
            _ => Err("pointer transaction requires exclusive recovery".into()),
        }?;
        let (generation_root, manifest) =
            self.open_validated_generation_with_manifest(&generation)?;
        let path = std::fs::read_link(format!("/proc/self/fd/{}", generation_root.fd.as_raw_fd()))
            .map_err(|error| error.to_string())?;
        let identity = LeaseRecord::for_process(generation.clone(), pid)?;
        let lease_directory = self
            .leases
            .fd
            .try_clone()
            .map_err(|error| error.to_string())?;
        let activation_lock = self.lock.try_clone().map_err(|error| error.to_string())?;
        let lease_directory_stat =
            rustix::fs::fstat(&lease_directory).map_err(|error| error.to_string())?;
        let activation_lock_stat =
            rustix::fs::fstat(&activation_lock).map_err(|error| error.to_string())?;
        let lease_name = self.create_lease_locked(&identity)?;
        lease_synced();
        Ok(GenerationSelection {
            generation,
            root: generation_root,
            path,
            lease_directory,
            lease_directory_identity: (lease_directory_stat.st_dev, lease_directory_stat.st_ino),
            lease_name,
            process_identity: identity,
            manifest_seal_digest: digest_hex(manifest.v1.raw.as_bytes()),
            activation_lock,
            activation_lock_identity: (activation_lock_stat.st_dev, activation_lock_stat.st_ino),
            intra_process_lock: Arc::clone(&self.intra_process_lock),
            released: false,
        })
    }

    /// Publish one captured generation while holding the store's exclusive lock.
    pub fn publish<C>(
        &self,
        capture: C,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        C: FnOnce() -> std::result::Result<GenerationPublication, String>,
    {
        self.publish_with_checkpoint_and_ids(capture, |_| Ok(()), random_generation_id)
    }

    fn publish_with_checkpoint_and_ids<C, H, I>(
        &self,
        capture: C,
        checkpoint: H,
        next_id: I,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        C: FnOnce() -> std::result::Result<GenerationPublication, String>,
        H: FnMut(CommitCheckpoint) -> std::result::Result<(), String>,
        I: FnMut() -> std::result::Result<GenerationId, String>,
    {
        let mut filesystem = RealPublicationFilesystem;
        self.publish_with_checkpoint_ids_and_filesystem(
            capture,
            checkpoint,
            next_id,
            &mut filesystem,
        )
    }

    fn publish_with_checkpoint_ids_and_filesystem<C, H, I, F>(
        &self,
        capture: C,
        checkpoint: H,
        mut next_id: I,
        filesystem: &mut F,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        C: FnOnce() -> std::result::Result<GenerationPublication, String>,
        H: FnMut(CommitCheckpoint) -> std::result::Result<(), String>,
        I: FnMut() -> std::result::Result<GenerationId, String>,
        F: PublicationFilesystem,
    {
        let _lock = self.lock_exclusive()?;
        let pointer_inventory = self.inspect_pointer_journals()?;
        self.clean_staging()?;
        self.clean_pointer_staging(pointer_inventory, |_| Ok(()), filesystem)?;
        self.clean_publication_order_staging(filesystem)?;
        let had_current = self.validate_current_or_pristine()?;
        let publication_order = self.load_publication_order()?;
        self.validate_publication_order_complete(&publication_order)?;
        let publication = capture()?;
        self.publish_allocated_locked(
            publication,
            had_current,
            publication_order,
            checkpoint,
            &mut next_id,
            filesystem,
        )
    }

    /// Recover interrupted staging and return the only validated active generation.
    pub fn recover(&self) -> std::result::Result<GenerationId, String> {
        let mut filesystem = RealPublicationFilesystem;
        self.recover_with_checkpoint_and_filesystem(|_| Ok(()), &mut filesystem)
    }

    fn recover_with_checkpoint_and_filesystem<H, F>(
        &self,
        checkpoint: H,
        filesystem: &mut F,
    ) -> std::result::Result<GenerationId, String>
    where
        H: FnMut(RecoveryCheckpoint) -> std::result::Result<(), String>,
        F: PublicationFilesystem,
    {
        let _lock = self.lock_exclusive()?;
        let pointer_inventory = self.inspect_pointer_journals()?;
        self.clean_staging()?;
        self.clean_pointer_staging(pointer_inventory, checkpoint, filesystem)?;
        self.resolve_current()
    }

    /// Validate a retained generation and durably select it for future launches.
    pub fn rollback(
        &self,
        generation: &GenerationId,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError> {
        let _lock = self.lock_exclusive()?;
        let mut filesystem = RealPublicationFilesystem;
        let inventory = self.inspect_pointer_journals()?;
        self.validate_generation(generation)?;
        self.clean_pointer_staging(inventory, |_| Ok(()), &mut filesystem)?;
        let current = self.resolve_current()?;
        if &current == generation {
            return Ok(GenerationPublicationOutcome::Committed(generation.clone()));
        }
        self.commit_existing_pointer_locked(generation.clone(), &mut filesystem)
    }

    /// Remove only provably stale leases and old valid unleased generations.
    pub fn garbage_collect(&self) -> std::result::Result<GenerationGcReport, String> {
        let _lock = self.lock_exclusive()?;
        let (mut report, mut live_generations, uncertain_lease) =
            self.reclaim_stale_leases_locked()?;
        if uncertain_lease {
            return Ok(report);
        }
        let inventory = match self.inspect_pointer_journals() {
            Ok(inventory) => inventory,
            Err(_) => return Ok(report),
        };
        let journal_previous = match inventory.state {
            PointerJournalState::Clean => None,
            PointerJournalState::CleanupCommitted { previous, .. } => previous,
            _ => return Ok(report),
        };
        let Some(current) = inventory.current else {
            return Ok(report);
        };

        live_generations.insert(current.clone());
        if let Some(previous) = journal_previous {
            live_generations.insert(previous);
        }

        let current_uid = rustix::process::getuid().as_raw();
        match validate_optional_control(
            &self.root,
            ".publication-order",
            FileType::RegularFile,
            0o600,
            current_uid,
        ) {
            Ok(false) => {}
            Ok(true) | Err(_) => return Ok(report),
        }
        let publication_order = match self.load_publication_order() {
            Ok(order) => order,
            Err(_) => return Ok(report),
        };
        if self
            .validate_publication_order_complete(&publication_order)
            .is_err()
        {
            return Ok(report);
        }

        let mut candidates = Vec::new();
        let mut malformed_generation = false;
        let mut generations =
            Dir::read_from(&self.generations.fd).map_err(|error| error.to_string())?;
        while let Some(entry) = generations.read() {
            let entry = entry.map_err(|error| error.to_string())?;
            let bytes = entry.file_name().to_bytes();
            if matches!(bytes, b"." | b"..") {
                continue;
            }
            let Ok(name) = std::str::from_utf8(bytes) else {
                malformed_generation = true;
                continue;
            };
            let Ok(generation) = GenerationId::parse(name) else {
                malformed_generation = true;
                continue;
            };
            let Ok(stat) = statat(&self.generations.fd, name, AtFlags::SYMLINK_NOFOLLOW) else {
                malformed_generation = true;
                continue;
            };
            if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
                || self.validate_generation(&generation).is_err()
            {
                malformed_generation = true;
                continue;
            }
            if live_generations.contains(&generation) {
                continue;
            }
            let Some(sequence) = publication_order.sequences.get(&generation).copied() else {
                malformed_generation = true;
                continue;
            };
            candidates.push((sequence, generation));
        }
        drop(generations);
        if malformed_generation {
            return Ok(report);
        }
        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
        for (_, generation) in candidates.into_iter().skip(2) {
            remove_tree_at(&self.generations.fd, OsStr::new(generation.as_str()))?;
            fsync(&self.generations.fd).map_err(|error| error.to_string())?;
            report.reclaimed_generations += 1;
        }
        Ok(report)
    }

    fn reclaim_stale_leases_locked(
        &self,
    ) -> std::result::Result<(GenerationGcReport, BTreeSet<GenerationId>, bool), String> {
        let mut report = GenerationGcReport::default();
        let mut live_generations = BTreeSet::new();
        let mut uncertain_lease = false;
        let mut stale_leases = Vec::new();
        let preflight = match gc_lease_inventory_preflight(&self.leases.fd) {
            Ok(preflight) => preflight,
            Err(_) => return Ok((report, live_generations, true)),
        };
        if preflight.has_transfer_stage {
            return Ok((report, live_generations, true));
        }
        for name in preflight.canonical_names {
            let Ok(validated) = read_validated_lease_inventory_entry(&self.leases.fd, &name) else {
                uncertain_lease = true;
                continue;
            };
            match validated.record {
                ParsedLeaseRecord::Process(record) => match record.liveness() {
                    LeaseLiveness::Live => {
                        live_generations.insert(record.generation);
                    }
                    LeaseLiveness::Stale => stale_leases.push(name.to_owned()),
                    LeaseLiveness::Uncertain => {
                        uncertain_lease = true;
                        live_generations.insert(record.generation);
                    }
                },
                ParsedLeaseRecord::Lifecycle(record) => {
                    uncertain_lease = true;
                    live_generations.insert(record.generation);
                }
            }
        }
        if uncertain_lease {
            return Ok((report, live_generations, true));
        }
        for lease in stale_leases {
            match unlinkat(&self.leases.fd, lease.as_str(), AtFlags::empty()) {
                Ok(()) => {
                    fsync(&self.leases.fd).map_err(|error| error.to_string())?;
                    report.reclaimed_leases += 1;
                }
                Err(Errno::NOENT) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok((report, live_generations, uncertain_lease))
    }

    fn load_publication_order(&self) -> std::result::Result<PublicationOrder, String> {
        let current_uid = rustix::process::getuid().as_raw();
        if !validate_optional_control(
            &self.root,
            "publication-order",
            FileType::RegularFile,
            0o600,
            current_uid,
        )? {
            return Err("publication order is absent".into());
        }
        let raw = read_optional_regular_bytes(&self.root, Path::new("publication-order"))?
            .ok_or("publication order disappeared while reading")?;
        PublicationOrder::parse(&raw)
    }

    fn validate_publication_order_complete(
        &self,
        order: &PublicationOrder,
    ) -> std::result::Result<(), String> {
        let mut directory =
            Dir::read_from(&self.generations.fd).map_err(|error| error.to_string())?;
        while let Some(entry) = directory.read() {
            let entry = entry.map_err(|error| error.to_string())?;
            let bytes = entry.file_name().to_bytes();
            if matches!(bytes, b"." | b"..") {
                continue;
            }
            let name = std::str::from_utf8(bytes)
                .map_err(|_| "generation name is not UTF-8 while validating publication order")?;
            let generation = GenerationId::parse(name)?;
            let stat = statat(&self.generations.fd, name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
                return Err("final generation is not a directory".into());
            }
            if !order.sequences.contains_key(&generation) {
                return Err("final generation lacks a publication sequence".into());
            }
        }
        Ok(())
    }

    fn clean_publication_order_staging<F: PublicationFilesystem>(
        &self,
        filesystem: &mut F,
    ) -> std::result::Result<(), String> {
        let current_uid = rustix::process::getuid().as_raw();
        if !validate_optional_control(
            &self.root,
            ".publication-order",
            FileType::RegularFile,
            0o600,
            current_uid,
        )? {
            return Ok(());
        }
        filesystem.unlink(self.root.fd.as_fd(), ".publication-order")?;
        filesystem.sync(self.root.fd.as_fd())
    }

    fn replace_publication_order<F: PublicationFilesystem>(
        &self,
        order: &PublicationOrder,
        filesystem: &mut F,
    ) -> std::result::Result<(), String> {
        write_synced_file(
            &self.root.fd,
            ".publication-order",
            &order.encode(),
            filesystem,
        )?;
        filesystem.rename(
            self.root.fd.as_fd(),
            ".publication-order",
            self.root.fd.as_fd(),
            "publication-order",
        )?;
        filesystem.sync(self.root.fd.as_fd())
    }

    fn validate_current_or_pristine(&self) -> std::result::Result<bool, String> {
        match self.resolve_current() {
            Ok(_) => Ok(true),
            Err(error) if error == "current generation is absent" => {
                let mut directory = Dir::read_from(&self.generations.fd)
                    .map_err(|read_error| read_error.to_string())?;
                while let Some(entry) = directory.read() {
                    let entry = entry.map_err(|read_error| read_error.to_string())?;
                    let bytes = entry.file_name().to_bytes();
                    if matches!(bytes, b"." | b"..") {
                        continue;
                    }
                    let name = std::str::from_utf8(bytes)
                        .map_err(|_| "unpointed generation name is not UTF-8")?;
                    let generation = GenerationId::parse(name)?;
                    self.validate_generation(&generation)?;
                }
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    fn publish_allocated_locked<H, I, F>(
        &self,
        publication: GenerationPublication,
        had_current: bool,
        mut publication_order: PublicationOrder,
        mut checkpoint: H,
        next_id: &mut I,
        filesystem: &mut F,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        H: FnMut(CommitCheckpoint) -> std::result::Result<(), String>,
        I: FnMut() -> std::result::Result<GenerationId, String>,
        F: PublicationFilesystem,
    {
        checkpoint(CommitCheckpoint::InputsCaptured)?;
        let publication_sequence = publication_order.reserve()?;
        self.replace_publication_order(&publication_order, filesystem)?;
        checkpoint(CommitCheckpoint::PublicationSequenceReserved)?;
        let mut generation = None;
        for _ in 0..64 {
            let candidate = next_id()?;
            if publication_order.sequences.contains_key(&candidate) {
                continue;
            }
            match statat(
                &self.generations.fd,
                candidate.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Ok(_) => continue,
                Err(Errno::NOENT) => {
                    generation = Some(candidate);
                    break;
                }
                Err(error) => return Err(error.to_string().into()),
            }
        }
        let generation = generation.ok_or("generation id collision retry limit exhausted")?;
        if publication_order
            .sequences
            .insert(generation.clone(), publication_sequence)
            .is_some()
        {
            return Err("generation already has a publication sequence".into());
        }
        self.publish_generation_locked(
            publication,
            generation,
            publication_order,
            had_current,
            checkpoint,
            filesystem,
        )
    }

    fn publish_generation_locked<H, F>(
        &self,
        publication: GenerationPublication,
        generation: GenerationId,
        publication_order: PublicationOrder,
        had_current: bool,
        mut checkpoint: H,
        filesystem: &mut F,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        H: FnMut(CommitCheckpoint) -> std::result::Result<(), String>,
        F: PublicationFilesystem,
    {
        let generation_name = generation.as_str();
        let staging_name = format!(".staging-{generation_name}");
        mkdirat(
            &self.generations.fd,
            staging_name.as_str(),
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )
        .map_err(|error| error.to_string())?;
        let staging = GenerationRoot {
            fd: openat(
                &self.generations.fd,
                staging_name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?,
        };

        for (path, bytes) in &publication.outputs {
            write_output(&staging, Path::new(path), bytes, filesystem)?;
        }
        checkpoint(CommitCheckpoint::OutputsSynced)?;

        let receipt = publication.receipt(&generation);
        write_synced_file(&staging.fd, "receipt", receipt.as_bytes(), filesystem)?;
        checkpoint(CommitCheckpoint::ReceiptSynced)?;
        let manifest = publication.manifest(&generation, &receipt);
        write_synced_file(&staging.fd, "manifest", manifest.as_bytes(), filesystem)?;
        checkpoint(CommitCheckpoint::ManifestSynced)?;
        let seal = format!("{}\n", digest_hex(manifest.as_bytes()));
        write_synced_file(&staging.fd, "seal", seal.as_bytes(), filesystem)?;
        checkpoint(CommitCheckpoint::SealSynced)?;
        filesystem.sync(staging.fd.as_fd())?;
        checkpoint(CommitCheckpoint::StagingDirectorySynced)?;

        self.replace_publication_order(&publication_order, filesystem)?;
        checkpoint(CommitCheckpoint::PublicationOrderSynced)?;

        filesystem.rename(
            self.generations.fd.as_fd(),
            staging_name.as_str(),
            self.generations.fd.as_fd(),
            generation_name,
        )?;
        checkpoint(CommitCheckpoint::TreeCommitted)?;
        filesystem.sync(self.generations.fd.as_fd())?;
        checkpoint(CommitCheckpoint::GenerationsDirectorySynced)?;

        let pointer_staging = format!(".current-{generation_name}");
        write_synced_file(
            &self.root.fd,
            pointer_staging.as_str(),
            format!("{generation_name}\n").as_bytes(),
            filesystem,
        )?;
        checkpoint(CommitCheckpoint::PointerFileSynced)?;
        let absent_marker = format!(".absent-{generation_name}");
        if had_current {
            filesystem.sync(self.root.fd.as_fd())?;
        } else {
            write_synced_file(&self.root.fd, absent_marker.as_str(), b"", filesystem)?;
            filesystem.sync(self.root.fd.as_fd())?;
        }
        if had_current {
            filesystem.exchange(
                self.root.fd.as_fd(),
                pointer_staging.as_str(),
                self.root.fd.as_fd(),
                "current",
            )?;
        } else {
            filesystem.rename(
                self.root.fd.as_fd(),
                pointer_staging.as_str(),
                self.root.fd.as_fd(),
                "current",
            )?;
        }
        checkpoint(CommitCheckpoint::PointerCommitted)?;
        if let Err(commit_error) = filesystem.sync(self.root.fd.as_fd()) {
            let rollback = if had_current {
                filesystem
                    .exchange(
                        self.root.fd.as_fd(),
                        pointer_staging.as_str(),
                        self.root.fd.as_fd(),
                        "current",
                    )
                    .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
            } else {
                filesystem
                    .rename(
                        self.root.fd.as_fd(),
                        "current",
                        self.root.fd.as_fd(),
                        pointer_staging.as_str(),
                    )
                    .and_then(|()| {
                        unlinkat(&self.root.fd, absent_marker.as_str(), AtFlags::empty())
                            .map_err(|error| error.to_string())
                    })
                    .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
            };
            return match rollback {
                Ok(()) => Err(commit_error.into()),
                Err(rollback_error) => Ok(GenerationPublicationOutcome::OutcomeAmbiguous {
                    candidate: generation,
                    cause: format!(
                        "pointer sync failed ({commit_error}) and rollback failed ({rollback_error})"
                    ),
                }),
            };
        }
        let cleanup_name = if had_current {
            pointer_staging.as_str()
        } else {
            absent_marker.as_str()
        };
        let committed_marker = format!(".committed-{generation_name}");
        if let Err(cause) = filesystem
            .rename(
                self.root.fd.as_fd(),
                cleanup_name,
                self.root.fd.as_fd(),
                committed_marker.as_str(),
            )
            .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
        {
            return Ok(GenerationPublicationOutcome::OutcomeAmbiguous {
                candidate: generation,
                cause: format!(
                    "commit-marker transition failed after pointer update; outcome is unconfirmed: {cause}"
                ),
            });
        }
        if let Err(cause) = filesystem
            .unlink(self.root.fd.as_fd(), committed_marker.as_str())
            .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
        {
            return Ok(GenerationPublicationOutcome::CommittedWithCleanupPending {
                generation,
                cause,
            });
        }
        checkpoint(CommitCheckpoint::RootDirectorySynced)?;
        Ok(GenerationPublicationOutcome::Committed(generation))
    }

    fn commit_existing_pointer_locked<F: PublicationFilesystem>(
        &self,
        generation: GenerationId,
        filesystem: &mut F,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError> {
        let generation_name = generation.as_str();
        let pointer_staging = format!(".current-{generation_name}");
        write_synced_file(
            &self.root.fd,
            pointer_staging.as_str(),
            format!("{generation_name}\n").as_bytes(),
            filesystem,
        )?;
        filesystem.sync(self.root.fd.as_fd())?;
        filesystem.exchange(
            self.root.fd.as_fd(),
            pointer_staging.as_str(),
            self.root.fd.as_fd(),
            "current",
        )?;
        if let Err(commit_error) = filesystem.sync(self.root.fd.as_fd()) {
            let rollback = filesystem
                .exchange(
                    self.root.fd.as_fd(),
                    pointer_staging.as_str(),
                    self.root.fd.as_fd(),
                    "current",
                )
                .and_then(|()| filesystem.sync(self.root.fd.as_fd()));
            return match rollback {
                Ok(()) => Err(commit_error.into()),
                Err(rollback_error) => Ok(GenerationPublicationOutcome::OutcomeAmbiguous {
                    candidate: generation,
                    cause: format!(
                        "pointer sync failed ({commit_error}) and rollback failed ({rollback_error})"
                    ),
                }),
            };
        }
        let committed_marker = format!(".committed-{generation_name}");
        if let Err(cause) = filesystem
            .rename(
                self.root.fd.as_fd(),
                pointer_staging.as_str(),
                self.root.fd.as_fd(),
                committed_marker.as_str(),
            )
            .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
        {
            return Ok(GenerationPublicationOutcome::OutcomeAmbiguous {
                candidate: generation,
                cause: format!(
                    "commit-marker transition failed after pointer update; outcome is unconfirmed: {cause}"
                ),
            });
        }
        if let Err(cause) = filesystem
            .unlink(self.root.fd.as_fd(), committed_marker.as_str())
            .and_then(|()| filesystem.sync(self.root.fd.as_fd()))
        {
            return Ok(GenerationPublicationOutcome::CommittedWithCleanupPending {
                generation,
                cause,
            });
        }
        Ok(GenerationPublicationOutcome::Committed(generation))
    }

    fn create_lease_locked(&self, record: &LeaseRecord) -> std::result::Result<String, String> {
        let mut filesystem = RealPublicationFilesystem;
        for _ in 0..64 {
            let name = random_generation_id()?.0;
            match write_synced_file(&self.leases.fd, &name, &record.encode(), &mut filesystem) {
                Ok(()) => {
                    if let Err(error) = filesystem.sync(self.leases.fd.as_fd()) {
                        let _ = unlinkat(&self.leases.fd, name.as_str(), AtFlags::empty());
                        let _ = fsync(&self.leases.fd);
                        return Err(error);
                    }
                    return Ok(name);
                }
                Err(error) if error == Errno::EXIST.to_string() => continue,
                Err(error) => return Err(error),
            }
        }
        Err("lease id collision retry limit exhausted".into())
    }

    fn resolve_current(&self) -> std::result::Result<GenerationId, String> {
        let Some(bytes) = read_optional_regular_bytes(&self.root, Path::new("current"))? else {
            return Err("current generation is absent".into());
        };
        let generation = parse_pointer_record(&bytes, "current")?;
        self.validate_generation(&generation)?;
        Ok(generation)
    }

    fn validate_generation(&self, generation: &GenerationId) -> std::result::Result<(), String> {
        Self::validate_generation_from(&self.generations, generation)
    }

    fn validate_generation_from(
        generations: &GenerationRoot,
        generation: &GenerationId,
    ) -> std::result::Result<(), String> {
        Self::open_validated_generation_with_manifest_from(generations, generation).map(|_| ())
    }

    fn open_validated_generation_with_manifest(
        &self,
        generation: &GenerationId,
    ) -> std::result::Result<(GenerationRoot, Manifest), String> {
        Self::open_validated_generation_with_manifest_from(&self.generations, generation)
    }

    fn open_validated_generation_with_manifest_from(
        generations: &GenerationRoot,
        generation: &GenerationId,
    ) -> std::result::Result<(GenerationRoot, Manifest), String> {
        let generation_root = GenerationRoot {
            fd: openat(
                &generations.fd,
                generation.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?,
        };
        let manifest = verify_opened_generation(&generation_root, generation)?;
        Ok((generation_root, manifest))
    }

    fn clean_staging(&self) -> std::result::Result<(), String> {
        let mut staging = Vec::new();
        let mut directory =
            Dir::read_from(&self.generations.fd).map_err(|error| error.to_string())?;
        while let Some(entry) = directory.read() {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_bytes();
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            if name
                .strip_prefix(".staging-")
                .is_some_and(|id| GenerationId::parse(id).is_ok())
            {
                staging.push(name.to_owned());
            }
        }
        drop(directory);
        for name in staging {
            remove_tree_at(&self.generations.fd, OsStr::new(&name))?;
        }
        fsync(&self.generations.fd).map_err(|error| error.to_string())
    }

    fn clean_pointer_staging<H, F>(
        &self,
        inventory: PointerJournalInventory,
        mut checkpoint: H,
        filesystem: &mut F,
    ) -> std::result::Result<(), String>
    where
        H: FnMut(RecoveryCheckpoint) -> std::result::Result<(), String>,
        F: PublicationFilesystem,
    {
        match inventory.state {
            PointerJournalState::Clean => return Ok(()),
            PointerJournalState::DiscardCurrent { name } => {
                filesystem.unlink(self.root.fd.as_fd(), name.as_str())?;
            }
            PointerJournalState::RollbackCurrent { name } => {
                filesystem.rename(
                    self.root.fd.as_fd(),
                    name.as_str(),
                    self.root.fd.as_fd(),
                    "current",
                )?;
            }
            PointerJournalState::RollbackAbsent { name } => {
                let current_name = name.replacen(".absent-", ".current-", 1);
                filesystem.rename(
                    self.root.fd.as_fd(),
                    "current",
                    self.root.fd.as_fd(),
                    current_name.as_str(),
                )?;
                checkpoint(RecoveryCheckpoint::PristineCurrentJournaled)?;
                filesystem.sync(self.root.fd.as_fd())?;
                filesystem.unlink(self.root.fd.as_fd(), name.as_str())?;
                checkpoint(RecoveryCheckpoint::PristineAbsentRemoved)?;
                filesystem.unlink(self.root.fd.as_fd(), current_name.as_str())?;
            }
            PointerJournalState::DiscardPristine {
                current_name,
                absent_name,
            } => {
                filesystem.unlink(self.root.fd.as_fd(), absent_name.as_str())?;
                filesystem.unlink(self.root.fd.as_fd(), current_name.as_str())?;
            }
            PointerJournalState::CleanupCommitted { name, .. } => {
                filesystem.unlink(self.root.fd.as_fd(), name.as_str())?;
            }
        }
        filesystem.sync(self.root.fd.as_fd())
    }

    fn inspect_pointer_journals(&self) -> std::result::Result<PointerJournalInventory, String> {
        Self::inspect_pointer_journals_from(&self.root, &self.generations)
    }

    fn inspect_pointer_journals_from(
        root: &GenerationRoot,
        generations: &GenerationRoot,
    ) -> std::result::Result<PointerJournalInventory, String> {
        let mut current_journals = Vec::new();
        let mut absent_journals = Vec::new();
        let mut committed_journals = Vec::new();
        let mut directory = Dir::read_from(&root.fd).map_err(|error| error.to_string())?;
        while let Some(entry) = directory.read() {
            let entry = entry.map_err(|error| error.to_string())?;
            let bytes = entry.file_name().to_bytes();
            if bytes.starts_with(b".current-") {
                let name = std::str::from_utf8(bytes)
                    .map_err(|_| "reserved current journal name is not UTF-8")?;
                let suffix = GenerationId::parse(
                    name.strip_prefix(".current-")
                        .expect("byte prefix was checked"),
                )?;
                let record = read_optional_regular_bytes(root, Path::new(name))?
                    .ok_or("current journal disappeared during inventory")?;
                current_journals.push(PointerJournalEntry {
                    name: name.to_owned(),
                    suffix,
                    content: Some(parse_pointer_record(&record, "current journal")?),
                });
            } else if bytes.starts_with(b".absent-") {
                let name = std::str::from_utf8(bytes)
                    .map_err(|_| "reserved absent journal name is not UTF-8")?;
                let suffix = GenerationId::parse(
                    name.strip_prefix(".absent-")
                        .expect("byte prefix was checked"),
                )?;
                let record = read_optional_regular_bytes(root, Path::new(name))?
                    .ok_or("absent journal disappeared during inventory")?;
                if !record.is_empty() {
                    return Err("absent-pointer marker is malformed".into());
                }
                absent_journals.push(PointerJournalEntry {
                    name: name.to_owned(),
                    suffix,
                    content: None,
                });
            } else if bytes.starts_with(b".committed-") {
                let name = std::str::from_utf8(bytes)
                    .map_err(|_| "reserved committed journal name is not UTF-8")?;
                let suffix = GenerationId::parse(
                    name.strip_prefix(".committed-")
                        .expect("byte prefix was checked"),
                )?;
                let record = read_optional_regular_bytes(root, Path::new(name))?
                    .ok_or("committed journal disappeared during inventory")?;
                let content = if record.is_empty() {
                    None
                } else {
                    Some(parse_pointer_record(&record, "committed journal")?)
                };
                committed_journals.push(PointerJournalEntry {
                    name: name.to_owned(),
                    suffix,
                    content,
                });
            }
        }
        drop(directory);

        let current = read_optional_regular_bytes(root, Path::new("current"))?
            .map(|bytes| parse_pointer_record(&bytes, "current"))
            .transpose()?;
        if let Some(current) = &current {
            Self::validate_generation_from(generations, current)?;
        }
        let state = match (
            current_journals.len(),
            absent_journals.len(),
            committed_journals.len(),
        ) {
            (0, 0, 0) => PointerJournalState::Clean,
            (1, 0, 0) => {
                let journal = current_journals.pop().expect("length was checked");
                let staged = journal
                    .content
                    .as_ref()
                    .expect("current journal has content");
                match (
                    current.as_ref() == Some(&journal.suffix),
                    staged == &journal.suffix,
                ) {
                    (false, true) => {
                        Self::validate_generation_from(generations, &journal.suffix)?;
                        PointerJournalState::DiscardCurrent { name: journal.name }
                    }
                    (true, false) => {
                        Self::validate_generation_from(generations, staged)?;
                        PointerJournalState::RollbackCurrent { name: journal.name }
                    }
                    _ => return Err("current journal is inconsistent with current".into()),
                }
            }
            (0, 1, 0) => {
                let journal = absent_journals.pop().expect("length was checked");
                if current.as_ref() != Some(&journal.suffix) {
                    return Err("absent journal is inconsistent with current".into());
                }
                Self::validate_generation_from(generations, &journal.suffix)?;
                PointerJournalState::RollbackAbsent { name: journal.name }
            }
            (1, 1, 0) => {
                let current_journal = current_journals.pop().expect("length was checked");
                let absent_journal = absent_journals.pop().expect("length was checked");
                if current.is_some()
                    || current_journal.suffix != absent_journal.suffix
                    || current_journal.content.as_ref() != Some(&current_journal.suffix)
                {
                    return Err("pristine pointer journals are inconsistent".into());
                }
                Self::validate_generation_from(generations, &current_journal.suffix)?;
                PointerJournalState::DiscardPristine {
                    current_name: current_journal.name,
                    absent_name: absent_journal.name,
                }
            }
            (0, 0, 1) => {
                let journal = committed_journals.pop().expect("length was checked");
                if current.as_ref() != Some(&journal.suffix)
                    || journal.content.as_ref() == Some(&journal.suffix)
                {
                    return Err("committed journal is inconsistent with current".into());
                }
                Self::validate_generation_from(generations, &journal.suffix)?;
                if let Some(previous) = &journal.content {
                    Self::validate_generation_from(generations, previous)?;
                }
                PointerJournalState::CleanupCommitted {
                    name: journal.name,
                    previous: journal.content,
                }
            }
            _ => return Err("pointer journal inventory is contradictory".into()),
        };

        Ok(PointerJournalInventory { current, state })
    }
}

impl Drop for GenerationOperationLock<'_> {
    fn drop(&mut self) {
        let _ = flock(self.lock, FlockOperation::Unlock);
    }
}

impl Drop for SelectionCleanupFileLock {
    fn drop(&mut self) {
        let _ = flock(&self.0, FlockOperation::Unlock);
    }
}

impl GenerationSelection {
    /// Return the immutable generation identity selected for this launch.
    pub fn as_str(&self) -> &str {
        self.generation.as_str()
    }

    /// Read one normalized manifest output through the held generation descriptor.
    pub fn read_output(&self, path: &str) -> std::result::Result<Vec<u8>, String> {
        if !safe_output_path(path) {
            return Err("unsafe generation output path".into());
        }
        read_regular_bytes(&self.root, Path::new(path))
    }

    /// Return the validated generation path to pass to target dependency evaluation.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Durably release this launch's lease after its target has exited.
    pub fn release(mut self) -> std::result::Result<(), String> {
        self.release_matching_process_lease()
    }

    fn read_current_lease(&self) -> std::result::Result<Option<ParsedLeaseRecord>, String> {
        self.read_current_lease_entry()
            .map(|entry| entry.map(|entry| entry.record))
    }

    fn read_current_lease_entry(
        &self,
    ) -> std::result::Result<Option<ValidatedLeaseInventoryEntry>, String> {
        match statat(
            &self.lease_directory,
            self.lease_name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(_) => read_validated_lease_inventory_entry(
                &self.lease_directory,
                self.lease_name.as_str(),
            )
            .map(Some),
            Err(Errno::NOENT) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn revalidate_for_lifecycle_transfer(&self) -> std::result::Result<(), String> {
        if self.process_identity.owner_uid != rustix::process::getuid().as_raw()
            || self.process_identity.boot_id != linux_boot_id()?
        {
            return Err("generation selection owner identity is stale".into());
        }
        let (start_time, owner_uid) = linux_process_identity(self.process_identity.pid)?;
        if start_time != self.process_identity.start_time
            || owner_uid != self.process_identity.owner_uid
        {
            return Err("generation selection process identity is stale".into());
        }
        let manifest = verify_opened_generation(&self.root, &self.generation)?;
        if digest_hex(manifest.v1.raw.as_bytes()) != self.manifest_seal_digest {
            return Err("generation manifest changed after selection".into());
        }
        match self.read_current_lease()? {
            Some(ParsedLeaseRecord::Process(record)) if record == self.process_identity => Ok(()),
            _ => Err("generation selection lacks its exact process lease".into()),
        }
    }

    #[allow(dead_code)]
    fn replace_with_lifecycle_locked(
        &self,
        record: LifecycleLeaseRecord,
    ) -> std::result::Result<(), String> {
        let mut filesystem = RealLeaseFilesystem;
        self.replace_with_lifecycle_locked_with_filesystem(record, &mut filesystem)
    }

    fn replace_with_lifecycle_locked_with_filesystem<F: LeaseFilesystem>(
        &self,
        record: LifecycleLeaseRecord,
        filesystem: &mut F,
    ) -> std::result::Result<(), String> {
        if record.generation != self.generation
            || record.pid != self.process_identity.pid
            || record.start_time != self.process_identity.start_time
            || record.boot_id != self.process_identity.boot_id
            || record.owner_uid != self.process_identity.owner_uid
        {
            return Err("lifecycle lease does not match its source selection".into());
        }
        let raw = record.encode();
        if LifecycleLeaseRecord::parse(&raw)? != record {
            return Err("lifecycle lease is not canonical".into());
        }
        if self.transfer_staging_present()? {
            return Err(
                "generation lease transfer staging requires registry reconciliation".into(),
            );
        }
        self.revalidate_for_lifecycle_transfer()?;
        let source_process =
            read_validated_lease_inventory_entry(&self.lease_directory, self.lease_name.as_str())?;
        if source_process.record != ParsedLeaseRecord::Process(self.process_identity.clone()) {
            return Err("lifecycle lease transfer source changed before staging".into());
        }

        let temporary = format!(".lease-transfer-{}", self.lease_name);
        let descriptor = openat(
            &self.lease_directory,
            ".",
            OFlags::RDWR | OFlags::TMPFILE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|error| error.to_string())?;
        let mut file = std::fs::File::from(descriptor);
        let staging_identity =
            rustix::fs::fstat(file.as_fd()).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "lifecycle lease transfer staging file",
            &staging_identity,
            FileType::RegularFile,
            0o600,
            rustix::process::getuid().as_raw(),
        )?;
        let staged = (|| {
            let split = raw.len() / 2;
            file.write_all(&raw[..split])
                .map_err(|error| error.to_string())?;
            filesystem.checkpoint(TransferCheckpoint::DuringStageWrite)?;
            file.write_all(&raw[split..])
                .map_err(|error| error.to_string())?;
            fsync(file.as_fd()).map_err(|error| error.to_string())?;
            let after_write = rustix::fs::fstat(file.as_fd()).map_err(|error| error.to_string())?;
            validate_owned_mode(
                "unnamed lifecycle lease transfer staging file",
                &after_write,
                FileType::RegularFile,
                0o600,
                rustix::process::getuid().as_raw(),
            )?;
            if after_write.st_dev != staging_identity.st_dev
                || after_write.st_ino != staging_identity.st_ino
                || after_write.st_size != raw.len() as i64
                || after_write.st_size > 4096
            {
                return Err("unnamed lifecycle lease transfer staging file changed".into());
            }
            file.seek(SeekFrom::Start(0))
                .map_err(|error| error.to_string())?;
            let mut held_raw = Vec::new();
            Read::by_ref(&mut file)
                .take(4097)
                .read_to_end(&mut held_raw)
                .map_err(|error| error.to_string())?;
            if held_raw != raw || LifecycleLeaseRecord::parse(&held_raw)? != record {
                return Err("unnamed lifecycle lease transfer bytes changed".into());
            }
            let staged_lifecycle = ValidatedLeaseInventoryEntry {
                record: ParsedLeaseRecord::Lifecycle(record.clone()),
                descriptor: file
                    .try_clone()
                    .map(OwnedFd::from)
                    .map_err(|error| error.to_string())?,
            };

            if filesystem
                .link_unnamed_empty_path(
                    &staged_lifecycle.descriptor,
                    &self.lease_directory,
                    temporary.as_str(),
                )
                .is_err()
            {
                let proc_source = filesystem.proc_fd_source(&staged_lifecycle.descriptor)?;
                let proc_identity = statat(CWD, &proc_source, AtFlags::empty())
                    .map_err(|error| error.to_string())?;
                validate_owned_mode(
                    "lifecycle lease transfer proc descriptor source",
                    &proc_identity,
                    FileType::RegularFile,
                    0o600,
                    rustix::process::getuid().as_raw(),
                )?;
                let held_identity = rustix::fs::fstat(&staged_lifecycle.descriptor)
                    .map_err(|error| error.to_string())?;
                if proc_identity.st_dev != held_identity.st_dev
                    || proc_identity.st_ino != held_identity.st_ino
                    || proc_identity.st_size != held_identity.st_size
                {
                    return Err(
                        "lifecycle lease transfer proc descriptor source is mismatched".into(),
                    );
                }
                filesystem
                    .link_unnamed_proc(&proc_source, &self.lease_directory, temporary.as_str())
                    .map_err(|error| error.to_string())?;
            }

            if !validated_lease_record_matches(
                &self.lease_directory,
                temporary.as_str(),
                &staged_lifecycle,
            )? {
                return Err("published lifecycle lease transfer staging file changed".into());
            }
            fsync(&self.lease_directory).map_err(|error| error.to_string())?;
            filesystem.checkpoint(TransferCheckpoint::BeforeReplace)?;
            self.revalidate_for_lifecycle_transfer()?;

            if !validated_lease_record_matches(
                &self.lease_directory,
                self.lease_name.as_str(),
                &source_process,
            )? || !validated_lease_path_matches(
                &self.lease_directory,
                temporary.as_str(),
                &staged_lifecycle.descriptor,
            )? || !validated_lease_path_matches(
                &self.lease_directory,
                self.lease_name.as_str(),
                &source_process.descriptor,
            )? {
                return Err("lifecycle lease transfer pair changed before exchange".into());
            }

            filesystem.checkpoint(TransferCheckpoint::BeforeExchange)?;
            renameat_with(
                &self.lease_directory,
                temporary.as_str(),
                &self.lease_directory,
                self.lease_name.as_str(),
                RenameFlags::EXCHANGE,
            )
            .map_err(|error| error.to_string())?;

            let target_is_staged_lifecycle = validated_lease_record_matches(
                &self.lease_directory,
                self.lease_name.as_str(),
                &staged_lifecycle,
            )
            .unwrap_or(false);
            let staging_is_source_process = validated_lease_record_matches(
                &self.lease_directory,
                temporary.as_str(),
                &source_process,
            )
            .unwrap_or(false);
            if !target_is_staged_lifecycle || !staging_is_source_process {
                if !target_is_staged_lifecycle && staging_is_source_process {
                    renameat_with(
                        &self.lease_directory,
                        temporary.as_str(),
                        &self.lease_directory,
                        self.lease_name.as_str(),
                        RenameFlags::EXCHANGE,
                    )
                    .map_err(|error| {
                        format!(
                            "lifecycle lease exchange is ambiguous and rollback failed: {error}"
                        )
                    })?;
                    fsync(&self.lease_directory).map_err(|error| error.to_string())?;
                    if validated_lease_record_matches(
                        &self.lease_directory,
                        self.lease_name.as_str(),
                        &source_process,
                    )? {
                        return Err(
                            "lifecycle lease exchange detected a pathname swap and rolled back"
                                .into(),
                        );
                    }
                }
                return Err("lifecycle lease exchange result is ambiguous".into());
            }

            filesystem.checkpoint(TransferCheckpoint::AfterReplace)?;
            fsync(&self.lease_directory).map_err(|error| error.to_string())?;
            filesystem.checkpoint(TransferCheckpoint::AfterLeaseDirectoryFsync)?;
            if !validated_lease_record_matches(
                &self.lease_directory,
                self.lease_name.as_str(),
                &staged_lifecycle,
            )? || !validated_lease_record_matches(
                &self.lease_directory,
                temporary.as_str(),
                &source_process,
            )? {
                return Err("lifecycle lease exchange changed before staging cleanup".into());
            }
            unlinkat(&self.lease_directory, temporary.as_str(), AtFlags::empty())
                .map_err(|error| error.to_string())?;
            fsync(&self.lease_directory).map_err(|error| error.to_string())
        })();
        let exact_untransferred_pair = matches!(
            read_validated_lease_inventory_entry(
                &self.lease_directory,
                temporary.as_str(),
            ),
            Ok(staging)
                if staging.record == ParsedLeaseRecord::Lifecycle(record.clone())
                    && matches!(
                        transfer_staging_matches(
                            &self.lease_directory,
                            temporary.as_str(),
                            &staging_identity,
                        ),
                        Ok(true)
                    )
                    && matches!(
                        validated_lease_record_matches(
                            &self.lease_directory,
                            self.lease_name.as_str(),
                            &source_process,
                        ),
                        Ok(true)
                    )
        );
        if staged.is_err() && exact_untransferred_pair {
            let _ = unlinkat(&self.lease_directory, temporary.as_str(), AtFlags::empty());
            let _ = fsync(&self.lease_directory);
        }
        staged
    }

    fn release_matching_process_lease(&mut self) -> std::result::Result<(), String> {
        self.release_matching_process_lease_with_checkpoint(|| {})
    }

    fn release_matching_process_lease_with_checkpoint<H>(
        &mut self,
        before_final_proof: H,
    ) -> std::result::Result<(), String>
    where
        H: FnOnce(),
    {
        let intra_process_lock = Arc::clone(&self.intra_process_lock);
        let _intra_process = intra_process_lock
            .lock()
            .map_err(|_| "generation selection cleanup mutex was poisoned")?;
        let lock = self
            .activation_lock
            .try_clone()
            .map_err(|error| error.to_string())?;
        flock(&lock, FlockOperation::LockShared).map_err(|error| error.to_string())?;
        let _cleanup_lock = SelectionCleanupFileLock(lock);
        self.release_matching_process_lease_locked_with_checkpoint(before_final_proof)
    }

    fn release_matching_process_lease_locked_with_checkpoint<H>(
        &mut self,
        before_final_proof: H,
    ) -> std::result::Result<(), String>
    where
        H: FnOnce(),
    {
        if self.released {
            return Ok(());
        }

        let transfer_staging = format!(".lease-transfer-{}", self.lease_name);
        match statat(
            &self.lease_directory,
            transfer_staging.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(_) => {
                return Err(
                    "generation lease release retained ambiguous transfer staging evidence".into(),
                );
            }
            Err(Errno::NOENT) => {}
            Err(error) => return Err(error.to_string()),
        }

        let current = match self.read_current_lease_entry() {
            Ok(current) => current,
            Err(_) => return Ok(()),
        };
        match current {
            Some(current)
                if current.record == ParsedLeaseRecord::Process(self.process_identity.clone()) =>
            {
                before_final_proof();
                if !validated_lease_path_matches(
                    &self.lease_directory,
                    self.lease_name.as_str(),
                    &current.descriptor,
                )? {
                    self.released = true;
                    return Ok(());
                }
                match unlinkat(
                    &self.lease_directory,
                    self.lease_name.as_str(),
                    AtFlags::empty(),
                ) {
                    Ok(()) | Err(Errno::NOENT) => {}
                    Err(error) => return Err(error.to_string()),
                }
                fsync(&self.lease_directory).map_err(|error| error.to_string())?;
            }
            Some(_) => {}
            None => fsync(&self.lease_directory).map_err(|error| error.to_string())?,
        }
        self.released = true;
        Ok(())
    }

    #[allow(dead_code)]
    fn disarm_after_transfer(&mut self) {
        self.released = true;
    }

    fn retain_after_authority_rejection(&mut self) {
        self.released = true;
    }

    fn retain_for_registry_reconciliation(&mut self) {
        self.released = true;
    }

    fn transfer_staging_present(&self) -> std::result::Result<bool, String> {
        classify_lease_transfer_staging_locked(&self.lease_directory).map(|plan| plan.has_staging())
    }
}

fn validated_lease_record_matches(
    lease_directory: &OwnedFd,
    name: &str,
    expected: &ValidatedLeaseInventoryEntry,
) -> std::result::Result<bool, String> {
    let current = match read_validated_lease_inventory_entry(lease_directory, name) {
        Ok(current) => current,
        Err(_) => return Ok(false),
    };
    let expected_stat =
        rustix::fs::fstat(&expected.descriptor).map_err(|error| error.to_string())?;
    let current_stat = rustix::fs::fstat(&current.descriptor).map_err(|error| error.to_string())?;
    Ok(current.record == expected.record
        && current_stat.st_dev == expected_stat.st_dev
        && current_stat.st_ino == expected_stat.st_ino)
}

fn transfer_staging_matches(
    parent: &OwnedFd,
    name: &str,
    opened: &rustix::fs::Stat,
) -> std::result::Result<bool, String> {
    let path = match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(path) => path,
        Err(Errno::NOENT) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    validate_owned_mode(
        "lifecycle lease transfer staging file",
        &path,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    Ok(path.st_dev == opened.st_dev && path.st_ino == opened.st_ino)
}

fn classify_lease_transfer_staging_locked(
    lease_directory: &OwnedFd,
) -> std::result::Result<LeaseTransferRecoveryPlan, String> {
    let mut final_entries = BTreeMap::<String, ValidatedLeaseInventoryEntry>::new();
    let mut staging_entries = BTreeMap::<String, ValidatedLeaseInventoryEntry>::new();
    let mut inventory_entries = 0_usize;
    let mut inventory_bytes = 0_usize;
    let mut directory = Dir::read_from(lease_directory).map_err(|error| error.to_string())?;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        inventory_entries = inventory_entries
            .checked_add(1)
            .ok_or("generation lease inventory exceeds its scan bounds")?;
        let stat = statat(
            lease_directory,
            entry.file_name(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|error| error.to_string())?;
        let size = usize::try_from(stat.st_size)
            .map_err(|_| "generation lease inventory has a negative size")?;
        inventory_bytes = inventory_bytes
            .checked_add(size)
            .ok_or("generation lease inventory exceeds its scan bounds")?;
        if inventory_entries > MAX_LEASE_INVENTORY_ENTRIES
            || inventory_bytes > MAX_LEASE_INVENTORY_BYTES
        {
            return Err("generation lease inventory exceeds its scan bounds".into());
        }
        let name = std::str::from_utf8(bytes)
            .map_err(|_| "generation lease inventory name is not UTF-8")?;
        if GenerationId::parse(name).is_ok() {
            let validated = read_validated_lease_inventory_entry(lease_directory, name)?;
            if final_entries.insert(name.to_owned(), validated).is_some() {
                return Err("generation lease inventory contains a duplicate name".into());
            }
            continue;
        }
        let Some(target) = name.strip_prefix(".lease-transfer-") else {
            return Err("generation lease inventory contains an unknown name".into());
        };
        GenerationId::parse(target)
            .map_err(|_| "generation lease transfer staging name is malformed")?;
        let validated = read_validated_lease_inventory_entry(lease_directory, name)?;
        if staging_entries
            .insert(target.to_owned(), validated)
            .is_some()
        {
            return Err("generation lease inventory contains duplicate transfer staging".into());
        }
    }
    drop(directory);

    for (target_name, staging) in &staging_entries {
        let target = final_entries
            .get(target_name)
            .ok_or("generation lease transfer staging target is absent")?;
        let exact_pair = match (&target.record, &staging.record) {
            (ParsedLeaseRecord::Process(process), ParsedLeaseRecord::Lifecycle(lifecycle))
            | (ParsedLeaseRecord::Lifecycle(lifecycle), ParsedLeaseRecord::Process(process)) => {
                lifecycle_matches_process(lifecycle, process)
            }
            _ => false,
        };
        if !exact_pair {
            return Err("generation lease transfer staging pair is inconsistent".into());
        }
    }

    Ok(LeaseTransferRecoveryPlan {
        final_entries,
        staging_entries,
    })
}

fn gc_lease_inventory_preflight(
    lease_directory: &OwnedFd,
) -> std::result::Result<GcLeaseInventoryPreflight, String> {
    let mut canonical_names = Vec::new();
    let mut has_transfer_stage = false;
    let mut inventory_entries = 0_usize;
    let mut inventory_bytes = 0_usize;
    let current_uid = rustix::process::getuid().as_raw();
    let mut entries = Dir::read_from(lease_directory).map_err(|error| error.to_string())?;
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let bytes = entry.file_name().to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        inventory_entries = inventory_entries
            .checked_add(1)
            .ok_or("generation lease inventory exceeds its scan bounds")?;
        let stat = statat(
            lease_directory,
            entry.file_name(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|error| error.to_string())?;
        let size = usize::try_from(stat.st_size)
            .map_err(|_| "generation lease inventory has a negative size")?;
        inventory_bytes = inventory_bytes
            .checked_add(size)
            .ok_or("generation lease inventory exceeds its scan bounds")?;
        if inventory_entries > MAX_LEASE_INVENTORY_ENTRIES
            || inventory_bytes > MAX_LEASE_INVENTORY_BYTES
        {
            return Err("generation lease inventory exceeds its scan bounds".into());
        }
        validate_owned_mode(
            "generation lease inventory entry",
            &stat,
            FileType::RegularFile,
            0o600,
            current_uid,
        )?;
        if size > 4096 {
            return Err("generation lease inventory entry exceeds 4096 bytes".into());
        }
        let name = std::str::from_utf8(bytes)
            .map_err(|_| "generation lease inventory name is not UTF-8")?;
        if name.starts_with(".lease-transfer-") {
            has_transfer_stage = true;
        } else if GenerationId::parse(name).is_ok() {
            canonical_names.push(name.to_owned());
        } else {
            return Err("generation lease inventory contains an unknown name".into());
        }
    }
    Ok(GcLeaseInventoryPreflight {
        canonical_names,
        has_transfer_stage,
    })
}

fn read_validated_lease_inventory_entry(
    lease_directory: &OwnedFd,
    name: &str,
) -> std::result::Result<ValidatedLeaseInventoryEntry, String> {
    let before = statat(lease_directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| error.to_string())?;
    validate_owned_mode(
        "generation lease inventory entry",
        &before,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if before.st_size > 4096 {
        return Err("generation lease inventory entry exceeds 4096 bytes".into());
    }
    let descriptor = openat(
        lease_directory,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let opened = rustix::fs::fstat(&descriptor).map_err(|error| error.to_string())?;
    if before.st_dev != opened.st_dev || before.st_ino != opened.st_ino {
        return Err("generation lease inventory entry changed during open".into());
    }
    let mut file = std::fs::File::from(descriptor.try_clone().map_err(|error| error.to_string())?);
    let mut raw = Vec::new();
    Read::by_ref(&mut file)
        .take(4097)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() > 4096 {
        return Err("generation lease inventory entry exceeds 4096 bytes".into());
    }
    let record = ParsedLeaseRecord::parse(&raw)?;
    let canonical = match &record {
        ParsedLeaseRecord::Process(record) => record.encode(),
        ParsedLeaseRecord::Lifecycle(record) => record.encode(),
    };
    if canonical != raw {
        return Err("generation lease inventory entry is not canonical".into());
    }
    if !validated_lease_path_matches(lease_directory, name, &descriptor)? {
        return Err("generation lease inventory entry changed during read".into());
    }
    Ok(ValidatedLeaseInventoryEntry { record, descriptor })
}

fn validated_lease_path_matches(
    lease_directory: &OwnedFd,
    name: &str,
    descriptor: &OwnedFd,
) -> std::result::Result<bool, String> {
    let path = match statat(lease_directory, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(path) => path,
        Err(Errno::NOENT) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    validate_owned_mode(
        "generation lease inventory entry",
        &path,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if path.st_size > 4096 {
        return Err("generation lease inventory entry exceeds 4096 bytes".into());
    }
    let opened = rustix::fs::fstat(descriptor).map_err(|error| error.to_string())?;
    Ok(path.st_dev == opened.st_dev && path.st_ino == opened.st_ino)
}

fn lifecycle_matches_process(lifecycle: &LifecycleLeaseRecord, process: &LeaseRecord) -> bool {
    lifecycle.generation == process.generation
        && lifecycle.pid == process.pid
        && lifecycle.start_time == process.start_time
        && lifecycle.boot_id == process.boot_id
        && lifecycle.owner_uid == process.owner_uid
}

impl Drop for GenerationSelection {
    fn drop(&mut self) {
        let _ = self.release_matching_process_lease();
    }
}

impl LeaseRecord {
    fn for_process(generation: GenerationId, pid: u32) -> std::result::Result<Self, String> {
        let (start_time, owner_uid) = linux_process_identity(pid)?;
        if owner_uid != rustix::process::getuid().as_raw() {
            return Err("lease target is not owned by the current user".into());
        }
        Ok(Self {
            generation,
            pid,
            start_time,
            boot_id: linux_boot_id()?,
            owner_uid,
        })
    }

    fn encode(&self) -> Vec<u8> {
        format!(
            "helm-generation-lease-v1\ngeneration {}\npid {}\nstart-time {}\nboot-id {}\nowner-uid {}\n",
            self.generation.as_str(),
            self.pid,
            self.start_time,
            self.boot_id,
            self.owner_uid,
        )
        .into_bytes()
    }

    fn parse(raw: &[u8]) -> std::result::Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|_| "generation lease is not UTF-8")?;
        if text.contains('\r') || !text.ends_with('\n') {
            return Err("generation lease must use LF-terminated lines".into());
        }
        let mut lines = text[..text.len() - 1].split('\n');
        if lines.next() != Some("helm-generation-lease-v1") {
            return Err("unsupported generation lease version".into());
        }
        let generation = GenerationId::parse(record_value(&mut lines, "generation")?)?;
        let pid = canonical_u32(record_value(&mut lines, "pid")?, "lease PID")?;
        let start_time =
            canonical_u64(record_value(&mut lines, "start-time")?, "lease start time")?;
        let boot_id = record_value(&mut lines, "boot-id")?;
        if !canonical_boot_id(boot_id) {
            return Err("lease boot ID is malformed".into());
        }
        let owner_uid = canonical_u32(record_value(&mut lines, "owner-uid")?, "lease owner UID")?;
        if lines.next().is_some() {
            return Err("generation lease has extra records".into());
        }
        Ok(Self {
            generation,
            pid,
            start_time,
            boot_id: boot_id.into(),
            owner_uid,
        })
    }

    fn liveness(&self) -> LeaseLiveness {
        let boot_id = match linux_boot_id() {
            Ok(boot_id) => boot_id,
            Err(_) => return LeaseLiveness::Uncertain,
        };
        if self.boot_id != boot_id {
            return LeaseLiveness::Stale;
        }
        let proc_root = match open_directory_chain(Path::new("/proc")) {
            Ok(proc_root) => proc_root,
            Err(_) => return LeaseLiveness::Uncertain,
        };
        let process = match openat(
            &proc_root,
            self.pid.to_string(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(process) => process,
            Err(Errno::NOENT) => return LeaseLiveness::Stale,
            Err(_) => return LeaseLiveness::Uncertain,
        };
        let stat = match rustix::fs::fstat(&process) {
            Ok(stat) => stat,
            Err(_) => return LeaseLiveness::Uncertain,
        };
        if stat.st_uid != self.owner_uid {
            return LeaseLiveness::Uncertain;
        }
        match linux_process_start_time_from(&process) {
            Ok(start_time) if start_time == self.start_time => LeaseLiveness::Live,
            Ok(_) => LeaseLiveness::Stale,
            Err(error) if error == Errno::NOENT.to_string() => LeaseLiveness::Stale,
            Err(_) => LeaseLiveness::Uncertain,
        }
    }
}

impl ParsedLeaseRecord {
    fn parse(raw: &[u8]) -> std::result::Result<Self, String> {
        match raw.split(|byte| *byte == b'\n').next() {
            Some(b"helm-generation-lease-v1") => Ok(Self::Process(LeaseRecord::parse(raw)?)),
            Some(b"helm-generation-lifecycle-lease-v1") => {
                Ok(Self::Lifecycle(LifecycleLeaseRecord::parse(raw)?))
            }
            _ => Err("unsupported generation lease version".into()),
        }
    }
}

impl LifecycleLeaseRecord {
    fn encode(&self) -> Vec<u8> {
        format!(
            "helm-generation-lifecycle-lease-v1\ngeneration {}\nlaunch {}\npid {}\nstart-time {}\nboot-id {}\nowner-uid {}\nowner-kind {}\nunit {}\nunit-invocation {}\nprocess-group {}\ncgroup {}\ncgroup-device {}\ncgroup-inode {}\n",
            self.generation.as_str(),
            self.launch.as_str(),
            self.pid,
            self.start_time,
            self.boot_id,
            self.owner_uid,
            match self.owner_kind {
                LifecycleOwnerKind::SystemdScope => "systemd-scope",
                LifecycleOwnerKind::ProcessGroup => "process-group",
            },
            self.unit,
            self.unit_invocation,
            self.process_group,
            self.cgroup,
            self.cgroup_device,
            self.cgroup_inode,
        )
        .into_bytes()
    }

    fn parse(raw: &[u8]) -> std::result::Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|_| "lifecycle lease is not UTF-8")?;
        if text.contains('\r') || !text.ends_with('\n') {
            return Err("lifecycle lease must use LF-terminated lines".into());
        }
        let mut lines = text[..text.len() - 1].split('\n');
        if lines.next() != Some("helm-generation-lifecycle-lease-v1") {
            return Err("unsupported lifecycle lease version".into());
        }
        let generation = GenerationId::parse(record_value(&mut lines, "generation")?)?;
        let launch = GenerationId::parse(record_value(&mut lines, "launch")?)?;
        let pid = canonical_u32(record_value(&mut lines, "pid")?, "lifecycle lease PID")?;
        if pid == 0 {
            return Err("lifecycle lease PID must be positive".into());
        }
        let start_time = canonical_positive_u64(
            record_value(&mut lines, "start-time")?,
            "lifecycle lease start time",
        )?;
        let boot_id = record_value(&mut lines, "boot-id")?;
        if !canonical_boot_id(boot_id) {
            return Err("lifecycle lease boot ID is malformed".into());
        }
        let owner_uid = canonical_u32(
            record_value(&mut lines, "owner-uid")?,
            "lifecycle lease owner UID",
        )?;
        let owner_kind = match record_value(&mut lines, "owner-kind")? {
            "systemd-scope" => LifecycleOwnerKind::SystemdScope,
            "process-group" => LifecycleOwnerKind::ProcessGroup,
            _ => return Err("lifecycle lease owner kind is malformed".into()),
        };
        let unit = record_value(&mut lines, "unit")?;
        let invocation = record_value(&mut lines, "unit-invocation")?;
        let process_group = canonical_u32(
            record_value(&mut lines, "process-group")?,
            "lifecycle lease process group",
        )?;
        let cgroup = record_value(&mut lines, "cgroup")?;
        let cgroup_device = canonical_u64(
            record_value(&mut lines, "cgroup-device")?,
            "lifecycle lease cgroup device",
        )?;
        let cgroup_inode = canonical_u64(
            record_value(&mut lines, "cgroup-inode")?,
            "lifecycle lease cgroup inode",
        )?;
        if lines.next().is_some() {
            return Err("lifecycle lease has extra records".into());
        }
        match owner_kind {
            LifecycleOwnerKind::SystemdScope => {
                if process_group != 0
                    || unit != format!("helm-launch-{}.scope", launch.as_str())
                    || !lower_hex_32(invocation)
                    || !normalized_cgroup_path(cgroup)
                    || cgroup_device == 0
                    || cgroup_inode == 0
                {
                    return Err("systemd lifecycle lease identity is malformed".into());
                }
            }
            LifecycleOwnerKind::ProcessGroup => {
                if unit != "none"
                    || invocation != "none"
                    || process_group != pid
                    || cgroup != "none"
                    || cgroup_device != 0
                    || cgroup_inode != 0
                {
                    return Err("process group lifecycle lease identity is malformed".into());
                }
            }
        }
        Ok(Self {
            generation,
            launch,
            pid,
            start_time,
            boot_id: boot_id.into(),
            owner_uid,
            owner_kind,
            unit: unit.into(),
            unit_invocation: invocation.into(),
            process_group,
            cgroup: cgroup.into(),
            cgroup_device,
            cgroup_inode,
        })
    }
}

impl PublicationOrder {
    fn empty() -> Self {
        Self {
            next_sequence: 1,
            sequences: BTreeMap::new(),
        }
    }

    fn parse(raw: &[u8]) -> std::result::Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|_| "publication order is not UTF-8")?;
        if text.contains('\r') || !text.ends_with('\n') {
            return Err("publication order must use LF-terminated lines".into());
        }
        let mut lines = text[..text.len() - 1].split('\n');
        if lines.next() != Some("helm-generation-publication-order-v1") {
            return Err("unsupported publication order version".into());
        }
        let next_sequence = canonical_positive_u64(
            record_value(&mut lines, "next-sequence")?,
            "publication next sequence",
        )?;
        let mut sequences = BTreeMap::new();
        let mut used_sequences = BTreeSet::new();
        let mut previous: Option<GenerationId> = None;
        for line in lines {
            let mut fields = line.split(' ');
            if fields.next() != Some("generation") {
                return Err("expected publication generation record".into());
            }
            let generation = GenerationId::parse(
                fields
                    .next()
                    .ok_or("publication generation record lacks an identifier")?,
            )?;
            let sequence = canonical_positive_u64(
                fields
                    .next()
                    .ok_or("publication generation record lacks a sequence")?,
                "publication sequence",
            )?;
            if fields.next().is_some() {
                return Err("publication generation record has extra fields".into());
            }
            if previous.as_ref().is_some_and(|prior| prior >= &generation) {
                return Err("publication generation records are not strictly ordered".into());
            }
            if sequence >= next_sequence {
                return Err("publication sequence is not below next sequence".into());
            }
            if !used_sequences.insert(sequence) {
                return Err("publication sequence is duplicated".into());
            }
            previous = Some(generation.clone());
            if sequences.insert(generation, sequence).is_some() {
                return Err("publication generation is duplicated".into());
            }
        }
        Ok(Self {
            next_sequence,
            sequences,
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut encoded = format!(
            "helm-generation-publication-order-v1\nnext-sequence {}\n",
            self.next_sequence
        );
        for (generation, sequence) in &self.sequences {
            encoded.push_str(&format!("generation {} {sequence}\n", generation.as_str()));
        }
        encoded.into_bytes()
    }

    fn reserve(&mut self) -> std::result::Result<u64, String> {
        let reserved = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or("publication sequence space is exhausted")?;
        Ok(reserved)
    }
}

impl GenerationRoot {
    fn open(root: &Path) -> std::result::Result<Self, String> {
        let fd = open_directory_chain(root)?;
        Ok(Self { fd })
    }

    fn open_parent(&self, target: &Path) -> std::result::Result<(OwnedFd, OsString), String> {
        let final_name = target
            .file_name()
            .ok_or("manifest path lacks a filename")?
            .to_owned();
        let mut parent_fd = self.fd.try_clone().map_err(|error| error.to_string())?;
        let directory_flags =
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;

        for component in target
            .parent()
            .ok_or("manifest path lacks a parent")?
            .components()
        {
            let Component::Normal(component) = component else {
                return Err("unsafe manifest path".into());
            };
            parent_fd = openat(&parent_fd, component, directory_flags, Mode::empty())
                .map_err(|error| error.to_string())?;
        }
        Ok((parent_fd, final_name))
    }
}

/// Open a directory pathname a component at a time, never following a symlink.
pub(crate) fn open_directory_chain(path: &Path) -> std::result::Result<OwnedFd, String> {
    open_directory_chain_no_follow(path).map_err(|error| error.to_string())
}

/// Open a directory pathname a component at a time, preserving the errno for
/// callers that distinguish absence from an unsafe existing entry.
pub(crate) fn open_directory_chain_no_follow(path: &Path) -> std::result::Result<OwnedFd, Errno> {
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut fd = match path.components().next() {
        Some(Component::RootDir) => openat(CWD, "/", directory_flags, Mode::empty()),
        Some(Component::Prefix(_)) => return Err(Errno::INVAL),
        _ => openat(CWD, ".", directory_flags, Mode::empty()),
    }?;

    for component in path.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::RootDir | Component::CurDir) {
                continue;
            }
            return Err(Errno::INVAL);
        };
        fd = openat(&fd, component, directory_flags, Mode::empty())?;
    }
    Ok(fd)
}

/// Open one manifest file without following any component below `root`.
fn open_regular_file(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<std::fs::File, String> {
    open_optional_regular_file(root, target)?.ok_or_else(|| "manifest target is absent".into())
}

fn open_optional_regular_file(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<Option<std::fs::File>, String> {
    let (parent_fd, final_name) = root.open_parent(target)?;
    let fd = match openat(
        &parent_fd,
        &final_name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let file = std::fs::File::from(fd);
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("manifest target is not a regular file".into());
    }
    Ok(Some(file))
}

fn write_synced_file<F: PublicationFilesystem>(
    parent: &OwnedFd,
    name: &str,
    bytes: &[u8],
    filesystem: &mut F,
) -> std::result::Result<(), String> {
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|error| error.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes).map_err(|error| error.to_string())?;
    filesystem.sync(file.as_fd())
}

fn write_output<F: PublicationFilesystem>(
    staging: &GenerationRoot,
    path: &Path,
    bytes: &[u8],
    filesystem: &mut F,
) -> std::result::Result<(), String> {
    let final_name = path.file_name().ok_or("output path lacks a filename")?;
    let mut parent = staging.fd.try_clone().map_err(|error| error.to_string())?;
    for component in path
        .parent()
        .ok_or("output path lacks a parent")?
        .components()
    {
        let Component::Normal(component) = component else {
            return Err("unsafe output path".into());
        };
        match mkdirat(&parent, component, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
            Ok(()) => filesystem.sync(parent.as_fd())?,
            Err(Errno::EXIST) => {}
            Err(error) => return Err(error.to_string()),
        }
        parent = openat(
            &parent,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| error.to_string())?;
    }
    let name = final_name.to_str().ok_or("output filename must be UTF-8")?;
    write_synced_file(&parent, name, bytes, filesystem)?;
    filesystem.sync(parent.as_fd())
}

fn remove_tree_at(parent: &OwnedFd, name: &OsStr) -> std::result::Result<(), String> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};

    match openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(child) => {
            let mut entries = Vec::new();
            let mut directory = Dir::read_from(&child).map_err(|error| error.to_string())?;
            while let Some(entry) = directory.read() {
                let entry = entry.map_err(|error| error.to_string())?;
                let bytes = entry.file_name().to_bytes();
                if !matches!(bytes, b"." | b"..") {
                    entries.push(OsString::from_vec(bytes.to_vec()));
                }
            }
            drop(directory);
            for entry in entries {
                remove_tree_at(&child, &entry)?;
            }
            fsync(&child).map_err(|error| error.to_string())?;
            unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(|error| error.to_string())
        }
        Err(Errno::NOTDIR | Errno::LOOP) => {
            unlinkat(parent, OsStr::from_bytes(name.as_bytes()), AtFlags::empty())
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

/// An opaque, filesystem-safe generation identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GenerationId(String);

impl GenerationId {
    /// Parse the stable ASCII identifier stored in `current`.
    pub fn parse(value: &str) -> std::result::Result<Self, String> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err("generation id must be 128-bit lowercase hex".into());
        }
        Ok(Self(value.into()))
    }

    /// Return the canonical lowercase-hex spelling.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl GenerationPublication {
    fn receipt(&self, generation: &GenerationId) -> String {
        format!(
            "helm-generation-receipt-v1\ngeneration {}\npalette-sha256 {}\ncatalogue-sha256 {}\ntemplates-sha256 {}\nrenderer-sha256 {}\nlaunch-profile-sha256 {}\n",
            generation.as_str(),
            self.input_digests[0],
            self.input_digests[1],
            self.input_digests[2],
            self.input_digests[3],
            self.input_digests[4],
        )
    }

    fn manifest(&self, generation: &GenerationId, receipt: &str) -> String {
        let mut manifest = format!(
            "helm-generation-manifest-v1\ngeneration {}\npalette-sha256 {}\ncatalogue-sha256 {}\ntemplates-sha256 {}\nrenderer-sha256 {}\nlaunch-profile-sha256 {}\nreceipt-sha256 {}\n",
            generation.as_str(),
            self.input_digests[0],
            self.input_digests[1],
            self.input_digests[2],
            self.input_digests[3],
            self.input_digests[4],
            digest_hex(receipt.as_bytes()),
        );
        for (path, bytes) in &self.outputs {
            manifest.push_str(&format!("output {path} {}\n", digest_hex(bytes)));
        }
        manifest
    }
}

/// One validated canonical manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    entries: Vec<(String, String)>,
    v1: V1Manifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct V1Manifest {
    generation: GenerationId,
    input_digests: [String; 5],
    receipt_digest: String,
    raw: String,
}

impl Manifest {
    /// Parse the canonical v1 manifest encoding.
    pub fn parse(input: &str) -> std::result::Result<Self, String> {
        Self::parse_v1(input)
    }

    fn parse_v1(input: &str) -> std::result::Result<Self, String> {
        if input.contains('\r') || !input.ends_with('\n') {
            return Err("v1 manifest must use LF-terminated lines".into());
        }
        let mut lines = input[..input.len() - 1].split('\n');
        if lines.next() != Some("helm-generation-manifest-v1") {
            return Err("unsupported manifest version".into());
        }
        let generation = GenerationId::parse(record_value(&mut lines, "generation")?)?;
        let field_names = [
            "palette-sha256",
            "catalogue-sha256",
            "templates-sha256",
            "renderer-sha256",
            "launch-profile-sha256",
        ];
        let mut input_digests = Vec::new();
        for field in field_names {
            let digest = record_value(&mut lines, field)?;
            if !sha256_digest(digest) {
                return Err("manifest digest must be lowercase SHA-256 hex".into());
            }
            input_digests.push(digest.into());
        }
        let receipt_digest = record_value(&mut lines, "receipt-sha256")?;
        if !sha256_digest(receipt_digest) {
            return Err("manifest digest must be lowercase SHA-256 hex".into());
        }
        let mut entries = Vec::new();
        let mut previous = None;
        for line in lines {
            let Some(rest) = line.strip_prefix("output ") else {
                return Err("unexpected manifest record".into());
            };
            let (path, digest) = rest.split_once(' ').ok_or("output record lacks digest")?;
            if path.is_empty() || !safe_output_path(path) || !sha256_digest(digest) {
                return Err("unsafe manifest output record".into());
            }
            if previous.is_some_and(|last: &str| last >= path) {
                return Err("manifest paths must be unique and sorted".into());
            }
            previous = Some(path);
            entries.push((path.into(), digest.into()));
        }
        if entries.is_empty() {
            return Err("v1 manifest requires an output record".into());
        }
        let input_digests: [String; 5] = input_digests
            .try_into()
            .map_err(|_| "v1 manifest requires five input digests")?;
        Ok(Self {
            entries,
            v1: V1Manifest {
                generation,
                input_digests,
                receipt_digest: receipt_digest.into(),
                raw: input.into(),
            },
        })
    }

    /// Verify every manifest-listed regular file against its SHA-256 digest.
    pub fn verify(&self, root_path: &Path) -> std::result::Result<(), String> {
        let root = GenerationRoot::open(root_path)?;
        let generation = root_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("generation directory lacks a UTF-8 identity")?;
        let generation = GenerationId::parse(generation)?;
        self.verify_opened(&root, &generation)
    }

    fn verify_opened(
        &self,
        root: &GenerationRoot,
        generation: &GenerationId,
    ) -> std::result::Result<(), String> {
        verify_v1_metadata(root, generation, &self.v1)?;
        validate_v1_tree(root, &self.entries)?;
        for (path, expected) in &self.entries {
            let mut file = open_regular_file(root, Path::new(path))?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            let actual = format!("{:x}", Sha256::digest(bytes));
            if &actual != expected {
                return Err("manifest digest mismatch".into());
            }
        }
        Ok(())
    }
}

fn validate_v1_tree(
    root: &GenerationRoot,
    entries: &[(String, String)],
) -> std::result::Result<(), String> {
    let mut files = BTreeSet::from([
        String::from("manifest"),
        String::from("receipt"),
        String::from("seal"),
    ]);
    let mut directories = BTreeSet::new();
    for (path, _) in entries {
        files.insert(path.clone());
        let mut prefix = String::new();
        let mut components = path.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            directories.insert(prefix.clone());
        }
    }
    validate_directory_entries(&root.fd, "", &files, &directories)
}

fn validate_directory_entries(
    fd: &OwnedFd,
    prefix: &str,
    files: &BTreeSet<String>,
    directories: &BTreeSet<String>,
) -> std::result::Result<(), String> {
    let mut directory = Dir::read_from(fd).map_err(|error| error.to_string())?;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        let name = std::str::from_utf8(name).map_err(|_| "non-UTF-8 tree entry")?;
        let path = if prefix.is_empty() {
            name.into()
        } else {
            format!("{prefix}/{name}")
        };
        if files.contains(&path) {
            let child = openat(
                fd,
                name,
                OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            if !std::fs::File::from(child)
                .metadata()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                return Err("sealed tree contains a non-regular listed file".into());
            }
        } else if directories.contains(&path) {
            let child = openat(
                fd,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|error| error.to_string())?;
            validate_directory_entries(&child, &path, files, directories)?;
        } else {
            return Err("sealed tree contains an unlisted entry".into());
        }
    }
    Ok(())
}

fn verify_v1_metadata(
    root: &GenerationRoot,
    generation: &GenerationId,
    v1: &V1Manifest,
) -> std::result::Result<(), String> {
    if generation != &v1.generation {
        return Err("generation directory does not match manifest identity".into());
    }
    let manifest = read_regular_bytes(root, Path::new("manifest"))?;
    if manifest != v1.raw.as_bytes() {
        return Err("manifest bytes changed after parsing".into());
    }
    let receipt = read_regular_bytes(root, Path::new("receipt"))?;
    if digest_hex(&receipt) != v1.receipt_digest {
        return Err("receipt digest mismatch".into());
    }
    let expected_receipt = format!(
        "helm-generation-receipt-v1\ngeneration {}\npalette-sha256 {}\ncatalogue-sha256 {}\ntemplates-sha256 {}\nrenderer-sha256 {}\nlaunch-profile-sha256 {}\n",
        v1.generation.0,
        v1.input_digests[0],
        v1.input_digests[1],
        v1.input_digests[2],
        v1.input_digests[3],
        v1.input_digests[4],
    );
    if receipt != expected_receipt.as_bytes() {
        return Err("receipt does not match manifest inputs".into());
    }
    let seal = read_regular_bytes(root, Path::new("seal"))?;
    if seal != format!("{}\n", digest_hex(v1.raw.as_bytes())).as_bytes() {
        return Err("manifest seal mismatch".into());
    }
    Ok(())
}

fn verify_opened_generation(
    root: &GenerationRoot,
    generation: &GenerationId,
) -> std::result::Result<Manifest, String> {
    let bytes = read_regular_bytes(root, Path::new("manifest"))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "manifest is not UTF-8")?;
    let manifest = Manifest::parse(text)?;
    manifest.verify_opened(root, generation)?;
    Ok(manifest)
}

fn read_regular_bytes(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<Vec<u8>, String> {
    let mut file = open_regular_file(root, target)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn read_optional_regular_bytes(
    root: &GenerationRoot,
    target: &Path,
) -> std::result::Result<Option<Vec<u8>>, String> {
    let Some(mut file) = open_optional_regular_file(root, target)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(Some(bytes))
}

fn parse_pointer_record(bytes: &[u8], name: &str) -> std::result::Result<GenerationId, String> {
    let record = std::str::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8"))?;
    let value = record
        .strip_suffix('\n')
        .ok_or_else(|| format!("{name} must end in one LF"))?;
    GenerationId::parse(value)
}

fn canonical_u32(value: &str, field: &str) -> std::result::Result<u32, String> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return Err(format!("{field} is not canonical decimal"));
    }
    value
        .parse()
        .map_err(|_| format!("{field} is not canonical decimal"))
}

fn canonical_u64(value: &str, field: &str) -> std::result::Result<u64, String> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(format!("{field} is not canonical decimal"));
    }
    value
        .parse()
        .map_err(|_| format!("{field} is not canonical decimal"))
}

fn canonical_positive_u64(value: &str, field: &str) -> std::result::Result<u64, String> {
    let value = canonical_u64(value, field)?;
    if value == 0 {
        return Err(format!("{field} must be positive"));
    }
    Ok(value)
}

fn canonical_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                matches!(byte, b'0'..=b'9' | b'a'..=b'f')
            }
        })
}

fn lower_hex_32(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn normalized_cgroup_path(value: &str) -> bool {
    !value.is_empty()
        && value != "none"
        && value
            .split('/')
            .all(|component| !component.is_empty()
                && !matches!(component, "." | "..")
                && component.bytes().all(|byte| matches!(byte,
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':' | b'@' | b'%' | b'+' | b'-')))
}

fn linux_boot_id() -> std::result::Result<String, String> {
    let root = GenerationRoot::open(Path::new("/proc/sys/kernel/random"))?;
    let raw = read_regular_bytes(&root, Path::new("boot_id"))?;
    let text = std::str::from_utf8(&raw).map_err(|_| "Linux boot ID is not UTF-8")?;
    let value = text
        .strip_suffix('\n')
        .ok_or("Linux boot ID is not LF-terminated")?;
    if !canonical_boot_id(value) {
        return Err("Linux boot ID is malformed".into());
    }
    Ok(value.into())
}

fn linux_process_identity(pid: u32) -> std::result::Result<(u64, u32), String> {
    let proc_root = open_directory_chain(Path::new("/proc"))?;
    let process = openat(
        &proc_root,
        pid.to_string(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let stat = rustix::fs::fstat(&process).map_err(|error| error.to_string())?;
    Ok((linux_process_start_time_from(&process)?, stat.st_uid))
}

fn linux_process_start_time_from(process: &OwnedFd) -> std::result::Result<u64, String> {
    let root = GenerationRoot {
        fd: process.try_clone().map_err(|error| error.to_string())?,
    };
    let raw = read_regular_bytes(&root, Path::new("stat"))?;
    let text = std::str::from_utf8(&raw).map_err(|_| "Linux process stat is not UTF-8")?;
    let fields = text
        .rsplit_once(") ")
        .ok_or("Linux process stat lacks a command terminator")?
        .1;
    fields
        .split_ascii_whitespace()
        .nth(19)
        .ok_or("Linux process stat lacks start time")?
        .parse()
        .map_err(|_| "Linux process start time is malformed".into())
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn random_generation_id() -> std::result::Result<GenerationId, String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| error.to_string())?;
    let mut encoded = String::with_capacity(32);
    for byte in random {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    GenerationId::parse(&encoded)
}

fn record_value<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
    name: &str,
) -> std::result::Result<&'a str, String> {
    let line = lines
        .next()
        .ok_or("manifest ended before required record")?;
    line.strip_prefix(&format!("{name} "))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("expected {name} record"))
}

fn safe_output_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .bytes()
            .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'/'))
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains("//")
        && path.split('/').all(|component| {
            !matches!(component, "." | ".." | "manifest" | "receipt" | "seal" | "activation.lock")
                && !component.starts_with(".staging-")
        })
}

fn sha256_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::BufRead;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, UNIX_EPOCH};

    #[derive(Debug, PartialEq, Eq)]
    enum ActualFilesystemEvent {
        Sync(PathBuf),
        Rename {
            from_directory: PathBuf,
            from_name: String,
            to_directory: PathBuf,
            to_name: String,
        },
        Exchange {
            left_directory: PathBuf,
            left_name: String,
            right_directory: PathBuf,
            right_name: String,
        },
        Unlink(PathBuf),
    }

    #[derive(Default)]
    struct RecordingPublicationFilesystem {
        events: Vec<ActualFilesystemEvent>,
    }

    struct FailOnceOnRootSyncFilesystem {
        root: PathBuf,
        fail_on: usize,
        root_syncs: usize,
        failed: bool,
        fail_unlink: bool,
    }

    struct FailAfterMappedPublicationOrderFilesystem {
        root: PathBuf,
        publication_order_renames: usize,
        failed: bool,
    }

    impl RecordingPublicationFilesystem {
        fn descriptor_path(fd: BorrowedFd<'_>) -> std::result::Result<PathBuf, String> {
            std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd()))
                .map_err(|error| error.to_string())
        }
    }

    impl PublicationFilesystem for RecordingPublicationFilesystem {
        fn sync(&mut self, fd: BorrowedFd<'_>) -> std::result::Result<(), String> {
            fsync(fd).map_err(|error| error.to_string())?;
            self.events
                .push(ActualFilesystemEvent::Sync(Self::descriptor_path(fd)?));
            Ok(())
        }

        fn rename(
            &mut self,
            from_directory: BorrowedFd<'_>,
            from_name: &str,
            to_directory: BorrowedFd<'_>,
            to_name: &str,
        ) -> std::result::Result<(), String> {
            renameat(from_directory, from_name, to_directory, to_name)
                .map_err(|error| error.to_string())?;
            self.events.push(ActualFilesystemEvent::Rename {
                from_directory: Self::descriptor_path(from_directory)?,
                from_name: from_name.into(),
                to_directory: Self::descriptor_path(to_directory)?,
                to_name: to_name.into(),
            });
            Ok(())
        }

        fn exchange(
            &mut self,
            left_directory: BorrowedFd<'_>,
            left_name: &str,
            right_directory: BorrowedFd<'_>,
            right_name: &str,
        ) -> std::result::Result<(), String> {
            renameat_with(
                left_directory,
                left_name,
                right_directory,
                right_name,
                RenameFlags::EXCHANGE,
            )
            .map_err(|error| error.to_string())?;
            self.events.push(ActualFilesystemEvent::Exchange {
                left_directory: Self::descriptor_path(left_directory)?,
                left_name: left_name.into(),
                right_directory: Self::descriptor_path(right_directory)?,
                right_name: right_name.into(),
            });
            Ok(())
        }

        fn unlink(
            &mut self,
            directory: BorrowedFd<'_>,
            name: &str,
        ) -> std::result::Result<(), String> {
            let target = Self::descriptor_path(directory)?.join(name);
            unlinkat(directory, name, AtFlags::empty()).map_err(|error| error.to_string())?;
            self.events.push(ActualFilesystemEvent::Unlink(target));
            Ok(())
        }
    }

    impl PublicationFilesystem for FailOnceOnRootSyncFilesystem {
        fn sync(&mut self, fd: BorrowedFd<'_>) -> std::result::Result<(), String> {
            let target = RecordingPublicationFilesystem::descriptor_path(fd)?;
            if target == self.root {
                self.root_syncs += 1;
                if self.root_syncs == self.fail_on && !self.failed {
                    self.failed = true;
                    return Err("injected generated-root fsync failure".into());
                }
            }
            fsync(fd).map_err(|error| error.to_string())
        }

        fn rename(
            &mut self,
            from_directory: BorrowedFd<'_>,
            from_name: &str,
            to_directory: BorrowedFd<'_>,
            to_name: &str,
        ) -> std::result::Result<(), String> {
            renameat(from_directory, from_name, to_directory, to_name)
                .map_err(|error| error.to_string())
        }

        fn exchange(
            &mut self,
            left_directory: BorrowedFd<'_>,
            left_name: &str,
            right_directory: BorrowedFd<'_>,
            right_name: &str,
        ) -> std::result::Result<(), String> {
            renameat_with(
                left_directory,
                left_name,
                right_directory,
                right_name,
                RenameFlags::EXCHANGE,
            )
            .map_err(|error| error.to_string())
        }

        fn unlink(
            &mut self,
            directory: BorrowedFd<'_>,
            name: &str,
        ) -> std::result::Result<(), String> {
            if self.fail_unlink {
                self.failed = true;
                return Err("injected committed-marker unlink failure".into());
            }
            unlinkat(directory, name, AtFlags::empty()).map_err(|error| error.to_string())
        }
    }

    impl PublicationFilesystem for FailAfterMappedPublicationOrderFilesystem {
        fn sync(&mut self, fd: BorrowedFd<'_>) -> std::result::Result<(), String> {
            fsync(fd).map_err(|error| error.to_string())?;
            if RecordingPublicationFilesystem::descriptor_path(fd)? == self.root
                && self.publication_order_renames == 2
                && !self.failed
            {
                self.failed = true;
                return Err("simulated death after mapped publication order became durable".into());
            }
            Ok(())
        }

        fn rename(
            &mut self,
            from_directory: BorrowedFd<'_>,
            from_name: &str,
            to_directory: BorrowedFd<'_>,
            to_name: &str,
        ) -> std::result::Result<(), String> {
            renameat(from_directory, from_name, to_directory, to_name)
                .map_err(|error| error.to_string())?;
            if from_name == ".publication-order" && to_name == "publication-order" {
                self.publication_order_renames += 1;
            }
            Ok(())
        }

        fn exchange(
            &mut self,
            left_directory: BorrowedFd<'_>,
            left_name: &str,
            right_directory: BorrowedFd<'_>,
            right_name: &str,
        ) -> std::result::Result<(), String> {
            renameat_with(
                left_directory,
                left_name,
                right_directory,
                right_name,
                RenameFlags::EXCHANGE,
            )
            .map_err(|error| error.to_string())
        }

        fn unlink(
            &mut self,
            directory: BorrowedFd<'_>,
            name: &str,
        ) -> std::result::Result<(), String> {
            unlinkat(directory, name, AtFlags::empty()).map_err(|error| error.to_string())
        }
    }

    fn publication(_generation: &str, marker: &str) -> GenerationPublication {
        GenerationPublication::new(
            [
                digest(format!("{marker}-palette").as_bytes()),
                digest(format!("{marker}-catalogue").as_bytes()),
                digest(format!("{marker}-templates").as_bytes()),
                digest(format!("{marker}-renderer").as_bytes()),
                digest(format!("{marker}-launch-profile").as_bytes()),
            ],
            vec![(String::from("theme.ini"), marker.as_bytes().to_vec())],
        )
        .unwrap()
    }

    fn publication_with_outputs(marker: &str, outputs: &[(&str, &str)]) -> GenerationPublication {
        GenerationPublication::new(
            [
                digest(format!("{marker}-palette").as_bytes()),
                digest(format!("{marker}-catalogue").as_bytes()),
                digest(format!("{marker}-templates").as_bytes()),
                digest(format!("{marker}-renderer").as_bytes()),
                digest(format!("{marker}-launch-profile").as_bytes()),
            ],
            outputs
                .iter()
                .map(|(path, bytes)| ((*path).into(), bytes.as_bytes().to_vec()))
                .collect(),
        )
        .unwrap()
    }

    fn publish_at<C, H>(
        store: &GenerationStore,
        generation: &str,
        capture: C,
        checkpoint: H,
    ) -> std::result::Result<GenerationPublicationOutcome, GenerationPublicationError>
    where
        C: FnOnce() -> std::result::Result<GenerationPublication, String>,
        H: FnMut(CommitCheckpoint) -> std::result::Result<(), String>,
    {
        let generation = GenerationId::parse(generation)?;
        store.publish_with_checkpoint_and_ids(capture, checkpoint, move || Ok(generation.clone()))
    }

    fn secure_store_root(root: &Path) {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn seeded_store(root: &Path) -> GenerationStore {
        secure_store_root(root);
        std::fs::create_dir(root.join("generations")).unwrap();
        std::fs::set_permissions(
            root.join("generations"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let old = "00000000000000000000000000000001";
        v1_fixture(&root.join("generations"), old, "theme.ini", b"old");
        std::fs::write(root.join("current"), format!("{old}\n")).unwrap();
        write_publication_order_fixture(
            root,
            "helm-generation-publication-order-v1\nnext-sequence 2\ngeneration 00000000000000000000000000000001 1\n",
        );
        GenerationStore::open(root).unwrap()
    }

    fn write_publication_order_fixture(root: &Path, contents: &str) {
        let path = root.join("publication-order");
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn set_generation_modified(root: &Path, generation: &str, seconds: u64) {
        let directory = std::fs::File::open(root.join("generations").join(generation)).unwrap();
        let modified = UNIX_EPOCH + Duration::from_secs(seconds);
        directory
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    fn lease_names(root: &Path) -> Vec<OsString> {
        let mut names: Vec<_> = std::fs::read_dir(root.join("leases"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }

    fn linux_boot_id() -> String {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim_end()
            .to_owned()
    }

    fn linux_start_time(pid: u32) -> u64 {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let after_comm = stat.rsplit_once(") ").unwrap().1;
        after_comm
            .split_ascii_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap()
    }

    fn write_lease_fixture(
        root: &Path,
        name: &str,
        generation: &str,
        pid: u32,
        start_time: u64,
        boot_id: &str,
        owner_uid: u32,
    ) {
        let path = root.join("leases").join(name);
        std::fs::write(
            &path,
            format!(
                "helm-generation-lease-v1\ngeneration {generation}\npid {pid}\nstart-time {start_time}\nboot-id {boot_id}\nowner-uid {owner_uid}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn write_raw_lease_fixture(root: &Path, name: &str, contents: &[u8]) {
        let path = root.join("leases").join(name);
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn lifecycle_direct_fixture(generation: &str) -> Vec<u8> {
        let launch = "00000000000000000000000000000002";
        let pid = std::process::id();
        let start_time = linux_start_time(pid);
        let boot_id = linux_boot_id();
        let owner_uid = rustix::process::getuid().as_raw();
        format!(
            "helm-generation-lifecycle-lease-v1\ngeneration {generation}\nlaunch {launch}\npid {pid}\nstart-time {start_time}\nboot-id {boot_id}\nowner-uid {owner_uid}\nowner-kind process-group\nunit none\nunit-invocation none\nprocess-group {pid}\ncgroup none\ncgroup-device 0\ncgroup-inode 0\n"
        )
        .into_bytes()
    }

    #[test]
    fn selection_drop_never_unlinks_a_replaced_lifecycle_lease() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let selection = store.select_current().unwrap();
        let name = selection.lease_name.clone();
        write_raw_lease_fixture(
            root.path(),
            &name,
            &lifecycle_direct_fixture(selection.as_str()),
        );
        drop(selection);
        let path = root.path().join("leases").join(name);
        assert!(path.exists());
        assert!(matches!(
            ParsedLeaseRecord::parse(&std::fs::read(path).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
    }

    #[test]
    fn selection_drop_never_unlinks_an_unsafe_process_lease() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let selection = store.select_current().unwrap();
        let name = selection.lease_name.clone();
        let path = root.path().join("leases").join(&name);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(selection);
        assert!(path.exists());
    }

    #[test]
    fn selection_cleanup_rejects_oversized_records_before_parsing() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let selection = store.select_current().unwrap();
        let name = selection.lease_name.clone();
        write_raw_lease_fixture(root.path(), &name, &vec![b'x'; 4097]);

        let error = selection.read_current_lease().unwrap_err();

        assert!(error.contains("exceeds 4096 bytes"), "{error}");
        assert!(root.path().join("leases").join(name).exists());
    }

    #[test]
    fn selection_cleanup_retains_a_process_lease_swapped_before_final_proof() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let mut selection = store.select_current().unwrap();
        let name = selection.lease_name.clone();
        let path = root.path().join("leases").join(&name);
        let displaced = root.path().join("displaced-selection-lease");
        let original = std::fs::read(&path).unwrap();

        selection
            .release_matching_process_lease_with_checkpoint(|| {
                std::fs::rename(&path, &displaced).unwrap();
                write_raw_lease_fixture(root.path(), &name, &original);
            })
            .unwrap();

        assert!(path.exists());
        assert!(displaced.exists());
    }

    #[test]
    fn selection_cleanup_holds_the_originating_generation_lock() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let competing_store = GenerationStore::open(root.path()).unwrap();
        let mut selection = store.select_current().unwrap();
        let competing_writer_acquired = std::cell::Cell::new(false);

        selection
            .release_matching_process_lease_with_checkpoint(|| {
                match flock(
                    &competing_store.lock,
                    FlockOperation::NonBlockingLockExclusive,
                ) {
                    Ok(()) => {
                        competing_writer_acquired.set(true);
                        flock(&competing_store.lock, FlockOperation::Unlock).unwrap();
                    }
                    Err(error) => assert_eq!(error, Errno::WOULDBLOCK),
                }
            })
            .unwrap();

        assert!(!competing_writer_acquired.get());
    }

    #[test]
    fn lifecycle_lease_parser_requires_canonical_direct_identity() {
        let generation = "00000000000000000000000000000001";
        let launch = "00000000000000000000000000000002";
        let pid = std::process::id();
        let start_time = linux_start_time(pid);
        let boot_id = linux_boot_id();
        let owner_uid = rustix::process::getuid().as_raw();
        let valid = format!(
            "helm-generation-lifecycle-lease-v1\ngeneration {generation}\nlaunch {launch}\npid {pid}\nstart-time {start_time}\nboot-id {boot_id}\nowner-uid {owner_uid}\nowner-kind process-group\nunit none\nunit-invocation none\nprocess-group {pid}\ncgroup none\ncgroup-device 0\ncgroup-inode 0\n"
        );

        let parsed = ParsedLeaseRecord::parse(valid.as_bytes()).unwrap();
        assert!(matches!(parsed, ParsedLeaseRecord::Lifecycle(_)));

        let zero_pid = valid.replacen(&format!("pid {pid}\n"), "pid 0\n", 1);
        assert!(ParsedLeaseRecord::parse(zero_pid.as_bytes()).is_err());

        let zero_start = valid.replacen(&format!("start-time {start_time}\n"), "start-time 0\n", 1);
        assert!(ParsedLeaseRecord::parse(zero_start.as_bytes()).is_err());

        let mismatched_group =
            valid.replacen(&format!("process-group {pid}\n"), "process-group 1\n", 1);
        assert!(ParsedLeaseRecord::parse(mismatched_group.as_bytes()).is_err());

        let systemd = format!(
            "helm-generation-lifecycle-lease-v1\ngeneration {generation}\nlaunch {launch}\npid {pid}\nstart-time {start_time}\nboot-id {boot_id}\nowner-uid {owner_uid}\nowner-kind systemd-scope\nunit helm-launch-{launch}.scope\nunit-invocation 00000000000000000000000000000000\nprocess-group 0\ncgroup user.slice/helm.scope\ncgroup-device 1\ncgroup-inode 1\n"
        );
        assert!(matches!(
            ParsedLeaseRecord::parse(systemd.as_bytes()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
        let systemd_with_direct_group =
            systemd.replacen("process-group 0\n", "process-group 1\n", 1);
        assert!(ParsedLeaseRecord::parse(systemd_with_direct_group.as_bytes()).is_err());

        let systemd_with_current_directory =
            systemd.replacen("cgroup user.slice/helm.scope\n", "cgroup .\n", 1);
        assert!(ParsedLeaseRecord::parse(systemd_with_current_directory.as_bytes()).is_err());

        let systemd_with_parent_directory = systemd.replacen(
            "cgroup user.slice/helm.scope\n",
            "cgroup user.slice/../helm.scope\n",
            1,
        );
        assert!(ParsedLeaseRecord::parse(systemd_with_parent_directory.as_bytes()).is_err());
    }

    #[test]
    fn g1_selected_old_generation_keeps_descriptor_pinned_bytes_after_new_commit() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = store.select_current().unwrap();
        assert_eq!(old.read_output("theme.ini").unwrap(), b"old");
        let leases = lease_names(root.path());
        assert_eq!(leases.len(), 1);
        assert_eq!(
            std::fs::read_to_string(root.path().join("leases").join(&leases[0])).unwrap(),
            format!(
                "helm-generation-lease-v1\ngeneration 00000000000000000000000000000001\npid {}\nstart-time {}\nboot-id {}\nowner-uid {}\n",
                std::process::id(),
                linux_start_time(std::process::id()),
                linux_boot_id(),
                rustix::process::getuid().as_raw(),
            )
        );

        let candidate = "00000000000000000000000000000002";
        publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "new")),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(old.as_str(), "00000000000000000000000000000001");
        assert_eq!(old.read_output("theme.ini").unwrap(), b"old");
        assert_eq!(store.select_current().unwrap().as_str(), candidate);
        drop(old);
        assert!(lease_names(root.path()).is_empty());
    }

    #[test]
    fn g5_invalid_current_refuses_without_creating_a_lease() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        std::fs::write(
            root.path()
                .join("generations/00000000000000000000000000000001/theme.ini"),
            b"tampered",
        )
        .unwrap();

        assert!(store.select_current().is_err());
        assert!(lease_names(root.path()).is_empty());
    }

    #[test]
    fn os_advisory_shared_lock_blocks_separately_opened_writer_through_durable_lease_creation() {
        use std::sync::Barrier;

        let root = tempfile::tempdir().unwrap();
        let selecting_store = Arc::new(seeded_store(root.path()));
        let publishing_store = Arc::new(GenerationStore::open(root.path()).unwrap());
        let lease_synced = Arc::new(Barrier::new(2));
        let release_selector = Arc::new(Barrier::new(2));
        let selecting_store = Arc::clone(&selecting_store);
        let selecting_lease_synced = Arc::clone(&lease_synced);
        let selecting_release = Arc::clone(&release_selector);
        let selecting = std::thread::spawn(move || {
            selecting_store
                .select_current_with_checkpoint(std::process::id(), || {
                    selecting_lease_synced.wait();
                    selecting_release.wait();
                })
                .unwrap()
        });
        lease_synced.wait();
        assert_eq!(lease_names(root.path()).len(), 1);

        let exclusive_attempt = flock(
            &publishing_store.lock,
            FlockOperation::NonBlockingLockExclusive,
        );
        if exclusive_attempt.is_ok() {
            flock(&publishing_store.lock, FlockOperation::Unlock).unwrap();
        }

        release_selector.wait();
        let selected = selecting.join().unwrap();
        assert_eq!(selected.as_str(), "00000000000000000000000000000001");
        assert_eq!(exclusive_attempt.unwrap_err(), Errno::WOULDBLOCK);

        let candidate = "00000000000000000000000000000002";
        assert!(publish_at(
            &publishing_store,
            candidate,
            || Ok(publication(candidate, "new")),
            |_| Ok(()),
        )
        .is_ok());
    }

    #[test]
    fn generation_diff_holds_shared_lock_through_validation_and_comparison() {
        fn assert_writer_is_blocked(lock: &OwnedFd) {
            let exclusive_attempt = flock(lock, FlockOperation::NonBlockingLockExclusive);
            if exclusive_attempt.is_ok() {
                flock(lock, FlockOperation::Unlock).unwrap();
            }
            assert_eq!(exclusive_attempt.unwrap_err(), Errno::WOULDBLOCK);
        }

        let root = tempfile::tempdir().unwrap();
        let publishing_store = seeded_store(root.path());
        publish_at(
            &publishing_store,
            "00000000000000000000000000000002",
            || {
                Ok(publication_with_outputs(
                    "current",
                    &[("a.ini", "old-a"), ("z.ini", "old-z")],
                ))
            },
            |_| Ok(()),
        )
        .unwrap();
        let reader_root = GenerationRoot::open(root.path()).unwrap();
        let reader = GenerationReader::open_from_fd(reader_root.fd).unwrap();
        let candidate =
            publication_with_outputs("candidate", &[("a.ini", "new-a"), ("c.ini", "added")]);
        let observed = std::cell::RefCell::new(Vec::new());
        let old_comparison = reader
            .diff_current_with_interval_checkpoints(
                candidate.outputs(),
                || {
                    observed.borrow_mut().push(String::from("locked"));
                    assert_writer_is_blocked(&publishing_store.lock);
                },
                |path| {
                    observed.borrow_mut().push(format!("read:{path}"));
                    assert_writer_is_blocked(&publishing_store.lock);
                },
                || {
                    observed.borrow_mut().push(String::from("result"));
                    assert_writer_is_blocked(&publishing_store.lock);
                },
            )
            .unwrap();

        assert_eq!(
            observed.into_inner(),
            vec!["locked", "read:a.ini", "read:z.ini", "result"]
        );
        assert_eq!(
            old_comparison,
            vec![
                (String::from("a.ini"), OutputDifference::ByteDifferent),
                (String::from("c.ini"), OutputDifference::Added),
                (String::from("z.ini"), OutputDifference::Removed),
            ]
        );
        flock(
            &publishing_store.lock,
            FlockOperation::NonBlockingLockExclusive,
        )
        .unwrap();
        flock(&publishing_store.lock, FlockOperation::Unlock).unwrap();

        publish_at(
            &publishing_store,
            "00000000000000000000000000000003",
            || {
                Ok(publication_with_outputs(
                    "candidate",
                    &[("a.ini", "new-a"), ("c.ini", "added")],
                ))
            },
            |_| Ok(()),
        )
        .unwrap();
        let candidate =
            publication_with_outputs("candidate", &[("a.ini", "new-a"), ("c.ini", "added")]);
        assert!(reader.diff_current(candidate.outputs()).unwrap().is_empty());
    }

    #[test]
    fn g7_rollback_changes_future_selection_without_changing_running_selection() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = GenerationId::parse("00000000000000000000000000000001").unwrap();
        let candidate = "00000000000000000000000000000002";
        publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "new")),
            |_| Ok(()),
        )
        .unwrap();
        let running = store.select_current().unwrap();

        store.rollback(&old).unwrap();

        assert_eq!(running.as_str(), candidate);
        assert_eq!(running.read_output("theme.ini").unwrap(), b"new");
        assert_eq!(store.select_current().unwrap().as_str(), old.as_str());
    }

    #[test]
    fn g7_rollback_refuses_a_malformed_retained_generation_without_changing_current() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = GenerationId::parse("00000000000000000000000000000001").unwrap();
        let candidate = "00000000000000000000000000000002";
        publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "new")),
            |_| Ok(()),
        )
        .unwrap();
        std::fs::write(
            root.path()
                .join("generations")
                .join(old.as_str())
                .join("theme.ini"),
            b"tampered",
        )
        .unwrap();

        assert!(store.rollback(&old).is_err());
        assert_eq!(store.select_current().unwrap().as_str(), candidate);
    }

    #[test]
    fn g6_gc_removes_actual_pid_boot_and_start_time_mismatch_leases() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let generation = "00000000000000000000000000000001";
        let pid = std::process::id();
        let start_time = linux_start_time(pid);
        let boot_id = linux_boot_id();
        let owner_uid = rustix::process::getuid().as_raw();
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000011",
            generation,
            i32::MAX as u32,
            start_time,
            &boot_id,
            owner_uid,
        );
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000012",
            generation,
            pid,
            start_time,
            "00000000-0000-0000-0000-000000000000",
            owner_uid,
        );
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000013",
            generation,
            pid,
            start_time + 1,
            &boot_id,
            owner_uid,
        );

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 3);
        assert!(lease_names(root.path()).is_empty());
        assert!(root.path().join("generations").join(generation).exists());
    }

    #[test]
    fn g6_gc_retains_stale_process_lease_when_lifecycle_evidence_is_malformed() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let generation = "00000000000000000000000000000001";
        let generations = [
            generation,
            "00000000000000000000000000000002",
            "00000000000000000000000000000003",
            "00000000000000000000000000000004",
        ];
        for (index, candidate) in generations.iter().skip(1).enumerate() {
            publish_at(
                &store,
                candidate,
                || Ok(publication(candidate, &format!("candidate-{index}"))),
                |_| Ok(()),
            )
            .unwrap();
        }
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000011",
            generation,
            i32::MAX as u32,
            linux_start_time(std::process::id()),
            &linux_boot_id(),
            rustix::process::getuid().as_raw(),
        );
        write_raw_lease_fixture(
            root.path(),
            "00000000000000000000000000000012",
            b"helm-generation-lifecycle-lease-v1\n",
        );

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 0);
        assert_eq!(report.reclaimed_generations, 0);
        assert_eq!(
            lease_names(root.path()),
            vec![
                OsString::from("00000000000000000000000000000011"),
                OsString::from("00000000000000000000000000000012"),
            ]
        );
        for generation in generations {
            assert!(root.path().join("generations").join(generation).exists());
        }
    }

    #[test]
    fn g6_gc_retains_stale_process_lease_when_lifecycle_evidence_is_valid() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let generation = "00000000000000000000000000000001";
        let generations = [
            generation,
            "00000000000000000000000000000002",
            "00000000000000000000000000000003",
            "00000000000000000000000000000004",
        ];
        for (index, candidate) in generations.iter().skip(1).enumerate() {
            publish_at(
                &store,
                candidate,
                || Ok(publication(candidate, &format!("candidate-{index}"))),
                |_| Ok(()),
            )
            .unwrap();
        }
        let launch = "00000000000000000000000000000002";
        let pid = std::process::id();
        let start_time = linux_start_time(pid);
        let boot_id = linux_boot_id();
        let owner_uid = rustix::process::getuid().as_raw();
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000011",
            generation,
            i32::MAX as u32,
            start_time,
            &boot_id,
            owner_uid,
        );
        write_raw_lease_fixture(
            root.path(),
            "00000000000000000000000000000012",
            format!(
                "helm-generation-lifecycle-lease-v1\ngeneration {generation}\nlaunch {launch}\npid {pid}\nstart-time {start_time}\nboot-id {boot_id}\nowner-uid {owner_uid}\nowner-kind process-group\nunit none\nunit-invocation none\nprocess-group {pid}\ncgroup none\ncgroup-device 0\ncgroup-inode 0\n"
            )
            .as_bytes(),
        );

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 0);
        assert_eq!(report.reclaimed_generations, 0);
        assert_eq!(
            lease_names(root.path()),
            vec![
                OsString::from("00000000000000000000000000000011"),
                OsString::from("00000000000000000000000000000012"),
            ]
        );
        for generation in generations {
            assert!(root.path().join("generations").join(generation).exists());
        }
    }

    #[test]
    fn g6_gc_reclaims_provably_stale_lease_even_when_current_is_malformed() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        let candidate = "00000000000000000000000000000002";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000011",
            old,
            i32::MAX as u32,
            linux_start_time(std::process::id()),
            &linux_boot_id(),
            rustix::process::getuid().as_raw(),
        );
        std::fs::write(root.path().join("current"), b"corrupt\n").unwrap();

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 1);
        assert_eq!(report.reclaimed_generations, 0);
        assert!(lease_names(root.path()).is_empty());
        assert!(root.path().join("generations").join(old).exists());
        assert!(root.path().join("generations").join(candidate).exists());
        assert_eq!(
            std::fs::read(root.path().join("current")).unwrap(),
            b"corrupt\n"
        );
    }

    #[test]
    fn g6_gc_reclaims_provably_stale_lease_when_current_is_absent() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        write_lease_fixture(
            root.path(),
            "00000000000000000000000000000011",
            old,
            i32::MAX as u32,
            linux_start_time(std::process::id()),
            &linux_boot_id(),
            rustix::process::getuid().as_raw(),
        );
        std::fs::remove_file(root.path().join("current")).unwrap();

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 1);
        assert_eq!(report.reclaimed_generations, 0);
        assert!(lease_names(root.path()).is_empty());
        assert!(root.path().join("generations").join(old).exists());
    }

    #[test]
    fn g6_committed_journal_prior_generation_is_protected_from_gc_candidates() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let prior = "00000000000000000000000000000001";
        let current = "00000000000000000000000000000002";
        publish_at(
            &store,
            current,
            || Ok(publication(current, "current")),
            |_| Ok(()),
        )
        .unwrap();
        std::fs::write(
            root.path().join(format!(".committed-{current}")),
            format!("{prior}\n"),
        )
        .unwrap();
        for number in 3..=4 {
            let generation = format!("{number:032x}");
            v1_fixture(
                &root.path().join("generations"),
                &generation,
                "theme.ini",
                format!("orphan-{number}").as_bytes(),
            );
        }

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 0);
        assert!(root.path().join("generations").join(prior).exists());
        assert_eq!(store.select_current().unwrap().as_str(), current);
        assert_eq!(store.recover().unwrap().as_str(), current);
    }

    #[test]
    fn g6_gc_retains_the_two_greatest_publication_sequences_not_mtime_or_id_order() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let generations = root.path().join("generations");
        std::fs::create_dir(&generations).unwrap();
        std::fs::set_permissions(&generations, std::fs::Permissions::from_mode(0o700)).unwrap();
        let newest = "00000000000000000000000000000001";
        let second_newest = "00000000000000000000000000000002";
        let current = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let oldest_candidate = "ffffffffffffffffffffffffffffffff";
        for (generation, marker) in [
            (newest, b"newest".as_slice()),
            (second_newest, b"second-newest".as_slice()),
            (current, b"current".as_slice()),
            (oldest_candidate, b"oldest-candidate".as_slice()),
        ] {
            v1_fixture(&generations, generation, "theme.ini", marker);
        }
        std::fs::write(root.path().join("current"), format!("{current}\n")).unwrap();
        write_publication_order_fixture(
            root.path(),
            "helm-generation-publication-order-v1\nnext-sequence 5\ngeneration 00000000000000000000000000000001 4\ngeneration 00000000000000000000000000000002 3\ngeneration aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\ngeneration ffffffffffffffffffffffffffffffff 2\n",
        );
        let store = GenerationStore::open(root.path()).unwrap();

        // Make the oldest publication look newest to an mtime-based collector,
        // while the two newest publications have equal, much older mtimes.
        set_generation_modified(root.path(), oldest_candidate, 4_000_000_000);
        set_generation_modified(root.path(), newest, 1);
        set_generation_modified(root.path(), second_newest, 1);

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 1);
        assert!(!generations.join(oldest_candidate).exists());
        assert!(generations.join(newest).exists());
        assert!(generations.join(second_newest).exists());
        assert!(generations.join(current).exists());
    }

    #[test]
    fn g6_missing_or_malformed_publication_order_retains_every_generation() {
        for malformed in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let store = seeded_store(root.path());
            for number in 2..=4 {
                let generation = format!("{number:032x}");
                v1_fixture(
                    &root.path().join("generations"),
                    &generation,
                    "theme.ini",
                    format!("generation-{number}").as_bytes(),
                );
            }
            write_lease_fixture(
                root.path(),
                "00000000000000000000000000000099",
                "00000000000000000000000000000001",
                i32::MAX as u32,
                linux_start_time(std::process::id()),
                &linux_boot_id(),
                rustix::process::getuid().as_raw(),
            );
            if malformed {
                write_publication_order_fixture(
                    root.path(),
                    "helm-generation-publication-order-v1\nnext-sequence 5\ngeneration out-of-order 4\n",
                );
            } else {
                std::fs::remove_file(root.path().join("publication-order")).unwrap();
            }

            let report = store.garbage_collect().unwrap();

            assert_eq!(
                report.reclaimed_generations, 0,
                "GC reclaimed a generation with malformed={malformed}"
            );
            assert_eq!(report.reclaimed_leases, 1);
            assert_eq!(
                std::fs::read_dir(root.path().join("generations"))
                    .unwrap()
                    .count(),
                4
            );
        }
    }

    #[test]
    fn g6_signed_publication_sequences_are_malformed_and_retain_every_generation() {
        for signed_order in [
            "helm-generation-publication-order-v1\nnext-sequence +5\ngeneration 00000000000000000000000000000001 1\ngeneration 00000000000000000000000000000002 2\ngeneration 00000000000000000000000000000003 3\ngeneration 00000000000000000000000000000004 4\n",
            "helm-generation-publication-order-v1\nnext-sequence 5\ngeneration 00000000000000000000000000000001 1\ngeneration 00000000000000000000000000000002 +2\ngeneration 00000000000000000000000000000003 3\ngeneration 00000000000000000000000000000004 4\n",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = seeded_store(root.path());
            for number in 2..=4 {
                let generation = format!("{number:032x}");
                v1_fixture(
                    &root.path().join("generations"),
                    &generation,
                    "theme.ini",
                    format!("generation-{number}").as_bytes(),
                );
            }
            write_publication_order_fixture(root.path(), signed_order);

            let report = store.garbage_collect().unwrap();

            assert_eq!(report.reclaimed_generations, 0);
            assert_eq!(
                std::fs::read_dir(root.path().join("generations"))
                    .unwrap()
                    .count(),
                4
            );
        }
    }

    #[test]
    fn g6_missing_mapping_for_a_present_generation_retains_every_generation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        for number in 2..=4 {
            let generation = format!("{number:032x}");
            v1_fixture(
                &root.path().join("generations"),
                &generation,
                "theme.ini",
                format!("generation-{number}").as_bytes(),
            );
        }

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 0);
        assert_eq!(
            std::fs::read_dir(root.path().join("generations"))
                .unwrap()
                .count(),
            4
        );
    }

    #[test]
    fn g6_lingering_publication_order_staging_retains_every_generation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        for number in 2..=4 {
            let generation = format!("{number:032x}");
            publish_at(
                &store,
                &generation,
                || Ok(publication(&generation, &format!("generation-{number}"))),
                |_| Ok(()),
            )
            .unwrap();
        }
        let staged = root.path().join(".publication-order");
        std::fs::write(&staged, b"interrupted rewrite").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 0);
        assert_eq!(
            std::fs::read_dir(root.path().join("generations"))
                .unwrap()
                .count(),
            4
        );
        assert!(staged.exists());
    }

    #[test]
    fn publication_order_is_durable_before_a_candidate_can_become_selectable() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = FailAfterMappedPublicationOrderFilesystem {
            root: root.path().into(),
            publication_order_renames: 0,
            failed: false,
        };

        let interrupted = store.publish_with_checkpoint_ids_and_filesystem(
            || Ok(publication(candidate, "candidate")),
            |_| Ok(()),
            || GenerationId::parse(candidate),
            &mut filesystem,
        );

        assert!(interrupted.is_err());
        assert!(filesystem.failed, "test did not reach the injected crash");
        assert_eq!(filesystem.publication_order_renames, 2);
        assert_eq!(store.select_current().unwrap().as_str(), old);
        assert_eq!(store.recover().unwrap().as_str(), old);
        assert!(!root.path().join("generations").join(candidate).exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("publication-order")).unwrap(),
            "helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 1\ngeneration 00000000000000000000000000000002 2\n"
        );
    }

    #[test]
    fn interrupted_sequence_reservation_leaves_a_gap_that_a_retry_does_not_fill() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let interrupted = "00000000000000000000000000000002";
        let fresh = "00000000000000000000000000000003";

        let failed = publish_at(
            &store,
            interrupted,
            || Ok(publication(interrupted, "interrupted")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PublicationSequenceReserved {
                    Err("simulated death after sequence reservation".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(failed.is_err());
        assert!(!root.path().join("generations").join(interrupted).exists());

        publish_at(
            &store,
            fresh,
            || Ok(publication(fresh, "fresh")),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.path().join("publication-order")).unwrap(),
            "helm-generation-publication-order-v1\nnext-sequence 4\ngeneration 00000000000000000000000000000001 1\ngeneration 00000000000000000000000000000003 3\n"
        );
        assert_eq!(store.select_current().unwrap().as_str(), fresh);
    }

    #[test]
    fn publication_removes_only_safe_lingering_order_staging_before_retrying() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        let staged = root.path().join(".publication-order");
        std::fs::write(&staged, b"interrupted rewrite").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();

        publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "candidate")),
            |_| Ok(()),
        )
        .unwrap();

        assert!(!staged.exists());
        assert_eq!(store.select_current().unwrap().as_str(), candidate);
    }

    #[test]
    fn exhausted_publication_sequence_refuses_without_constructing_a_generation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        write_publication_order_fixture(
            root.path(),
            "helm-generation-publication-order-v1\nnext-sequence 18446744073709551615\ngeneration 00000000000000000000000000000001 1\n",
        );

        let outcome = publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "candidate")),
            |_| Ok(()),
        );

        assert!(outcome.is_err());
        assert!(!root.path().join("generations").join(candidate).exists());
        assert_eq!(
            store.select_current().unwrap().as_str(),
            "00000000000000000000000000000001"
        );
    }

    #[test]
    fn g6_live_old_generation_is_pinned_while_only_oldest_unleased_candidate_is_collected() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let pinned = store.select_current().unwrap();
        for number in 2..=5 {
            let generation = format!("{number:032x}");
            publish_at(
                &store,
                &generation,
                || Ok(publication(&generation, &format!("generation-{number}"))),
                |_| Ok(()),
            )
            .unwrap();
        }

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 1);
        assert!(root
            .path()
            .join("generations/00000000000000000000000000000001")
            .exists());
        assert!(!root
            .path()
            .join("generations/00000000000000000000000000000002")
            .exists());
        for number in 3..=5 {
            assert!(root
                .path()
                .join("generations")
                .join(format!("{number:032x}"))
                .exists());
        }
        assert_eq!(pinned.read_output("theme.ini").unwrap(), b"old");
    }

    #[test]
    fn g6_malformed_lease_keeps_all_generations_and_is_not_removed() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        for number in 2..=4 {
            let generation = format!("{number:032x}");
            publish_at(
                &store,
                &generation,
                || Ok(publication(&generation, &format!("generation-{number}"))),
                |_| Ok(()),
            )
            .unwrap();
        }
        let malformed = root.path().join("leases/00000000000000000000000000000099");
        std::fs::write(&malformed, b"not a canonical lease\n").unwrap();
        std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o600)).unwrap();

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_leases, 0);
        assert_eq!(report.reclaimed_generations, 0);
        assert!(malformed.exists());
        assert_eq!(
            std::fs::read_dir(root.path().join("generations"))
                .unwrap()
                .count(),
            4
        );
    }

    #[test]
    fn g6_malformed_sealed_generation_blocks_all_generation_collection() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        for number in 2..=5 {
            let generation = format!("{number:032x}");
            publish_at(
                &store,
                &generation,
                || Ok(publication(&generation, &format!("generation-{number}"))),
                |_| Ok(()),
            )
            .unwrap();
        }
        let malformed = root
            .path()
            .join("generations/00000000000000000000000000000001");
        std::fs::write(malformed.join("unlisted"), b"diagnostic evidence").unwrap();

        let report = store.garbage_collect().unwrap();

        assert_eq!(report.reclaimed_generations, 0);
        assert!(malformed.exists());
        assert!(root
            .path()
            .join("generations/00000000000000000000000000000002")
            .exists());
    }

    #[test]
    fn selection_can_lease_a_distinct_paused_target_process_and_expose_its_generation_path() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let start_time = linux_start_time(pid);

        let selected = store.select_current_for_process(pid).unwrap();

        let leases = lease_names(root.path());
        assert_eq!(leases.len(), 1);
        assert_eq!(
            std::fs::read_to_string(root.path().join("leases").join(&leases[0])).unwrap(),
            format!(
                "helm-generation-lease-v1\ngeneration 00000000000000000000000000000001\npid {pid}\nstart-time {start_time}\nboot-id {}\nowner-uid {}\n",
                linux_boot_id(),
                rustix::process::getuid().as_raw(),
            )
        );
        assert_eq!(
            selected.path(),
            root.path()
                .join("generations/00000000000000000000000000000001")
        );

        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(store.garbage_collect().unwrap().reclaimed_leases, 1);
        assert!(lease_names(root.path()).is_empty());
    }

    #[test]
    fn apply_id_exhaustion_refuses_without_reclaiming_a_protected_generation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let protected = store.select_current().unwrap();
        let collision = GenerationId::parse("00000000000000000000000000000001").unwrap();
        let outcome = store.publish_with_checkpoint_and_ids(
            || Ok(publication(collision.as_str(), "candidate")),
            |_| Ok(()),
            || Ok(collision.clone()),
        );

        assert!(outcome.is_err());
        assert_eq!(protected.read_output("theme.ini").unwrap(), b"old");
        assert!(root
            .path()
            .join("generations/00000000000000000000000000000001")
            .exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{}\n", protected.as_str())
        );
    }

    #[test]
    fn g2_interruption_before_pointer_commit_keeps_previous_current_and_recovery_discards_staging()
    {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let interrupted = publish_at(
            &store,
            "00000000000000000000000000000002",
            || Ok(publication("00000000000000000000000000000002", "new")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::OutputsSynced {
                    Err("simulated interruption".into())
                } else {
                    Ok(())
                }
            },
        );

        assert!(interrupted.is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            "00000000000000000000000000000001\n"
        );
        assert_eq!(
            store.recover().unwrap().as_str(),
            "00000000000000000000000000000001"
        );
        assert!(
            std::fs::read_dir(root.path().join("generations"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".staging-")),
            "recovery left an interrupted staging tree launch-adjacent"
        );
    }

    #[test]
    fn g3_interruption_after_tree_seal_retains_old_current_and_never_promotes_the_orphan() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let new = "00000000000000000000000000000002";
        let interrupted = publish_at(
            &store,
            new,
            || Ok(publication(new, "new")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::GenerationsDirectorySynced {
                    Err("simulated interruption".into())
                } else {
                    Ok(())
                }
            },
        );

        assert!(interrupted.is_err());
        assert!(root.path().join("generations").join(new).is_dir());
        assert_eq!(
            store.recover().unwrap().as_str(),
            "00000000000000000000000000000001"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            "00000000000000000000000000000001\n"
        );
    }

    #[test]
    fn g2_recovery_removes_an_uncommitted_pointer_sibling_without_promoting_its_sealed_tree() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let new = "00000000000000000000000000000002";
        let interrupted = publish_at(
            &store,
            new,
            || Ok(publication(new, "new")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PointerFileSynced {
                    Err("simulated interruption".into())
                } else {
                    Ok(())
                }
            },
        );

        assert!(interrupted.is_err());
        assert!(root.path().join(format!(".current-{new}")).is_file());
        assert_eq!(
            store.recover().unwrap().as_str(),
            "00000000000000000000000000000001"
        );
        assert!(root.path().join("generations").join(new).is_dir());
        assert!(!root.path().join(format!(".current-{new}")).exists());
    }

    #[test]
    fn g4_two_writers_cannot_overlap_generation_construction_or_mix_inputs() {
        let root = tempfile::tempdir().unwrap();
        let first_store = seeded_store(root.path());
        let second_store = GenerationStore::open(root.path()).unwrap();
        let (first_staged_tx, first_staged_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first = std::thread::spawn(move || {
            publish_at(
                &first_store,
                "00000000000000000000000000000002",
                || {
                    Ok(publication(
                        "00000000000000000000000000000002",
                        "writer-one",
                    ))
                },
                |checkpoint| {
                    if checkpoint == CommitCheckpoint::OutputsSynced {
                        first_staged_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });
        first_staged_rx.recv().unwrap();

        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_captured_tx, second_captured_rx) = mpsc::channel();
        let second = std::thread::spawn(move || {
            second_started_tx.send(()).unwrap();
            publish_at(
                &second_store,
                "00000000000000000000000000000003",
                || {
                    second_captured_tx.send(()).unwrap();
                    Ok(publication(
                        "00000000000000000000000000000003",
                        "writer-two",
                    ))
                },
                |_| Ok(()),
            )
        });

        second_started_rx.recv().unwrap();
        let overlapped = second_captured_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).unwrap();
        assert!(
            overlapped.is_err(),
            "the second writer captured inputs while the first tree was incomplete"
        );
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();

        for (id, marker) in [
            ("00000000000000000000000000000002", "writer-one"),
            ("00000000000000000000000000000003", "writer-two"),
        ] {
            let path = root.path().join("generations").join(id);
            let raw = std::fs::read_to_string(path.join("manifest")).unwrap();
            let manifest = Manifest::parse(&raw).unwrap();
            manifest.verify(&path).unwrap();
            assert_eq!(
                manifest.v1.input_digests,
                publication(id, marker).input_digests
            );
            assert_eq!(
                std::fs::read(path.join("theme.ini")).unwrap(),
                marker.as_bytes()
            );
        }
    }

    #[test]
    fn g9_two_writers_serialize_capture_through_pointer_directory_durability() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(seeded_store(root.path()));
        let (pointer_replaced_tx, pointer_replaced_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_store = Arc::clone(&store);
        let first = std::thread::spawn(move || {
            publish_at(
                first_store.as_ref(),
                "00000000000000000000000000000002",
                || {
                    Ok(publication(
                        "00000000000000000000000000000002",
                        "writer-one",
                    ))
                },
                |checkpoint| {
                    if checkpoint == CommitCheckpoint::PointerCommitted {
                        pointer_replaced_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });
        pointer_replaced_rx.recv().unwrap();

        let (second_started_tx, second_started_rx) = mpsc::channel();
        let (second_captured_tx, second_captured_rx) = mpsc::channel();
        let second_store = Arc::clone(&store);
        let second = std::thread::spawn(move || {
            second_started_tx.send(()).unwrap();
            publish_at(
                second_store.as_ref(),
                "00000000000000000000000000000003",
                || {
                    second_captured_tx.send(()).unwrap();
                    Ok(publication(
                        "00000000000000000000000000000003",
                        "writer-two",
                    ))
                },
                |_| Ok(()),
            )
        });

        second_started_rx.recv().unwrap();
        let overlapped = second_captured_rx.recv_timeout(Duration::from_secs(1));
        release_tx.send(()).unwrap();
        assert!(
            overlapped.is_err(),
            "the second writer captured inputs before the first pointer directory fsync"
        );
        assert_eq!(
            first.join().unwrap().unwrap().as_str(),
            "00000000000000000000000000000002"
        );
        assert_eq!(
            second.join().unwrap().unwrap().as_str(),
            "00000000000000000000000000000003"
        );
        assert_eq!(
            store.recover().unwrap().as_str(),
            "00000000000000000000000000000003"
        );
        for (id, marker) in [
            ("00000000000000000000000000000002", "writer-one"),
            ("00000000000000000000000000000003", "writer-two"),
        ] {
            assert_eq!(
                std::fs::read_to_string(root.path().join("generations").join(id).join("receipt"))
                    .unwrap(),
                publication(id, marker).receipt(&GenerationId::parse(id).unwrap())
            );
        }
    }

    #[test]
    fn process_death_releases_the_persistent_lock_and_recovery_discards_its_staging_tree() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let generation = "00000000000000000000000000000002";
        let staging = root
            .path()
            .join("generations")
            .join(format!(".staging-{generation}"));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generation::tests::generation_lock_holder_subprocess",
                "--nocapture",
            ])
            .env("HELM_THEME_LOCK_HOLDER_ROOT", root.path())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            if output.read_line(&mut line).unwrap() == 0 {
                break;
            }
            let locked = line.contains("HELM_THEME_LOCK_HELD");
            lines.push(line);
            if locked {
                break;
            }
        }
        assert!(
            lines
                .iter()
                .any(|line| line.contains("HELM_THEME_LOCK_HELD")),
            "lock-holder subprocess exited before acquiring the lock: {lines:?}"
        );
        assert!(staging.is_dir());

        child.kill().unwrap();
        let status = child.wait().unwrap();
        assert!(!status.success(), "lock-holder subprocess was not killed");
        assert_eq!(
            store.recover().unwrap().as_str(),
            "00000000000000000000000000000001"
        );
        assert!(!staging.exists());
    }

    #[test]
    fn killed_writer_at_pointer_transition_is_never_selected() {
        let root = tempfile::tempdir().unwrap();
        let store = Arc::new(seeded_store(root.path()));
        let old = "00000000000000000000000000000001";
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "generation::tests::generation_lock_holder_subprocess",
                "--nocapture",
            ])
            .env("HELM_THEME_LOCK_HOLDER_ROOT", root.path())
            .env("HELM_THEME_POINTER_TRANSITION", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(output.read_line(&mut line).unwrap(), 0);
            if line.contains("HELM_THEME_POINTER_TRANSITION_HELD") {
                break;
            }
        }
        let (selected_tx, selected_rx) = mpsc::channel();
        let selecting_store = Arc::clone(&store);
        let selecting = std::thread::spawn(move || {
            selected_tx.send(selecting_store.select_current()).unwrap();
        });
        assert!(selected_rx
            .recv_timeout(Duration::from_millis(150))
            .is_err());
        child.kill().unwrap();
        child.wait().unwrap();

        assert!(selected_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_err());
        selecting.join().unwrap();
        assert_eq!(store.recover().unwrap().as_str(), old);
        assert_eq!(store.select_current().unwrap().as_str(), old);
    }

    #[test]
    fn generation_lock_holder_subprocess() {
        let Some(root) = std::env::var_os("HELM_THEME_LOCK_HOLDER_ROOT") else {
            return;
        };
        let root = PathBuf::from(root);
        let store = GenerationStore::open(&root).unwrap();
        if std::env::var_os("HELM_THEME_POINTER_TRANSITION").is_some() {
            let candidate = "00000000000000000000000000000002";
            let _ = publish_at(
                &store,
                candidate,
                || Ok(publication(candidate, "killed-writer")),
                |checkpoint| {
                    if checkpoint == CommitCheckpoint::PointerCommitted {
                        println!("HELM_THEME_POINTER_TRANSITION_HELD");
                        std::io::stdout().flush().unwrap();
                        loop {
                            std::thread::park_timeout(Duration::from_secs(30));
                        }
                    }
                    Ok(())
                },
            );
            unreachable!();
        }
        let _lock = store.lock_exclusive().unwrap();
        let staging = ".staging-00000000000000000000000000000002";
        mkdirat(
            &store.generations.fd,
            staging,
            Mode::RUSR | Mode::WUSR | Mode::XUSR,
        )
        .unwrap();
        write_synced_file(
            &openat(
                &store.generations.fd,
                staging,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
            "partial",
            b"partial",
            &mut RealPublicationFilesystem,
        )
        .unwrap();
        println!("HELM_THEME_LOCK_HELD");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park_timeout(Duration::from_secs(30));
        }
    }

    #[cfg(unix)]
    #[test]
    fn g8_recovery_fails_closed_for_corrupt_missing_mismatched_and_special_pointers() {
        let corrupt_root = tempfile::tempdir().unwrap();
        let corrupt_store = seeded_store(corrupt_root.path());
        v1_fixture(
            &corrupt_root.path().join("generations"),
            "ffffffffffffffffffffffffffffffff",
            "theme.ini",
            b"newest-but-not-selected",
        );
        std::fs::write(corrupt_root.path().join("current"), "corrupt\n").unwrap();
        assert!(corrupt_store.recover().is_err());
        assert_eq!(
            std::fs::read_to_string(corrupt_root.path().join("current")).unwrap(),
            "corrupt\n",
            "recovery guessed a retained generation"
        );

        let missing_root = tempfile::tempdir().unwrap();
        let missing_store = seeded_store(missing_root.path());
        std::fs::write(
            missing_root.path().join("current"),
            "00000000000000000000000000000044\n",
        )
        .unwrap();
        assert!(missing_store.recover().is_err());

        let mismatch_root = tempfile::tempdir().unwrap();
        let mismatch_store = seeded_store(mismatch_root.path());
        std::fs::write(
            mismatch_root
                .path()
                .join("generations/00000000000000000000000000000001/theme.ini"),
            "changed",
        )
        .unwrap();
        assert!(mismatch_store.recover().is_err());

        let fifo_root = tempfile::tempdir().unwrap();
        secure_store_root(fifo_root.path());
        let fifo_store = Arc::new(GenerationStore::open(fifo_root.path()).unwrap());
        rustix::fs::mkfifoat(
            CWD,
            fifo_root.path().join("current"),
            Mode::RUSR | Mode::WUSR,
        )
        .unwrap();
        let (recovered_tx, recovered_rx) = mpsc::channel();
        let recovering_store = Arc::clone(&fifo_store);
        let recovering = std::thread::spawn(move || {
            recovered_tx.send(recovering_store.recover()).unwrap();
        });
        let prompt_result = recovered_rx.recv_timeout(Duration::from_millis(150));
        if prompt_result.is_err() {
            let _writer = std::fs::OpenOptions::new()
                .write(true)
                .open(fifo_root.path().join("current"))
                .unwrap();
            recovering.join().unwrap();
        }
        assert!(
            prompt_result.is_ok_and(|result| result.is_err()),
            "recovery blocked while opening a FIFO current pointer"
        );

        let output_fifo_root = tempfile::tempdir().unwrap();
        let output_fifo_store = Arc::new(seeded_store(output_fifo_root.path()));
        let output = output_fifo_root
            .path()
            .join("generations/00000000000000000000000000000001/theme.ini");
        std::fs::remove_file(&output).unwrap();
        rustix::fs::mkfifoat(CWD, &output, Mode::RUSR | Mode::WUSR).unwrap();
        let (validated_tx, validated_rx) = mpsc::channel();
        let validating_store = Arc::clone(&output_fifo_store);
        let validating = std::thread::spawn(move || {
            validated_tx.send(validating_store.recover()).unwrap();
        });
        let prompt_result = validated_rx.recv_timeout(Duration::from_millis(150));
        if prompt_result.is_err() {
            let _writer = std::fs::OpenOptions::new()
                .write(true)
                .open(&output)
                .unwrap();
            validating.join().unwrap();
        }
        assert!(
            prompt_result.is_ok_and(|result| result.is_err()),
            "recovery blocked while validating a FIFO generation output"
        );
    }

    #[test]
    fn generation_store_creates_one_regular_activation_lock() {
        use std::os::unix::fs::MetadataExt;

        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let first = GenerationStore::open(root.path()).unwrap();
        let second = GenerationStore::open(root.path()).unwrap();
        let lock = std::fs::symlink_metadata(root.path().join("activation.lock")).unwrap();
        assert!(lock.is_file());
        assert!(!lock.file_type().is_symlink());
        assert_eq!(
            std::fs::File::from(first.lock.try_clone().unwrap())
                .metadata()
                .unwrap()
                .ino(),
            std::fs::File::from(second.lock.try_clone().unwrap())
                .metadata()
                .unwrap()
                .ino(),
            "store handles did not retain the same lock inode"
        );
        let order = std::fs::symlink_metadata(root.path().join("publication-order")).unwrap();
        assert!(order.is_file());
        assert!(!order.file_type().is_symlink());
        assert_eq!(order.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(root.path().join("publication-order")).unwrap(),
            "helm-generation-publication-order-v1\nnext-sequence 1\n"
        );
    }

    #[test]
    fn concurrent_first_openers_join_the_same_safely_created_controls() {
        use std::os::unix::fs::MetadataExt;
        use std::sync::Barrier;

        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let root_path = root.path().to_owned();
        let preflight = Arc::new(Barrier::new(2));
        let openers: Vec<_> = (0..2)
            .map(|_| {
                let root_path = root_path.clone();
                let preflight = Arc::clone(&preflight);
                std::thread::spawn(move || {
                    GenerationStore::open_with_preflight_checkpoint(&root_path, || {
                        preflight.wait();
                    })
                })
            })
            .collect();
        let stores: Vec<_> = openers
            .into_iter()
            .map(|opener| opener.join().unwrap().unwrap())
            .collect();

        let inodes: Vec<_> = stores
            .iter()
            .map(|store| {
                std::fs::File::from(store.lock.try_clone().unwrap())
                    .metadata()
                    .unwrap()
                    .ino()
            })
            .collect();
        assert_eq!(inodes[0], inodes[1]);
        assert_eq!(
            std::fs::metadata(root.path().join("activation.lock"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(root.path().join("generations"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn generation_store_never_follows_or_replaces_an_activation_lock_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let victim = root.path().join("victim");
        std::fs::write(&victim, "unchanged").unwrap();
        symlink(&victim, root.path().join("activation.lock")).unwrap();

        assert!(GenerationStore::open(root.path()).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");
        assert!(
            std::fs::symlink_metadata(root.path().join("activation.lock"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn generation_store_rejects_an_insecure_root_before_creating_control_entries() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o777)).unwrap();

        assert!(GenerationStore::open(root.path()).is_err());
        assert!(!root.path().join("activation.lock").exists());
        assert!(!root.path().join("generations").exists());
    }

    #[test]
    fn generation_store_rejects_unsafe_existing_control_entries_before_mutation() {
        let lock_root = tempfile::tempdir().unwrap();
        secure_store_root(lock_root.path());
        let lock = lock_root.path().join("activation.lock");
        std::fs::write(&lock, "hostile").unwrap();
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o666)).unwrap();

        assert!(GenerationStore::open(lock_root.path()).is_err());
        assert_eq!(std::fs::read_to_string(&lock).unwrap(), "hostile");
        assert!(!lock_root.path().join("generations").exists());

        let generations_root = tempfile::tempdir().unwrap();
        secure_store_root(generations_root.path());
        let generations = generations_root.path().join("generations");
        std::fs::create_dir(&generations).unwrap();
        std::fs::set_permissions(&generations, std::fs::Permissions::from_mode(0o777)).unwrap();

        assert!(GenerationStore::open(generations_root.path()).is_err());
        assert!(!generations_root.path().join("activation.lock").exists());
        assert_eq!(
            std::fs::metadata(&generations)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o777
        );

        let leases_root = tempfile::tempdir().unwrap();
        secure_store_root(leases_root.path());
        let victim = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(victim.path(), leases_root.path().join("leases")).unwrap();

        assert!(GenerationStore::open(leases_root.path()).is_err());
        assert!(!leases_root.path().join("activation.lock").exists());
        assert!(!leases_root.path().join("generations").exists());
        assert!(std::fs::read_dir(victim.path()).unwrap().next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_or_malformed_order_does_not_block_restart_selection_or_stale_lease_cleanup() {
        for state in [
            "official-symlink",
            "official-fifo",
            "official-unsafe-mode",
            "staging-symlink",
            "malformed-official",
        ] {
            let root = tempfile::tempdir().unwrap();
            let store = seeded_store(root.path());
            for number in 2..=4 {
                let generation = format!("{number:032x}");
                publish_at(
                    &store,
                    &generation,
                    || Ok(publication(&generation, &format!("generation-{number}"))),
                    |_| Ok(()),
                )
                .unwrap();
            }
            drop(store);

            let victim = root.path().join("victim");
            std::fs::write(&victim, "unchanged").unwrap();
            let official = root.path().join("publication-order");
            let staging = root.path().join(".publication-order");
            match state {
                "official-symlink" => {
                    std::fs::remove_file(&official).unwrap();
                    std::os::unix::fs::symlink(&victim, &official).unwrap();
                }
                "official-fifo" => {
                    std::fs::remove_file(&official).unwrap();
                    rustix::fs::mkfifoat(CWD, &official, Mode::RUSR | Mode::WUSR).unwrap();
                }
                "official-unsafe-mode" => {
                    std::fs::set_permissions(&official, std::fs::Permissions::from_mode(0o666))
                        .unwrap();
                }
                "staging-symlink" => {
                    std::os::unix::fs::symlink(&victim, &staging).unwrap();
                }
                "malformed-official" => {
                    std::fs::write(&official, "not publication order v1\n").unwrap();
                }
                _ => unreachable!(),
            }

            let reopened = GenerationStore::open(root.path())
                .unwrap_or_else(|error| panic!("restart rejected {state}: {error}"));
            let selected = reopened.select_current().unwrap();
            assert_eq!(selected.as_str(), "00000000000000000000000000000004");
            selected.release().unwrap();
            write_lease_fixture(
                root.path(),
                "00000000000000000000000000000099",
                "00000000000000000000000000000001",
                i32::MAX as u32,
                linux_start_time(std::process::id()),
                &linux_boot_id(),
                rustix::process::getuid().as_raw(),
            );

            let report = reopened.garbage_collect().unwrap();

            assert_eq!(report.reclaimed_leases, 1, "state={state}");
            assert_eq!(report.reclaimed_generations, 0, "state={state}");
            assert_eq!(
                std::fs::read_dir(root.path().join("generations"))
                    .unwrap()
                    .count(),
                4
            );
            let candidate = "00000000000000000000000000000005";
            assert!(publish_at(
                &reopened,
                candidate,
                || Ok(publication(candidate, "must-fail-closed")),
                |_| Ok(()),
            )
            .is_err());
            assert!(!root.path().join("generations").join(candidate).exists());
            assert_eq!(
                reopened.select_current().unwrap().as_str(),
                "00000000000000000000000000000004"
            );
            assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");
            match state {
                "official-symlink" => assert!(std::fs::symlink_metadata(&official)
                    .unwrap()
                    .file_type()
                    .is_symlink()),
                "official-fifo" => assert!(std::fs::symlink_metadata(&official)
                    .unwrap()
                    .file_type()
                    .is_fifo()),
                "official-unsafe-mode" => assert_eq!(
                    std::fs::metadata(&official).unwrap().permissions().mode() & 0o777,
                    0o666
                ),
                "staging-symlink" => assert!(std::fs::symlink_metadata(&staging)
                    .unwrap()
                    .file_type()
                    .is_symlink()),
                "malformed-official" => assert_eq!(
                    std::fs::read_to_string(&official).unwrap(),
                    "not publication order v1\n"
                ),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn secure_metadata_validation_rejects_a_foreign_owner() {
        let root = tempfile::tempdir().unwrap();
        let stat = rustix::fs::fstat(&GenerationRoot::open(root.path()).unwrap().fd).unwrap();
        let current_uid = rustix::process::getuid().as_raw();
        let foreign_uid = current_uid.wrapping_add(1);

        assert!(validate_owned_mode(
            "generated root",
            &stat,
            rustix::fs::FileType::Directory,
            0o700,
            foreign_uid,
        )
        .is_err());
    }

    #[test]
    fn publication_checkpoint_trace_pins_the_durability_order() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let mut observed = Vec::new();

        publish_at(
            &store,
            "00000000000000000000000000000001",
            || Ok(publication("00000000000000000000000000000001", "ordered")),
            |checkpoint| {
                observed.push(checkpoint);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            observed,
            vec![
                CommitCheckpoint::InputsCaptured,
                CommitCheckpoint::PublicationSequenceReserved,
                CommitCheckpoint::OutputsSynced,
                CommitCheckpoint::ReceiptSynced,
                CommitCheckpoint::ManifestSynced,
                CommitCheckpoint::SealSynced,
                CommitCheckpoint::StagingDirectorySynced,
                CommitCheckpoint::PublicationOrderSynced,
                CommitCheckpoint::TreeCommitted,
                CommitCheckpoint::GenerationsDirectorySynced,
                CommitCheckpoint::PointerFileSynced,
                CommitCheckpoint::PointerCommitted,
                CommitCheckpoint::RootDirectorySynced,
            ]
        );
    }

    #[test]
    fn publication_invokes_real_sync_and_rename_syscalls_in_durability_order() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let generation = "00000000000000000000000000000001";
        let staging = root
            .path()
            .join("generations")
            .join(format!(".staging-{generation}"));
        let final_generation = root.path().join("generations").join(generation);
        let mut filesystem = RecordingPublicationFilesystem::default();

        store
            .publish_with_checkpoint_ids_and_filesystem(
                || Ok(publication(generation, "ordered")),
                |_| Ok(()),
                || GenerationId::parse(generation),
                &mut filesystem,
            )
            .unwrap();

        assert_eq!(
            filesystem.events,
            vec![
                ActualFilesystemEvent::Sync(root.path().join(".publication-order")),
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().into(),
                    from_name: ".publication-order".into(),
                    to_directory: root.path().into(),
                    to_name: "publication-order".into(),
                },
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Sync(staging.join("theme.ini")),
                ActualFilesystemEvent::Sync(staging.clone()),
                ActualFilesystemEvent::Sync(staging.join("receipt")),
                ActualFilesystemEvent::Sync(staging.join("manifest")),
                ActualFilesystemEvent::Sync(staging.join("seal")),
                ActualFilesystemEvent::Sync(staging.clone()),
                ActualFilesystemEvent::Sync(root.path().join(".publication-order")),
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().into(),
                    from_name: ".publication-order".into(),
                    to_directory: root.path().into(),
                    to_name: "publication-order".into(),
                },
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().join("generations"),
                    from_name: format!(".staging-{generation}"),
                    to_directory: root.path().join("generations"),
                    to_name: generation.into(),
                },
                ActualFilesystemEvent::Sync(root.path().join("generations")),
                ActualFilesystemEvent::Sync(root.path().join(format!(".current-{generation}"))),
                ActualFilesystemEvent::Sync(root.path().join(format!(".absent-{generation}"))),
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().into(),
                    from_name: format!(".current-{generation}"),
                    to_directory: root.path().into(),
                    to_name: "current".into(),
                },
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().into(),
                    from_name: format!(".absent-{generation}"),
                    to_directory: root.path().into(),
                    to_name: format!(".committed-{generation}"),
                },
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Unlink(root.path().join(format!(".committed-{generation}")),),
                ActualFilesystemEvent::Sync(root.path().into()),
            ]
        );
        assert!(final_generation.join("seal").is_file());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{generation}\n")
        );
    }

    #[test]
    fn existing_current_exchange_records_durable_rollback_before_activation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = RecordingPublicationFilesystem::default();

        store
            .publish_with_checkpoint_ids_and_filesystem(
                || Ok(publication(candidate, "candidate")),
                |_| Ok(()),
                || GenerationId::parse(candidate),
                &mut filesystem,
            )
            .unwrap();

        let pointer = root.path().join(format!(".current-{candidate}"));
        let committed = root.path().join(format!(".committed-{candidate}"));
        let root_path = root.path().to_path_buf();
        let tail = &filesystem.events[filesystem.events.len() - 8..];
        assert_eq!(
            tail,
            [
                ActualFilesystemEvent::Sync(pointer),
                ActualFilesystemEvent::Sync(root_path.clone()),
                ActualFilesystemEvent::Exchange {
                    left_directory: root_path.clone(),
                    left_name: format!(".current-{candidate}"),
                    right_directory: root_path.clone(),
                    right_name: "current".into(),
                },
                ActualFilesystemEvent::Sync(root_path.clone()),
                ActualFilesystemEvent::Rename {
                    from_directory: root_path.clone(),
                    from_name: format!(".current-{candidate}"),
                    to_directory: root_path.clone(),
                    to_name: format!(".committed-{candidate}"),
                },
                ActualFilesystemEvent::Sync(root_path.clone()),
                ActualFilesystemEvent::Unlink(committed),
                ActualFilesystemEvent::Sync(root_path),
            ]
        );
    }

    #[test]
    fn selector_uses_the_shared_lock_and_refuses_a_pending_pointer_rollback() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        let candidate = "00000000000000000000000000000002";
        let interrupted = publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "candidate")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PointerCommitted {
                    Err("killed writer".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(interrupted.is_err());
        assert!(store.select_current().is_err());
        assert_eq!(store.recover().unwrap().as_str(), old);
        assert_eq!(store.select_current().unwrap().as_str(), old);
    }

    #[test]
    fn selector_rejects_a_committed_marker_that_conflicts_with_current() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        std::fs::write(
            root.path()
                .join(".committed-00000000000000000000000000000002"),
            "00000000000000000000000000000001\n",
        )
        .unwrap();

        assert!(store.select_current().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn reserved_pointer_journal_prefixes_fail_closed_when_malformed() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;

        let malformed = tempfile::tempdir().unwrap();
        let malformed_store = seeded_store(malformed.path());
        let malformed_path = malformed.path().join(".current-not-a-generation-id");
        std::fs::write(&malformed_path, b"junk").unwrap();
        assert!(malformed_store.select_current().is_err());
        assert!(malformed_store.recover().is_err());
        assert!(malformed_path.exists());

        let inconsistent = tempfile::tempdir().unwrap();
        let inconsistent_store = seeded_store(inconsistent.path());
        let inconsistent_path = inconsistent
            .path()
            .join(".current-00000000000000000000000000000002");
        std::fs::write(&inconsistent_path, b"00000000000000000000000000000003\n").unwrap();
        assert!(inconsistent_store.select_current().is_err());
        assert!(inconsistent_store.recover().is_err());
        assert!(inconsistent_path.exists());

        let repeated_prefix = tempfile::tempdir().unwrap();
        let repeated_prefix_store = seeded_store(repeated_prefix.path());
        let repeated_prefix_path = repeated_prefix
            .path()
            .join(".current-.current-00000000000000000000000000000002");
        std::fs::write(&repeated_prefix_path, b"00000000000000000000000000000002\n").unwrap();
        assert!(repeated_prefix_store.select_current().is_err());
        assert!(repeated_prefix_store.recover().is_err());
        assert!(repeated_prefix_path.exists());

        let non_utf8 = tempfile::tempdir().unwrap();
        let non_utf8_store = seeded_store(non_utf8.path());
        let non_utf8_path = non_utf8
            .path()
            .join(OsString::from_vec(b".absent-\xff".to_vec()));
        std::fs::write(&non_utf8_path, b"").unwrap();
        assert!(non_utf8_store.select_current().is_err());
        assert!(non_utf8_store.recover().is_err());
        assert!(non_utf8_path.exists());

        let special = tempfile::tempdir().unwrap();
        let special_store = seeded_store(special.path());
        let suffix = "00000000000000000000000000000002";
        let special_path = special.path().join(format!(".current-{suffix}"));
        symlink(special.path().join("current"), &special_path).unwrap();
        assert!(special_store.select_current().is_err());
        assert!(special_store.recover().is_err());
        assert!(std::fs::symlink_metadata(&special_path)
            .unwrap()
            .file_type()
            .is_symlink());

        let bad_content = tempfile::tempdir().unwrap();
        let bad_content_store = seeded_store(bad_content.path());
        let bad_content_path = bad_content.path().join(format!(".absent-{suffix}"));
        std::fs::write(&bad_content_path, b"not-empty").unwrap();
        assert!(bad_content_store.select_current().is_err());
        assert!(bad_content_store.recover().is_err());
        assert!(bad_content_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn reserved_committed_journal_prefixes_fail_closed_when_malformed() {
        use std::os::unix::ffi::OsStringExt;

        let malformed = tempfile::tempdir().unwrap();
        let malformed_store = seeded_store(malformed.path());
        let malformed_path = malformed.path().join(".committed-not-a-generation-id");
        std::fs::write(&malformed_path, b"").unwrap();
        assert!(malformed_store.select_current().is_err());
        assert!(malformed_store.recover().is_err());
        assert!(malformed_path.exists());

        let non_utf8 = tempfile::tempdir().unwrap();
        let non_utf8_store = seeded_store(non_utf8.path());
        let non_utf8_path = non_utf8
            .path()
            .join(OsString::from_vec(b".committed-\xff".to_vec()));
        std::fs::write(&non_utf8_path, b"").unwrap();
        assert!(non_utf8_store.select_current().is_err());
        assert!(non_utf8_store.recover().is_err());
        assert!(non_utf8_path.exists());
    }

    #[test]
    fn recovery_rejects_multiple_current_journals_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let current = "00000000000000000000000000000001";
        let orphan = "00000000000000000000000000000002";
        v1_fixture(
            &root.path().join("generations"),
            orphan,
            "theme.ini",
            b"orphan",
        );
        let pre_exchange = root.path().join(format!(".current-{orphan}"));
        let post_exchange = root.path().join(format!(".current-{current}"));
        let unrelated_staging = root
            .path()
            .join("generations/.staging-00000000000000000000000000000003");
        std::fs::write(&pre_exchange, format!("{orphan}\n")).unwrap();
        std::fs::write(&post_exchange, format!("{orphan}\n")).unwrap();
        std::fs::create_dir(&unrelated_staging).unwrap();

        assert!(store.select_current().is_err());
        assert!(store.recover().is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{current}\n")
        );
        assert!(pre_exchange.exists());
        assert!(post_exchange.exists());
        assert!(unrelated_staging.exists());
    }

    #[test]
    fn recovery_rejects_mixed_pristine_journals_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let current = "00000000000000000000000000000001";
        let orphan = "00000000000000000000000000000002";
        v1_fixture(
            &root.path().join("generations"),
            orphan,
            "theme.ini",
            b"orphan",
        );
        let absent = root.path().join(format!(".absent-{current}"));
        let staged = root.path().join(format!(".current-{orphan}"));
        std::fs::write(&absent, b"").unwrap();
        std::fs::write(&staged, format!("{orphan}\n")).unwrap();

        assert!(store.select_current().is_err());
        assert!(store.recover().is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{current}\n")
        );
        assert!(absent.exists());
        assert!(staged.exists());
    }

    #[test]
    fn committed_journal_rejects_unreachable_self_content() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let current = "00000000000000000000000000000001";
        let committed = root.path().join(format!(".committed-{current}"));
        std::fs::write(&committed, format!("{current}\n")).unwrap();

        assert!(store.select_current().is_err());
        assert!(store.recover().is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{current}\n")
        );
        assert!(committed.exists());
    }

    #[test]
    fn recovery_accepts_only_the_exact_pristine_two_journal_inventory() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        let staged = root.path().join(format!(".current-{candidate}"));
        let absent = root.path().join(format!(".absent-{candidate}"));
        std::fs::write(&staged, format!("{candidate}\n")).unwrap();
        std::fs::write(&absent, b"").unwrap();

        assert!(store.select_current().is_err());
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");
        assert!(!root.path().join("current").exists());
        assert!(!staged.exists());
        assert!(!absent.exists());
    }

    #[test]
    fn interrupted_pristine_rollback_leaves_a_selectable_refusal_and_recovers_idempotently() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        std::fs::write(root.path().join("current"), format!("{candidate}\n")).unwrap();
        let staged = root.path().join(format!(".current-{candidate}"));
        let absent = root.path().join(format!(".absent-{candidate}"));
        std::fs::write(&absent, b"").unwrap();
        let mut filesystem = RecordingPublicationFilesystem::default();

        let interrupted = store.recover_with_checkpoint_and_filesystem(
            |checkpoint| {
                if checkpoint == RecoveryCheckpoint::PristineCurrentJournaled {
                    Err("simulated recovery death".into())
                } else {
                    Ok(())
                }
            },
            &mut filesystem,
        );

        assert_eq!(interrupted.unwrap_err(), "simulated recovery death");
        assert!(!root.path().join("current").exists());
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            format!("{candidate}\n")
        );
        assert!(absent.is_file());
        assert!(store.select_current().is_err());
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");
        assert!(!staged.exists());
        assert!(!absent.exists());
    }

    #[test]
    fn interrupted_pristine_cleanup_leaves_the_lone_current_journal_recoverable() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        std::fs::write(root.path().join("current"), format!("{candidate}\n")).unwrap();
        let staged = root.path().join(format!(".current-{candidate}"));
        let absent = root.path().join(format!(".absent-{candidate}"));
        std::fs::write(&absent, b"").unwrap();
        let mut filesystem = RecordingPublicationFilesystem::default();

        let interrupted = store.recover_with_checkpoint_and_filesystem(
            |checkpoint| {
                if checkpoint == RecoveryCheckpoint::PristineAbsentRemoved {
                    Err("simulated cleanup death".into())
                } else {
                    Ok(())
                }
            },
            &mut filesystem,
        );

        assert_eq!(interrupted.unwrap_err(), "simulated cleanup death");
        assert!(!root.path().join("current").exists());
        assert_eq!(
            std::fs::read_to_string(&staged).unwrap(),
            format!("{candidate}\n")
        );
        assert!(!absent.exists());
        assert!(store.select_current().is_err());
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");
        assert!(!staged.exists());
    }

    #[test]
    fn pristine_rollback_syscalls_pin_recoverable_root_durability_order() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        std::fs::write(root.path().join("current"), format!("{candidate}\n")).unwrap();
        std::fs::write(root.path().join(format!(".absent-{candidate}")), b"").unwrap();
        let mut filesystem = RecordingPublicationFilesystem::default();

        assert_eq!(
            store
                .recover_with_checkpoint_and_filesystem(|_| Ok(()), &mut filesystem)
                .unwrap_err(),
            "current generation is absent"
        );
        assert_eq!(
            filesystem.events,
            vec![
                ActualFilesystemEvent::Rename {
                    from_directory: root.path().into(),
                    from_name: "current".into(),
                    to_directory: root.path().into(),
                    to_name: format!(".current-{candidate}"),
                },
                ActualFilesystemEvent::Sync(root.path().into()),
                ActualFilesystemEvent::Unlink(root.path().join(format!(".absent-{candidate}"))),
                ActualFilesystemEvent::Unlink(root.path().join(format!(".current-{candidate}"))),
                ActualFilesystemEvent::Sync(root.path().into()),
            ]
        );
    }

    #[test]
    fn committed_journal_accepts_reachable_empty_pristine_content() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";
        v1_fixture(
            &root.path().join("generations"),
            candidate,
            "theme.ini",
            b"candidate",
        );
        std::fs::write(root.path().join("current"), format!("{candidate}\n")).unwrap();
        let committed = root.path().join(format!(".committed-{candidate}"));
        std::fs::write(&committed, b"").unwrap();

        assert_eq!(store.select_current().unwrap().as_str(), candidate);
        assert_eq!(store.recover().unwrap().as_str(), candidate);
        assert!(!committed.exists());
    }

    #[test]
    fn failed_pointer_directory_sync_restores_the_previous_selection() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = FailOnceOnRootSyncFilesystem {
            root: root.path().into(),
            fail_on: 4,
            root_syncs: 0,
            failed: false,
            fail_unlink: false,
        };

        let published = store.publish_with_checkpoint_ids_and_filesystem(
            || Ok(publication(candidate, "candidate")),
            |_| Ok(()),
            || GenerationId::parse(candidate),
            &mut filesystem,
        );

        assert!(matches!(
            published,
            Err(GenerationPublicationError::NotCommitted(error))
                if error == "injected generated-root fsync failure"
        ));
        assert!(
            filesystem.failed,
            "test did not reach the injected fsync failure"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{old}\n"),
            "a failed publication left its candidate launchable"
        );
        assert_eq!(store.recover().unwrap().as_str(), old);
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{old}\n")
        );
    }

    #[test]
    fn cleanup_sync_failure_does_not_report_a_durable_pointer_as_uncommitted() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = FailOnceOnRootSyncFilesystem {
            root: root.path().into(),
            fail_on: 6,
            root_syncs: 0,
            failed: false,
            fail_unlink: false,
        };

        let outcome = store
            .publish_with_checkpoint_ids_and_filesystem(
                || Ok(publication(candidate, "candidate")),
                |_| Ok(()),
                || GenerationId::parse(candidate),
                &mut filesystem,
            )
            .unwrap();

        assert!(
            filesystem.failed,
            "test did not reach the cleanup fsync failure"
        );
        assert!(matches!(
            outcome,
            GenerationPublicationOutcome::CommittedWithCleanupPending { generation, .. }
                if generation.as_str() == candidate
        ));
        std::fs::write(
            root.path().join(format!(".committed-{candidate}")),
            "00000000000000000000000000000001\n",
        )
        .unwrap();
        assert_eq!(store.select_current().unwrap().as_str(), candidate);
        assert_eq!(store.recover().unwrap().as_str(), candidate);
    }

    #[test]
    fn cleanup_unlink_failure_returns_committed_pending_and_recovery_keeps_candidate() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = FailOnceOnRootSyncFilesystem {
            root: root.path().into(),
            fail_on: usize::MAX,
            root_syncs: 0,
            failed: false,
            fail_unlink: true,
        };

        let outcome = store
            .publish_with_checkpoint_ids_and_filesystem(
                || Ok(publication(candidate, "candidate")),
                |_| Ok(()),
                || GenerationId::parse(candidate),
                &mut filesystem,
            )
            .unwrap();
        assert!(matches!(
            outcome,
            GenerationPublicationOutcome::CommittedWithCleanupPending { generation, .. }
                if generation.as_str() == candidate
        ));
        assert_eq!(store.select_current().unwrap().as_str(), candidate);
        assert_eq!(store.recover().unwrap().as_str(), candidate);
    }

    fn assert_unconfirmed_ambiguous_outcome(
        outcome: GenerationPublicationOutcome,
        candidate: &str,
    ) {
        let GenerationPublicationOutcome::OutcomeAmbiguous {
            candidate: actual,
            cause,
        } = outcome
        else {
            panic!("expected an ambiguous outcome, got {outcome:?}");
        };
        assert_eq!(actual.as_str(), candidate);
        let cause = cause.to_ascii_lowercase();
        for success_claim in [
            "activated",
            "selected",
            "active",
            "current",
            "applied",
            "durable",
            "successful",
            "succeeded",
        ] {
            assert!(
                !cause.contains(success_claim),
                "ambiguous cause made a success-like claim `{success_claim}`: {cause}"
            );
        }
        assert!(
            cause.contains("unconfirmed"),
            "ambiguous cause did not describe the outcome as unconfirmed: {cause}"
        );
        assert!(
            cause.contains("injected generated-root fsync failure"),
            "ambiguous cause omitted the filesystem failure: {cause}"
        );
    }

    #[test]
    fn ambiguous_publication_marker_failure_never_claims_activation_success() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let candidate = "00000000000000000000000000000002";
        let mut filesystem = FailOnceOnRootSyncFilesystem {
            root: root.path().into(),
            fail_on: 5,
            root_syncs: 0,
            failed: false,
            fail_unlink: false,
        };

        let outcome = store
            .publish_with_checkpoint_ids_and_filesystem(
                || Ok(publication(candidate, "candidate")),
                |_| Ok(()),
                || GenerationId::parse(candidate),
                &mut filesystem,
            )
            .unwrap();

        assert!(filesystem.failed, "test did not reach the injected failure");
        assert_unconfirmed_ambiguous_outcome(outcome, candidate);
    }

    #[test]
    fn ambiguous_rollback_marker_failure_never_claims_rollback_success() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let current = "00000000000000000000000000000002";
        publish_at(
            &store,
            current,
            || Ok(publication(current, "current")),
            |_| Ok(()),
        )
        .unwrap();
        let candidate = "00000000000000000000000000000001";
        let mut filesystem = FailOnceOnRootSyncFilesystem {
            root: root.path().into(),
            fail_on: 3,
            root_syncs: 0,
            failed: false,
            fail_unlink: false,
        };

        let outcome = store
            .commit_existing_pointer_locked(
                GenerationId::parse(candidate).unwrap(),
                &mut filesystem,
            )
            .unwrap();

        assert!(filesystem.failed, "test did not reach the injected failure");
        assert_unconfirmed_ambiguous_outcome(outcome, candidate);
    }

    #[test]
    fn process_death_after_pointer_exchange_recovers_the_previous_selection() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let old = "00000000000000000000000000000001";
        let candidate = "00000000000000000000000000000002";

        let interrupted = publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "candidate")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PointerCommitted {
                    Err("simulated process death".into())
                } else {
                    Ok(())
                }
            },
        );

        assert!(interrupted.is_err());
        assert_eq!(store.recover().unwrap().as_str(), old);
        assert_eq!(
            std::fs::read_to_string(root.path().join("current")).unwrap(),
            format!("{old}\n")
        );
    }

    #[test]
    fn process_death_after_first_pointer_rename_recovers_an_absent_selection() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let candidate = "00000000000000000000000000000001";

        let interrupted = publish_at(
            &store,
            candidate,
            || Ok(publication(candidate, "candidate")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PointerCommitted {
                    Err("simulated process death".into())
                } else {
                    Ok(())
                }
            },
        );

        assert!(interrupted.is_err());
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");
        assert!(!root.path().join("current").exists());
    }

    #[test]
    fn recovered_pristine_orphan_does_not_block_a_fresh_publication() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let orphan = "00000000000000000000000000000001";
        let fresh = "00000000000000000000000000000002";

        let interrupted = publish_at(
            &store,
            orphan,
            || Ok(publication(orphan, "orphan")),
            |checkpoint| {
                if checkpoint == CommitCheckpoint::PointerCommitted {
                    Err("simulated process death".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(interrupted.is_err());
        assert_eq!(store.recover().unwrap_err(), "current generation is absent");

        let committed = publish_at(
            &store,
            fresh,
            || Ok(publication(fresh, "fresh")),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(committed.as_str(), fresh);
        assert_eq!(store.select_current().unwrap().as_str(), fresh);
        assert!(root.path().join("generations").join(orphan).is_dir());
        assert!(root.path().join("generations").join(fresh).is_dir());
    }

    #[test]
    fn malformed_unpointed_final_tree_blocks_pristine_publication() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let malformed = "00000000000000000000000000000001";
        let fresh = "00000000000000000000000000000002";
        let malformed_root = root.path().join("generations").join(malformed);
        std::fs::create_dir(&malformed_root).unwrap();
        std::fs::write(malformed_root.join("unexpected"), b"malformed").unwrap();

        assert!(publish_at(
            &store,
            fresh,
            || Ok(publication(fresh, "fresh")),
            |_| Ok(()),
        )
        .is_err());
        assert!(!root.path().join("current").exists());
        assert!(malformed_root.join("unexpected").is_file());
        assert!(!root.path().join("generations").join(fresh).exists());
    }

    #[test]
    fn recognized_pristine_staging_is_recovery_cleaned_before_fresh_publication() {
        let root = tempfile::tempdir().unwrap();
        secure_store_root(root.path());
        let store = GenerationStore::open(root.path()).unwrap();
        let interrupted = root
            .path()
            .join("generations/.staging-00000000000000000000000000000001");
        std::fs::create_dir(&interrupted).unwrap();
        std::fs::write(interrupted.join("partial"), b"unsealed").unwrap();
        let fresh = "00000000000000000000000000000002";

        let committed = publish_at(
            &store,
            fresh,
            || Ok(publication(fresh, "fresh")),
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(committed.as_str(), fresh);
        assert!(!interrupted.exists());
        assert_eq!(store.select_current().unwrap().as_str(), fresh);
    }

    #[test]
    fn generation_publication_rejects_a_non_adjacent_output_prefix_collision() {
        let digest = "0".repeat(64);
        let publication = GenerationPublication::new(
            [
                digest.clone(),
                digest.clone(),
                digest.clone(),
                digest.clone(),
                digest,
            ],
            vec![
                (String::from("a"), vec![1]),
                (String::from("a-"), vec![2]),
                (String::from("a/b"), vec![3]),
            ],
        );

        assert!(publication.is_err());
    }

    #[test]
    fn generation_store_allocates_a_fresh_id_after_collision_without_recapturing_inputs() {
        let root = tempfile::tempdir().unwrap();
        let store = seeded_store(root.path());
        let capture_count = AtomicUsize::new(0);
        let mut candidates = VecDeque::from([
            GenerationId::parse("00000000000000000000000000000001").unwrap(),
            GenerationId::parse("00000000000000000000000000000002").unwrap(),
        ]);

        let committed = store
            .publish_with_checkpoint_and_ids(
                || {
                    capture_count.fetch_add(1, Ordering::SeqCst);
                    Ok(publication(
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "captured-once",
                    ))
                },
                |_| Ok(()),
                || Ok(candidates.pop_front().unwrap()),
            )
            .unwrap();

        assert_eq!(capture_count.load(Ordering::SeqCst), 1);
        assert_eq!(committed.as_str(), "00000000000000000000000000000002");
        assert!(!root
            .path()
            .join("generations/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .exists());
        assert!(std::fs::read_to_string(
            root.path()
                .join("generations/00000000000000000000000000000002/receipt")
        )
        .unwrap()
        .starts_with("helm-generation-receipt-v1\ngeneration 00000000000000000000000000000002\n"));
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn v1_manifest_with_output(path: &str) -> String {
        let generation_id = "0123456789abcdef0123456789abcdef";
        let digest = "0".repeat(64);
        format!(
            "helm-generation-manifest-v1\ngeneration {generation_id}\npalette-sha256 {digest}\ncatalogue-sha256 {digest}\ntemplates-sha256 {digest}\nrenderer-sha256 {digest}\nlaunch-profile-sha256 {digest}\nreceipt-sha256 {digest}\noutput {path} {digest}\n"
        )
    }

    fn v1_fixture(
        base: &Path,
        generation_id: &str,
        output_path: &str,
        output: &[u8],
    ) -> (std::path::PathBuf, Manifest) {
        let generation = base.join(generation_id);
        std::fs::create_dir(&generation).unwrap();
        let input_digest = digest(b"input");
        let receipt = format!(
            "helm-generation-receipt-v1\ngeneration {generation_id}\npalette-sha256 {input_digest}\ncatalogue-sha256 {input_digest}\ntemplates-sha256 {input_digest}\nrenderer-sha256 {input_digest}\nlaunch-profile-sha256 {input_digest}\n"
        );
        let output_file = generation.join(output_path);
        std::fs::create_dir_all(output_file.parent().unwrap()).unwrap();
        std::fs::write(&output_file, output).unwrap();
        std::fs::write(generation.join("receipt"), &receipt).unwrap();
        let manifest = format!(
            "helm-generation-manifest-v1\ngeneration {generation_id}\npalette-sha256 {input_digest}\ncatalogue-sha256 {input_digest}\ntemplates-sha256 {input_digest}\nrenderer-sha256 {input_digest}\nlaunch-profile-sha256 {input_digest}\nreceipt-sha256 {}\noutput {output_path} {}\n",
            digest(receipt.as_bytes()),
            digest(output),
        );
        std::fs::write(generation.join("manifest"), &manifest).unwrap();
        std::fs::write(
            generation.join("seal"),
            format!("{}\n", digest(manifest.as_bytes())),
        )
        .unwrap();
        (generation, Manifest::parse(&manifest).unwrap())
    }

    #[test]
    fn v1_manifest_rejects_noncanonical_and_reserved_output_paths() {
        for path in [
            "café.ini",
            "theme file",
            "a/./b",
            "manifest",
            "receipt",
            "seal",
            "activation.lock",
            ".staging-0123456789abcdef0123456789abcdef",
        ] {
            assert!(
                Manifest::parse(&v1_manifest_with_output(path)).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn v1_manifest_verifies_its_receipt_seal_and_output() {
        let base = tempfile::tempdir().unwrap();
        let (generation, manifest) = v1_fixture(
            base.path(),
            "0123456789abcdef0123456789abcdef",
            "themes/theme.ini",
            b"sealed output",
        );
        manifest.verify(&generation).unwrap();
    }

    #[test]
    fn generation_id_rejects_paths_and_non_ascii() {
        assert!(GenerationId::parse("../old").is_err());
        assert!(GenerationId::parse("génération").is_err());
    }

    #[test]
    fn publication_order_v1_accepts_gaps_and_reencodes_in_generation_id_order() {
        let parsed = PublicationOrder::parse(
            b"helm-generation-publication-order-v1\nnext-sequence 9\ngeneration 00000000000000000000000000000001 8\ngeneration ffffffffffffffffffffffffffffffff 2\n",
        )
        .unwrap();

        assert_eq!(parsed.next_sequence, 9);
        assert_eq!(
            parsed.sequences[&GenerationId::parse("00000000000000000000000000000001").unwrap()],
            8
        );
        assert_eq!(
            parsed.encode(),
            b"helm-generation-publication-order-v1\nnext-sequence 9\ngeneration 00000000000000000000000000000001 8\ngeneration ffffffffffffffffffffffffffffffff 2\n"
        );
    }

    #[test]
    fn publication_order_v1_rejects_every_noncanonical_or_ambiguous_shape() {
        for malformed in [
            b"".as_slice(),
            b"helm-generation-publication-order-v1\nnext-sequence 1",
            b"helm-generation-publication-order-v1\r\nnext-sequence 1\r\n",
            b"helm-generation-publication-order-v2\nnext-sequence 1\n",
            b"helm-generation-publication-order-v1\nnext-sequence 0\n",
            b"helm-generation-publication-order-v1\nnext-sequence 01\n",
            b"helm-generation-publication-order-v1\nnext-sequence +1\n",
            b"helm-generation-publication-order-v1\nnext-sequence -1\n",
            b"helm-generation-publication-order-v1\nnext-sequence \t1\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 0\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 03\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 +1\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 -1\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 1\t\n",
            b"helm-generation-publication-order-v1\nnext-sequence 3\ngeneration 00000000000000000000000000000001 3\n",
            b"helm-generation-publication-order-v1\nnext-sequence 4\ngeneration 00000000000000000000000000000001 2\ngeneration 00000000000000000000000000000002 2\n",
            b"helm-generation-publication-order-v1\nnext-sequence 4\ngeneration 00000000000000000000000000000002 2\ngeneration 00000000000000000000000000000001 3\n",
            b"helm-generation-publication-order-v1\nnext-sequence 4\ngeneration 00000000000000000000000000000001 2 extra\n",
            b"helm-generation-publication-order-v1\nnext-sequence 4\n\n",
            b"\xff\n",
        ] {
            assert!(
                PublicationOrder::parse(malformed).is_err(),
                "accepted malformed publication order: {malformed:?}"
            );
        }
    }

    #[test]
    fn manifest_rejects_legacy_and_malformed_records() {
        assert!(Manifest::parse("b 00\na 00\n").is_err());
        assert!(Manifest::parse("a 00\na 00\n").is_err());
        assert!(Manifest::parse("../a 00\n").is_err());
        assert!(Manifest::parse("a//b 00\n").is_err());
        assert!(Manifest::parse("a not-a-sha256\n").is_err());
        assert!(Manifest::parse(
            "a 277089D91C0BDF4F2E6862BA7E4A07605119431F5D13F726DD352B06F1B206A9\n"
        )
        .is_err());
    }

    #[test]
    fn manifest_verification_rejects_changed_output_bytes() {
        let base = tempfile::tempdir().unwrap();
        let (generation, manifest) = v1_fixture(
            base.path(),
            "0123456789abcdef0123456789abcdef",
            "theme.ini",
            b"sealed output",
        );
        std::fs::write(generation.join("theme.ini"), "new bytes").unwrap();
        assert!(manifest.verify(&generation).is_err());
    }

    #[test]
    fn v1_verifier_rejects_unlisted_file_and_empty_directory() {
        let file_base = tempfile::tempdir().unwrap();
        let (file_generation, file_manifest) = v1_fixture(
            file_base.path(),
            "0123456789abcdef0123456789abcdef",
            "theme.ini",
            b"sealed output",
        );
        std::fs::write(file_generation.join("unexpected"), "not listed").unwrap();
        assert!(file_manifest.verify(&file_generation).is_err());

        let directory_base = tempfile::tempdir().unwrap();
        let (directory_generation, directory_manifest) = v1_fixture(
            directory_base.path(),
            "0123456789abcdef0123456789abcdef",
            "theme.ini",
            b"sealed output",
        );
        std::fs::create_dir(directory_generation.join("empty")).unwrap();
        assert!(directory_manifest.verify(&directory_generation).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_file() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("actual"), "bytes").unwrap();
        symlink("actual", directory.path().join("theme.ini")).unwrap();
        let root = GenerationRoot::open(directory.path()).unwrap();
        assert!(open_regular_file(&root, Path::new("theme.ini")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("outside")).unwrap();
        std::fs::write(directory.path().join("outside/theme.ini"), "bytes").unwrap();
        symlink("outside", directory.path().join("themes")).unwrap();
        let root = GenerationRoot::open(directory.path()).unwrap();
        assert!(open_regular_file(&root, Path::new("themes/theme.ini")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verification_rejects_a_symlinked_root_ancestor() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let real = base.path().join("real");
        let generation = real.join("generation");
        std::fs::create_dir_all(&generation).unwrap();
        std::fs::write(generation.join("theme.ini"), "bytes").unwrap();
        symlink(&real, base.path().join("link")).unwrap();
        assert!(GenerationRoot::open(&base.path().join("link/generation")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn held_descriptor_read_cannot_be_redirected_by_replacing_a_parent() {
        use std::io::Read;
        use std::os::unix::fs::symlink;

        let generation = tempfile::tempdir().unwrap();
        let themes = generation.path().join("themes");
        std::fs::create_dir(&themes).unwrap();
        std::fs::write(themes.join("theme.ini"), "sealed bytes").unwrap();
        let root = GenerationRoot::open(generation.path()).unwrap();
        let mut held = open_regular_file(&root, Path::new("themes/theme.ini")).unwrap();
        let moved = generation.path().join("held");
        let victim = tempfile::tempdir().unwrap();

        std::fs::rename(&themes, &moved).unwrap();
        symlink(victim.path(), &themes).unwrap();

        let mut bytes = String::new();
        held.read_to_string(&mut bytes).unwrap();
        assert_eq!(bytes, "sealed bytes");
        assert!(victim.path().read_dir().unwrap().next().is_none());
    }
}
