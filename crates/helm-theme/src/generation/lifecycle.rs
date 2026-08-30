use super::{
    canonical_boot_id, canonical_positive_u64, canonical_u32, canonical_u64,
    classify_lease_transfer_staging_locked, linux_boot_id, linux_process_identity, lower_hex_32,
    normalized_cgroup_path, open_directory_chain, record_value, validate_owned_mode, GenerationId,
    GenerationRoot, GenerationSelection, GenerationStore, LeaseFilesystem, LeaseRecord,
    LeaseTransferRecoveryPlan, LifecycleLeaseRecord, LifecycleOwnerKind, ParsedLeaseRecord,
    RealLeaseFilesystem, TransferCheckpoint,
};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rustix::fs::{
    flock, fsync, mkdirat, openat, renameat, renameat_with, statat, unlinkat, AtFlags, Dir,
    FileType, FlockOperation, Mode, OFlags, RenameFlags,
};
use rustix::io::Errno;

const MAX_RECORD_BYTES: usize = 4096;
const MAX_INVENTORY_ENTRIES: usize = 4096;
const MAX_INVENTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct ActivationRegistry {
    activation: GenerationRoot,
    launches: GenerationRoot,
    generation_leases: GenerationLeaseCapability,
    lock: OwnedFd,
    intra_process_lock: Mutex<()>,
    device: u64,
    inode: u64,
}

#[derive(Debug)]
struct GenerationLeaseCapability {
    generated_root: OwnedFd,
    leases: OwnedFd,
    activation_lock: OwnedFd,
    intra_process_lock: Arc<Mutex<()>>,
    root_identity: (u64, u64),
    leases_identity: (u64, u64),
    lock_identity: (u64, u64),
}

struct GenerationLeaseLock<'capability> {
    lock: &'capability OwnedFd,
    _intra_process: MutexGuard<'capability, ()>,
}

impl GenerationLeaseCapability {
    fn from_store(store: &GenerationStore) -> Result<Self, String> {
        let generated_root = store
            .root
            .fd
            .try_clone()
            .map_err(|error| error.to_string())?;
        let leases = store
            .leases
            .fd
            .try_clone()
            .map_err(|error| error.to_string())?;
        let activation_lock = store.lock.try_clone().map_err(|error| error.to_string())?;
        let root_stat = rustix::fs::fstat(&generated_root).map_err(|error| error.to_string())?;
        let leases_stat = rustix::fs::fstat(&leases).map_err(|error| error.to_string())?;
        let lock_stat = rustix::fs::fstat(&activation_lock).map_err(|error| error.to_string())?;
        let capability = Self {
            generated_root,
            leases,
            activation_lock,
            intra_process_lock: Arc::clone(&store.intra_process_lock),
            root_identity: (root_stat.st_dev, root_stat.st_ino),
            leases_identity: (leases_stat.st_dev, leases_stat.st_ino),
            lock_identity: (lock_stat.st_dev, lock_stat.st_ino),
        };
        capability.revalidate()?;
        Ok(capability)
    }

    fn revalidate(&self) -> Result<(), String> {
        let uid = rustix::process::getuid().as_raw();
        let root_stat =
            rustix::fs::fstat(&self.generated_root).map_err(|error| error.to_string())?;
        let leases_stat = rustix::fs::fstat(&self.leases).map_err(|error| error.to_string())?;
        let lock_stat =
            rustix::fs::fstat(&self.activation_lock).map_err(|error| error.to_string())?;
        validate_owned_mode(
            "generated root",
            &root_stat,
            FileType::Directory,
            0o700,
            uid,
        )?;
        validate_owned_mode(
            "leases directory",
            &leases_stat,
            FileType::Directory,
            0o700,
            uid,
        )?;
        validate_owned_mode(
            "generation activation lock",
            &lock_stat,
            FileType::RegularFile,
            0o600,
            uid,
        )?;
        if (root_stat.st_dev, root_stat.st_ino) != self.root_identity
            || (leases_stat.st_dev, leases_stat.st_ino) != self.leases_identity
            || (lock_stat.st_dev, lock_stat.st_ino) != self.lock_identity
        {
            return Err("generation lease capability identity changed".into());
        }
        revalidate_opened_path(&self.generated_root, "leases", &self.leases)?;
        revalidate_opened_path(
            &self.generated_root,
            "activation.lock",
            &self.activation_lock,
        )
    }

    fn lock_shared(&self) -> Result<GenerationLeaseLock<'_>, String> {
        let intra_process = self
            .intra_process_lock
            .lock()
            .map_err(|_| "generation store lock is poisoned")?;
        flock(&self.activation_lock, FlockOperation::LockShared)
            .map_err(|error| error.to_string())?;
        if let Err(error) = self.revalidate() {
            let _ = flock(&self.activation_lock, FlockOperation::Unlock);
            return Err(error);
        }
        Ok(GenerationLeaseLock {
            lock: &self.activation_lock,
            _intra_process: intra_process,
        })
    }
}

#[derive(Debug)]
struct RegistryLock<'registry> {
    lock: &'registry OwnedFd,
    _intra_process: MutexGuard<'registry, ()>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionClaim {
    record: SessionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSessionCapability {
    record: SessionRecord,
    registry_device: u64,
    registry_inode: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionClaimRequest {
    session: SessionId,
    entry_pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PrepareLaunch {
    launch: LaunchId,
    mode: OwnershipMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedLaunch {
    record: LaunchRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AdoptedLaunch {
    record: LaunchRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VerifiedOwnership {
    Direct {
        binding: OwnershipBinding,
    },
    Systemd {
        binding: OwnershipBinding,
        unit_invocation: String,
        cgroup: String,
        cgroup_device: u64,
        cgroup_inode: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnershipBinding {
    launch: LaunchId,
    session: SessionId,
    generation: GenerationId,
    manifest_sha256: String,
    lease: String,
    owner_uid: u32,
    boot_id: String,
    owner_pid: u32,
    owner_start_time: u64,
    mode: OwnershipMode,
    unit: String,
    process_group: u32,
}

impl OwnershipBinding {
    fn from_preparing(record: &LaunchRecord) -> Self {
        Self {
            launch: record.launch,
            session: record.session,
            generation: record.generation.clone(),
            manifest_sha256: record.manifest_sha256.clone(),
            lease: record.lease.clone(),
            owner_uid: record.owner_uid,
            boot_id: record.boot_id.clone(),
            owner_pid: record.owner_pid,
            owner_start_time: record.owner_start_time,
            mode: record.mode,
            unit: record.unit.clone(),
            process_group: record.process_group,
        }
    }
}

mod ownership_verifier {
    pub(super) trait Sealed {}
}

trait OwnershipVerifier: ownership_verifier::Sealed {
    fn verify(&self, prepared: &PreparedLaunch) -> Result<VerifiedOwnership, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedSystemd {
    record: LaunchRecord,
    lease: Option<LifecycleLeaseRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemdObservation {
    ExactRecursiveEmpty,
    ExactLive,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectObservation {
    Exact {
        owner_written_witness: bool,
        exact_owner_stale: bool,
        recorded_group_empty: bool,
    },
    Detached,
    Live,
    Uncertain,
}

mod ownership_inspector {
    pub(super) trait Sealed {}
}

trait OwnershipInspector: ownership_inspector::Sealed {
    fn inspect_systemd(&self, ownership: &VerifiedSystemd) -> SystemdObservation;
    fn inspect_direct(&self, record: &LaunchRecord) -> DirectObservation;
}

mod gate_closed_controller {
    pub(super) trait Sealed {}
}

trait GateClosedController: gate_closed_controller::Sealed {
    fn abort_unadopted_direct(&self, record: &LaunchRecord) -> DirectObservation;
    fn abort_unadopted_systemd(&self, record: &LaunchRecord) -> SystemdObservation;
    fn abort_adopted_systemd(&self, ownership: &VerifiedSystemd) -> SystemdObservation;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ReconciliationReport {
    terminalized: usize,
    released: usize,
    collected: usize,
    retained: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconciliationCheckpoint {
    AfterTerminalRecordFsync,
    BeforeLeaseRetire,
    AfterLeaseRetire,
    AfterLeaseUnlink,
    AfterLeaseDirectoryFsync,
    BeforeRecordRemoval,
    BeforeLaunchRetire,
    AfterLaunchRetire,
}

trait ReconciliationFilesystem {
    fn checkpoint(&mut self, checkpoint: ReconciliationCheckpoint) -> Result<(), String>;
}

struct RealReconciliationFilesystem;

impl ReconciliationFilesystem for RealReconciliationFilesystem {
    fn checkpoint(&mut self, _checkpoint: ReconciliationCheckpoint) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug)]
struct ValidatedReconciliationLease {
    record: ParsedLeaseRecord,
    descriptor: OwnedFd,
    storage_name: String,
    retired: bool,
}

#[derive(Debug)]
struct ValidatedReconciliationLaunch {
    record: LaunchRecord,
    descriptor: OwnedFd,
    storage_name: String,
    retired: bool,
}

#[derive(Debug)]
enum ReconciliationLease {
    Missing,
    Valid(Box<ValidatedReconciliationLease>),
    Uncertain,
}

impl ReconciliationLease {
    fn process(&self) -> Option<&LeaseRecord> {
        match self {
            Self::Valid(validated) => match &validated.record {
                ParsedLeaseRecord::Process(record) => Some(record),
                ParsedLeaseRecord::Lifecycle(_) => None,
            },
            Self::Missing | Self::Uncertain => None,
        }
    }

    fn lifecycle(&self) -> Option<&LifecycleLeaseRecord> {
        match self {
            Self::Valid(validated) => match &validated.record {
                ParsedLeaseRecord::Lifecycle(record) => Some(record),
                ParsedLeaseRecord::Process(_) => None,
            },
            Self::Missing | Self::Uncertain => None,
        }
    }
}

#[derive(Debug, Clone)]
enum ReconciliationControllerAction {
    UnadoptedDirect,
    UnadoptedSystemd,
    AdoptedSystemd(Box<VerifiedSystemd>),
}

#[derive(Debug, Clone)]
enum ReconciliationDurableAction {
    Retain,
    TerminalizeAndRetain(LaunchResult),
    Complete {
        terminal_result: Option<LaunchResult>,
        transferred: Option<LifecycleLeaseRecord>,
        release: bool,
    },
}

#[derive(Debug)]
struct PlannedReconciliation {
    launch: ValidatedReconciliationLaunch,
    lease: ReconciliationLease,
    controller: Option<ReconciliationControllerAction>,
    durable: ReconciliationDurableAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionId([u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LaunchId([u8; 16]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Claimed,
    Preparing,
    Active,
    AdmissionFrozen,
    HelpersStopped,
    CleanupDelegated,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HelperMode {
    None,
    Systemd,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionRecord {
    session: SessionId,
    sequence: u64,
    state: SessionState,
    owner_uid: u32,
    boot_id: String,
    entry_pid: u32,
    entry_start_time: u64,
    compositor_pid: u32,
    compositor_start_time: u64,
    helper_mode: HelperMode,
    wm_pid: u32,
    wm_start_time: u64,
    wm_process_group: u32,
    bar_pid: u32,
    bar_start_time: u64,
    bar_process_group: u32,
}

/// The immutable ownership strategy selected before launch preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipMode {
    /// A directly supervised process group whose id is the owner PID.
    Direct,
    /// A deterministic transient systemd scope.
    Systemd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchState {
    Preparing,
    Adopted,
    Running,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchResult {
    None,
    Exited,
    Failed,
    Lost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseKind {
    Process,
    Lifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LaunchRecord {
    launch: LaunchId,
    session: SessionId,
    sequence: u64,
    state: LaunchState,
    result: LaunchResult,
    owner_uid: u32,
    boot_id: String,
    generation: GenerationId,
    manifest_sha256: String,
    lease: String,
    lease_kind: LeaseKind,
    owner_pid: u32,
    owner_start_time: u64,
    mode: OwnershipMode,
    unit: String,
    unit_invocation: String,
    process_group: u32,
    cgroup: String,
    cgroup_device: u64,
    cgroup_inode: u64,
    exec_open: bool,
    direct_drained: bool,
}

impl SessionId {
    fn parse(value: &str) -> Result<Self, String> {
        parse_id(value, "session id").map(Self)
    }

    fn encode(self) -> String {
        encode_id(self.0)
    }
}

impl LaunchId {
    fn parse(value: &str) -> Result<Self, String> {
        parse_id(value, "launch id").map(Self)
    }

    fn encode(self) -> String {
        encode_id(self.0)
    }
}

impl SessionRecord {
    fn claimed(
        session: SessionId,
        entry_pid: u32,
        entry_start_time: u64,
        owner_uid: u32,
        boot_id: String,
    ) -> Self {
        Self {
            session,
            sequence: 1,
            state: SessionState::Claimed,
            owner_uid,
            boot_id,
            entry_pid,
            entry_start_time,
            compositor_pid: 0,
            compositor_start_time: 0,
            helper_mode: HelperMode::None,
            wm_pid: 0,
            wm_start_time: 0,
            wm_process_group: 0,
            bar_pid: 0,
            bar_start_time: 0,
            bar_process_group: 0,
        }
    }

    fn encode(&self) -> Vec<u8> {
        format!(
            "helm-activation-session-v1\nsession {}\nsequence {}\nstate {}\nowner-uid {}\nboot-id {}\nentry-pid {}\nentry-start-time {}\ncompositor-pid {}\ncompositor-start-time {}\nhelper-mode {}\nwm-pid {}\nwm-start-time {}\nwm-process-group {}\nbar-pid {}\nbar-start-time {}\nbar-process-group {}\n",
            self.session.encode(),
            self.sequence,
            session_state_name(self.state),
            self.owner_uid,
            self.boot_id,
            self.entry_pid,
            self.entry_start_time,
            self.compositor_pid,
            self.compositor_start_time,
            helper_mode_name(self.helper_mode),
            self.wm_pid,
            self.wm_start_time,
            self.wm_process_group,
            self.bar_pid,
            self.bar_start_time,
            self.bar_process_group,
        )
        .into_bytes()
    }

    fn parse(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > MAX_RECORD_BYTES {
            return Err("session record exceeds 4096 bytes".into());
        }
        let text = std::str::from_utf8(raw).map_err(|_| "session record is not UTF-8")?;
        if text.contains('\r') || !text.ends_with('\n') {
            return Err("session record must use LF-terminated lines".into());
        }
        let mut lines = text[..text.len() - 1].split('\n');
        if lines.next() != Some("helm-activation-session-v1") {
            return Err("unsupported session record version".into());
        }
        let session = SessionId::parse(record_value(&mut lines, "session")?)?;
        let sequence =
            canonical_positive_u64(record_value(&mut lines, "sequence")?, "session sequence")?;
        let state = match record_value(&mut lines, "state")? {
            "claimed" => SessionState::Claimed,
            "preparing" => SessionState::Preparing,
            "active" => SessionState::Active,
            "admission-frozen" => SessionState::AdmissionFrozen,
            "helpers-stopped" => SessionState::HelpersStopped,
            "cleanup-delegated" => SessionState::CleanupDelegated,
            "closed" => SessionState::Closed,
            _ => return Err("session state is malformed".into()),
        };
        let owner_uid = canonical_u32(record_value(&mut lines, "owner-uid")?, "session owner UID")?;
        let boot_id = record_value(&mut lines, "boot-id")?;
        if !canonical_boot_id(boot_id) {
            return Err("session boot ID is malformed".into());
        }
        let entry_pid = canonical_u32(record_value(&mut lines, "entry-pid")?, "session entry PID")?;
        if entry_pid == 0 {
            return Err("session entry PID must be positive".into());
        }
        let entry_start_time = canonical_positive_u64(
            record_value(&mut lines, "entry-start-time")?,
            "session entry start time",
        )?;
        let compositor_pid = canonical_u32(
            record_value(&mut lines, "compositor-pid")?,
            "session compositor PID",
        )?;
        let compositor_start_time = canonical_u64(
            record_value(&mut lines, "compositor-start-time")?,
            "session compositor start time",
        )?;
        let helper_mode = match record_value(&mut lines, "helper-mode")? {
            "none" => HelperMode::None,
            "systemd" => HelperMode::Systemd,
            "direct" => HelperMode::Direct,
            _ => return Err("session helper mode is malformed".into()),
        };
        let wm_pid = canonical_u32(record_value(&mut lines, "wm-pid")?, "session WM PID")?;
        let wm_start_time = canonical_u64(
            record_value(&mut lines, "wm-start-time")?,
            "session WM start time",
        )?;
        let wm_process_group = canonical_u32(
            record_value(&mut lines, "wm-process-group")?,
            "session WM process group",
        )?;
        let bar_pid = canonical_u32(record_value(&mut lines, "bar-pid")?, "session bar PID")?;
        let bar_start_time = canonical_u64(
            record_value(&mut lines, "bar-start-time")?,
            "session bar start time",
        )?;
        let bar_process_group = canonical_u32(
            record_value(&mut lines, "bar-process-group")?,
            "session bar process group",
        )?;
        if lines.next().is_some() {
            return Err("session record has extra or duplicate fields".into());
        }

        if (compositor_pid == 0) != (compositor_start_time == 0) {
            return Err("session compositor identity is incomplete".into());
        }
        if matches!(state, SessionState::Claimed)
            && (compositor_pid != 0
                || compositor_start_time != 0
                || helper_mode != HelperMode::None)
        {
            return Err("claimed session contains unpublished identities".into());
        }
        if matches!(state, SessionState::Preparing | SessionState::Active)
            && (compositor_pid == 0
                || compositor_start_time == 0
                || helper_mode == HelperMode::None)
        {
            return Err("preparing or active session lacks published identities".into());
        }
        if (compositor_pid == 0) != (helper_mode == HelperMode::None) {
            return Err("session helper mode lacks its compositor publication boundary".into());
        }
        let wm_complete = identity_triple_valid(wm_pid, wm_start_time, wm_process_group);
        let bar_complete = identity_triple_valid(bar_pid, bar_start_time, bar_process_group);
        match helper_mode {
            HelperMode::None => {
                if wm_pid != 0
                    || wm_start_time != 0
                    || wm_process_group != 0
                    || bar_pid != 0
                    || bar_start_time != 0
                    || bar_process_group != 0
                {
                    return Err("helper-mode none contains helper identities".into());
                }
                if matches!(state, SessionState::Active) {
                    return Err("active session cannot have helper-mode none".into());
                }
            }
            HelperMode::Systemd => {
                if wm_pid != 0
                    || wm_start_time != 0
                    || wm_process_group != 0
                    || bar_pid != 0
                    || bar_start_time != 0
                    || bar_process_group != 0
                {
                    return Err("systemd helper mode contains direct identities".into());
                }
            }
            HelperMode::Direct => {
                if !wm_complete || !bar_complete {
                    if matches!(state, SessionState::Active) {
                        return Err("active direct session lacks helper identities".into());
                    }
                    if !identity_triple_absent_or_complete(wm_pid, wm_start_time, wm_process_group)
                        || !identity_triple_absent_or_complete(
                            bar_pid,
                            bar_start_time,
                            bar_process_group,
                        )
                    {
                        return Err("direct helper identity is incomplete".into());
                    }
                }
            }
        }

        Ok(Self {
            session,
            sequence,
            state,
            owner_uid,
            boot_id: boot_id.into(),
            entry_pid,
            entry_start_time,
            compositor_pid,
            compositor_start_time,
            helper_mode,
            wm_pid,
            wm_start_time,
            wm_process_group,
            bar_pid,
            bar_start_time,
            bar_process_group,
        })
    }
}

fn session_state_name(state: SessionState) -> &'static str {
    match state {
        SessionState::Claimed => "claimed",
        SessionState::Preparing => "preparing",
        SessionState::Active => "active",
        SessionState::AdmissionFrozen => "admission-frozen",
        SessionState::HelpersStopped => "helpers-stopped",
        SessionState::CleanupDelegated => "cleanup-delegated",
        SessionState::Closed => "closed",
    }
}

fn helper_mode_name(mode: HelperMode) -> &'static str {
    match mode {
        HelperMode::None => "none",
        HelperMode::Systemd => "systemd",
        HelperMode::Direct => "direct",
    }
}

fn identity_triple_valid(pid: u32, start_time: u64, process_group: u32) -> bool {
    pid > 0 && start_time > 0 && process_group == pid
}

fn identity_triple_absent_or_complete(pid: u32, start_time: u64, process_group: u32) -> bool {
    (pid == 0 && start_time == 0 && process_group == 0)
        || identity_triple_valid(pid, start_time, process_group)
}

impl LaunchRecord {
    fn encode(&self) -> Vec<u8> {
        format!(
            "helm-activation-launch-v1\nlaunch {}\nsession {}\nsequence {}\nstate {}\nresult {}\nowner-uid {}\nboot-id {}\ngeneration {}\nmanifest-sha256 {}\nlease {}\nlease-kind {}\nowner-pid {}\nowner-start-time {}\nowner-kind {}\nunit {}\nunit-invocation {}\nprocess-group {}\ncgroup {}\ncgroup-device {}\ncgroup-inode {}\nexec-open {}\ndirect-drained {}\n",
            self.launch.encode(),
            self.session.encode(),
            self.sequence,
            launch_state_name(self.state),
            launch_result_name(self.result),
            self.owner_uid,
            self.boot_id,
            self.generation.as_str(),
            self.manifest_sha256,
            self.lease,
            lease_kind_name(self.lease_kind),
            self.owner_pid,
            self.owner_start_time,
            ownership_mode_name(self.mode),
            self.unit,
            self.unit_invocation,
            self.process_group,
            self.cgroup,
            self.cgroup_device,
            self.cgroup_inode,
            yes_no(self.exec_open),
            yes_no(self.direct_drained),
        )
        .into_bytes()
    }

    fn parse(raw: &[u8]) -> Result<Self, String> {
        if raw.len() > MAX_RECORD_BYTES {
            return Err("launch record exceeds 4096 bytes".into());
        }
        let text = std::str::from_utf8(raw).map_err(|_| "launch record is not UTF-8")?;
        if text.contains('\r') || !text.ends_with('\n') {
            return Err("launch record must use LF-terminated lines".into());
        }
        let mut lines = text[..text.len() - 1].split('\n');
        if lines.next() != Some("helm-activation-launch-v1") {
            return Err("unsupported launch record version".into());
        }

        let launch = LaunchId::parse(record_value(&mut lines, "launch")?)?;
        let session = SessionId::parse(record_value(&mut lines, "session")?)?;
        let sequence =
            canonical_positive_u64(record_value(&mut lines, "sequence")?, "launch sequence")?;
        let state = match record_value(&mut lines, "state")? {
            "preparing" => LaunchState::Preparing,
            "adopted" => LaunchState::Adopted,
            "running" => LaunchState::Running,
            "terminal" => LaunchState::Terminal,
            _ => return Err("launch state is malformed".into()),
        };
        let result = match record_value(&mut lines, "result")? {
            "none" => LaunchResult::None,
            "exited" => LaunchResult::Exited,
            "failed" => LaunchResult::Failed,
            "lost" => LaunchResult::Lost,
            _ => return Err("launch result is malformed".into()),
        };
        let owner_uid = canonical_u32(record_value(&mut lines, "owner-uid")?, "launch owner UID")?;
        let boot_id = record_value(&mut lines, "boot-id")?;
        if !canonical_boot_id(boot_id) {
            return Err("launch boot ID is malformed".into());
        }
        let generation = GenerationId::parse(record_value(&mut lines, "generation")?)?;
        let manifest_sha256 = record_value(&mut lines, "manifest-sha256")?;
        if !lower_hex(manifest_sha256, 64) {
            return Err("launch manifest digest is malformed".into());
        }
        let lease = record_value(&mut lines, "lease")?;
        if !lower_hex_32(lease) {
            return Err("launch lease id is malformed".into());
        }
        let lease_kind = match record_value(&mut lines, "lease-kind")? {
            "process" => LeaseKind::Process,
            "lifecycle" => LeaseKind::Lifecycle,
            _ => return Err("launch lease kind is malformed".into()),
        };
        let owner_pid = canonical_u32(record_value(&mut lines, "owner-pid")?, "launch owner PID")?;
        if owner_pid == 0 {
            return Err("launch owner PID must be positive".into());
        }
        let owner_start_time = canonical_positive_u64(
            record_value(&mut lines, "owner-start-time")?,
            "launch owner start time",
        )?;
        let mode = match record_value(&mut lines, "owner-kind")? {
            "process-group" => OwnershipMode::Direct,
            "systemd-scope" => OwnershipMode::Systemd,
            _ => return Err("launch owner kind is malformed".into()),
        };
        let unit = record_value(&mut lines, "unit")?;
        let unit_invocation = record_value(&mut lines, "unit-invocation")?;
        let process_group = canonical_u32(
            record_value(&mut lines, "process-group")?,
            "launch process group",
        )?;
        let cgroup = record_value(&mut lines, "cgroup")?;
        let cgroup_device = canonical_u64(
            record_value(&mut lines, "cgroup-device")?,
            "launch cgroup device",
        )?;
        let cgroup_inode = canonical_u64(
            record_value(&mut lines, "cgroup-inode")?,
            "launch cgroup inode",
        )?;
        let exec_open = parse_yes_no(record_value(&mut lines, "exec-open")?, "exec-open")?;
        let direct_drained = parse_yes_no(
            record_value(&mut lines, "direct-drained")?,
            "direct-drained",
        )?;
        if lines.next().is_some() {
            return Err("launch record has extra or duplicate fields".into());
        }

        if !producer_reachable_launch_tuple(
            state,
            result,
            lease_kind,
            mode,
            exec_open,
            direct_drained,
        ) {
            return Err("launch state tuple is not producer-reachable".into());
        }

        match mode {
            OwnershipMode::Direct => {
                if unit != "none"
                    || unit_invocation != "none"
                    || process_group != owner_pid
                    || cgroup != "none"
                    || cgroup_device != 0
                    || cgroup_inode != 0
                {
                    return Err("direct launch ownership is malformed".into());
                }
                if direct_drained
                    && !(matches!(state, LaunchState::Terminal)
                        && matches!(result, LaunchResult::Exited))
                {
                    return Err("direct drain witness lacks terminal owner evidence".into());
                }
            }
            OwnershipMode::Systemd => {
                if unit != format!("helm-launch-{}.scope", launch.encode())
                    || process_group != 0
                    || direct_drained
                {
                    return Err("systemd launch ownership is malformed".into());
                }
                let pre_adoption = matches!(state, LaunchState::Preparing)
                    || (matches!(state, LaunchState::Terminal)
                        && matches!(result, LaunchResult::Failed)
                        && lease_kind == LeaseKind::Process
                        && !exec_open);
                if pre_adoption {
                    if unit_invocation != "none"
                        || cgroup != "none"
                        || cgroup_device != 0
                        || cgroup_inode != 0
                    {
                        return Err("preparing systemd launch contains adopted identity".into());
                    }
                } else if !lower_hex_32(unit_invocation)
                    || !normalized_cgroup_path(cgroup)
                    || cgroup_device == 0
                    || cgroup_inode == 0
                {
                    return Err("adopted systemd launch identity is malformed".into());
                }
            }
        }

        Ok(Self {
            launch,
            session,
            sequence,
            state,
            result,
            owner_uid,
            boot_id: boot_id.into(),
            generation,
            manifest_sha256: manifest_sha256.into(),
            lease: lease.into(),
            lease_kind,
            owner_pid,
            owner_start_time,
            mode,
            unit: unit.into(),
            unit_invocation: unit_invocation.into(),
            process_group,
            cgroup: cgroup.into(),
            cgroup_device,
            cgroup_inode,
            exec_open,
            direct_drained,
        })
    }
}

fn launch_state_name(state: LaunchState) -> &'static str {
    match state {
        LaunchState::Preparing => "preparing",
        LaunchState::Adopted => "adopted",
        LaunchState::Running => "running",
        LaunchState::Terminal => "terminal",
    }
}

fn producer_reachable_launch_tuple(
    state: LaunchState,
    result: LaunchResult,
    lease_kind: LeaseKind,
    mode: OwnershipMode,
    exec_open: bool,
    direct_drained: bool,
) -> bool {
    match (state, lease_kind, mode, result) {
        (LaunchState::Preparing, LeaseKind::Process, _, LaunchResult::None) => {
            !exec_open && !direct_drained
        }
        (LaunchState::Adopted, LeaseKind::Lifecycle, _, LaunchResult::None) => !direct_drained,
        (LaunchState::Running, LeaseKind::Lifecycle, _, LaunchResult::None) => {
            exec_open && !direct_drained
        }
        (LaunchState::Terminal, LeaseKind::Process, _, LaunchResult::Failed) => {
            !exec_open && !direct_drained
        }
        (
            LaunchState::Terminal,
            LeaseKind::Lifecycle,
            OwnershipMode::Systemd,
            LaunchResult::Failed,
        ) => !direct_drained,
        (
            LaunchState::Terminal,
            LeaseKind::Lifecycle,
            OwnershipMode::Systemd,
            LaunchResult::Exited,
        ) => exec_open && !direct_drained,
        (
            LaunchState::Terminal,
            LeaseKind::Lifecycle,
            OwnershipMode::Direct,
            LaunchResult::Exited,
        ) => exec_open && direct_drained,
        (
            LaunchState::Terminal,
            LeaseKind::Lifecycle,
            OwnershipMode::Direct,
            LaunchResult::Lost,
        ) => exec_open && !direct_drained,
        _ => false,
    }
}

fn launch_result_name(result: LaunchResult) -> &'static str {
    match result {
        LaunchResult::None => "none",
        LaunchResult::Exited => "exited",
        LaunchResult::Failed => "failed",
        LaunchResult::Lost => "lost",
    }
}

fn lease_kind_name(kind: LeaseKind) -> &'static str {
    match kind {
        LeaseKind::Process => "process",
        LeaseKind::Lifecycle => "lifecycle",
    }
}

fn ownership_mode_name(mode: OwnershipMode) -> &'static str {
    match mode {
        OwnershipMode::Direct => "process-group",
        OwnershipMode::Systemd => "systemd-scope",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn read_reconciliation_lease(
    capability: &GenerationLeaseCapability,
    name: &str,
) -> ReconciliationLease {
    let retirement = format!(".lease-retire-{name}");
    let canonical_exists = match statat(&capability.leases, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => true,
        Err(Errno::NOENT) => false,
        Err(_) => return ReconciliationLease::Uncertain,
    };
    let retirement_exists = match statat(
        &capability.leases,
        retirement.as_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(_) => true,
        Err(Errno::NOENT) => false,
        Err(_) => return ReconciliationLease::Uncertain,
    };
    match (canonical_exists, retirement_exists) {
        (false, false) => ReconciliationLease::Missing,
        (true, true) => ReconciliationLease::Uncertain,
        (true, false) => read_reconciliation_lease_at(capability, name, false),
        (false, true) => read_reconciliation_lease_at(capability, &retirement, true),
    }
}

fn read_reconciliation_lease_at(
    capability: &GenerationLeaseCapability,
    storage_name: &str,
    retired: bool,
) -> ReconciliationLease {
    let descriptor = match openat(
        &capability.leases,
        storage_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(_) => return ReconciliationLease::Uncertain,
    };
    let stat = match rustix::fs::fstat(&descriptor) {
        Ok(stat) => stat,
        Err(_) => return ReconciliationLease::Uncertain,
    };
    if validate_owned_mode(
        "generation lease",
        &stat,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )
    .is_err()
        || stat.st_size > MAX_RECORD_BYTES as i64
    {
        return ReconciliationLease::Uncertain;
    }
    let mut file = std::fs::File::from(descriptor);
    let mut raw = Vec::new();
    if Read::by_ref(&mut file)
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .is_err()
    {
        return ReconciliationLease::Uncertain;
    }
    let Ok(record) = ParsedLeaseRecord::parse(&raw) else {
        return ReconciliationLease::Uncertain;
    };
    ReconciliationLease::Valid(Box::new(ValidatedReconciliationLease {
        record,
        descriptor: file.into(),
        storage_name: storage_name.to_owned(),
        retired,
    }))
}

fn read_reconciliation_launch(
    launches: &OwnedFd,
    storage_name: &str,
) -> Result<ValidatedReconciliationLaunch, String> {
    let (name, retired) = match storage_name.strip_prefix(".launch-retire-") {
        Some(name) => (name, true),
        None => (storage_name, false),
    };
    LaunchId::parse(name)?;
    let descriptor = openat(
        launches,
        storage_name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let stat = rustix::fs::fstat(&descriptor).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "launch record",
        &stat,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if stat.st_size > MAX_RECORD_BYTES as i64 {
        return Err("launch record exceeds 4096 bytes".into());
    }
    let mut file = std::fs::File::from(descriptor);
    let mut raw = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    let record = LaunchRecord::parse(&raw)?;
    if record.launch.encode() != name {
        return Err("launch filename and record id disagree".into());
    }
    Ok(ValidatedReconciliationLaunch {
        record,
        descriptor: file.into(),
        storage_name: storage_name.to_owned(),
        retired,
    })
}

fn process_lease_matches(record: &LaunchRecord, lease: &LeaseRecord) -> bool {
    lease.generation == record.generation
        && lease.pid == record.owner_pid
        && lease.start_time == record.owner_start_time
        && lease.boot_id == record.boot_id
        && lease.owner_uid == record.owner_uid
}

fn lifecycle_lease_matches(record: &LaunchRecord, lease: &LifecycleLeaseRecord) -> bool {
    if lease.generation != record.generation
        || lease.launch.as_str() != record.launch.encode()
        || lease.pid != record.owner_pid
        || lease.start_time != record.owner_start_time
        || lease.boot_id != record.boot_id
        || lease.owner_uid != record.owner_uid
    {
        return false;
    }
    match (record.mode, lease.owner_kind) {
        (OwnershipMode::Direct, LifecycleOwnerKind::ProcessGroup) => {
            lease.unit == "none"
                && lease.unit_invocation == "none"
                && lease.process_group == record.process_group
                && lease.cgroup == "none"
                && lease.cgroup_device == 0
                && lease.cgroup_inode == 0
        }
        (OwnershipMode::Systemd, LifecycleOwnerKind::SystemdScope) => {
            lease.unit == record.unit
                && lease.process_group == 0
                && (record.state == LaunchState::Preparing
                    || (lease.unit_invocation == record.unit_invocation
                        && lease.cgroup == record.cgroup
                        && lease.cgroup_device == record.cgroup_device
                        && lease.cgroup_inode == record.cgroup_inode))
        }
        _ => false,
    }
}

fn direct_empty_after_abort(observation: DirectObservation) -> bool {
    matches!(
        observation,
        DirectObservation::Exact {
            exact_owner_stale: true,
            recorded_group_empty: true,
            ..
        }
    )
}

fn direct_terminal_releasable(record: &LaunchRecord, observation: DirectObservation) -> bool {
    record.direct_drained
        && matches!(
            observation,
            DirectObservation::Exact {
                owner_written_witness: true,
                exact_owner_stale: true,
                recorded_group_empty: true,
            }
        )
}

fn systemd_empty(observation: SystemdObservation) -> bool {
    observation == SystemdObservation::ExactRecursiveEmpty
}

fn reconciliation_lease_relation_is_known(
    record: &LaunchRecord,
    lease: &ReconciliationLease,
) -> bool {
    match lease {
        ReconciliationLease::Missing => matches!(
            (record.state, record.lease_kind),
            (LaunchState::Preparing, LeaseKind::Process) | (LaunchState::Terminal, _)
        ),
        ReconciliationLease::Uncertain => false,
        ReconciliationLease::Valid(_) => {
            lease.process().is_some_and(|process| {
                process_lease_matches(record, process)
                    && matches!(
                        (record.state, record.lease_kind),
                        (
                            LaunchState::Preparing | LaunchState::Terminal,
                            LeaseKind::Process
                        )
                    )
            }) || lease.lifecycle().is_some_and(|lifecycle| {
                lifecycle_lease_matches(record, lifecycle)
                    && (record.lease_kind == LeaseKind::Lifecycle
                        || (record.state == LaunchState::Preparing
                            && record.lease_kind == LeaseKind::Process))
            })
        }
    }
}

fn reconciliation_lease_inventory_is_safe(
    capability: &GenerationLeaseCapability,
    records: &[ValidatedReconciliationLaunch],
    names: &[OsString],
) -> bool {
    for (index, record) in records.iter().enumerate() {
        if records[index + 1..]
            .iter()
            .any(|other| other.record.lease == record.record.lease)
        {
            return false;
        }
    }

    let mut logical_names = Vec::<String>::new();
    let mut entries = 0_usize;
    let mut bytes = 0_usize;
    for name in names {
        let Some(storage_name) = name.to_str() else {
            return false;
        };
        let (logical_name, retired) = if GenerationId::parse(storage_name).is_ok() {
            (storage_name, false)
        } else {
            let Some(logical_name) = storage_name.strip_prefix(".lease-retire-") else {
                return false;
            };
            if GenerationId::parse(logical_name).is_err() {
                return false;
            }
            (logical_name, true)
        };
        if logical_names
            .iter()
            .any(|existing| existing == logical_name)
        {
            return false;
        }
        logical_names.push(logical_name.to_owned());
        let Ok(stat) = statat(&capability.leases, storage_name, AtFlags::SYMLINK_NOFOLLOW) else {
            return false;
        };
        if account_inventory(&mut entries, &mut bytes, stat.st_size).is_err() {
            return false;
        }
        let ReconciliationLease::Valid(validated) =
            read_reconciliation_lease_at(capability, storage_name, retired)
        else {
            return false;
        };
        let references: Vec<_> = records
            .iter()
            .filter(|record| record.record.lease == logical_name)
            .collect();
        if retired
            && (references.len() != 1
                || references[0].retired
                || references[0].record.state != LaunchState::Terminal)
        {
            return false;
        }
        if let ParsedLeaseRecord::Lifecycle(lifecycle) = &validated.record {
            if references.len() != 1 || !lifecycle_lease_matches(&references[0].record, lifecycle) {
                return false;
            }
        }
    }
    inventory_within_bounds(entries, bytes).is_ok()
        && records.iter().all(|record| {
            !record.retired
                || (record.record.state == LaunchState::Terminal
                    && !logical_names
                        .iter()
                        .any(|name| name == &record.record.lease))
        })
}

fn validate_selection_store_authority(
    capability: &GenerationLeaseCapability,
    selection: &GenerationSelection,
) -> Result<(), String> {
    capability.revalidate()?;
    let leases =
        rustix::fs::fstat(&selection.lease_directory).map_err(|error| error.to_string())?;
    let lock = rustix::fs::fstat(&selection.activation_lock).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "selection leases directory",
        &leases,
        FileType::Directory,
        0o700,
        rustix::process::getuid().as_raw(),
    )?;
    validate_owned_mode(
        "selection activation lock",
        &lock,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    let leases_identity = (leases.st_dev, leases.st_ino);
    let lock_identity = (lock.st_dev, lock.st_ino);
    if leases_identity != selection.lease_directory_identity
        || lock_identity != selection.activation_lock_identity
        || leases_identity != capability.leases_identity
        || lock_identity != capability.lock_identity
    {
        return Err("generation selection belongs to another store authority".into());
    }
    Ok(())
}

impl ActivationRegistry {
    fn open(
        state_home: &Path,
        generation_leases: GenerationLeaseCapability,
    ) -> Result<Self, String> {
        if state_home.as_os_str().is_empty() || !state_home.is_absolute() {
            return Err("state home must be a non-empty absolute path".into());
        }
        generation_leases.revalidate()?;
        let state = open_directory_chain(state_home)?;
        let state_stat = rustix::fs::fstat(&state).map_err(|error| error.to_string())?;
        if FileType::from_raw_mode(state_stat.st_mode) != FileType::Directory
            || state_stat.st_uid != rustix::process::getuid().as_raw()
        {
            return Err("state home is not an owned directory".into());
        }

        let helm = ensure_owned_directory(&state, "helm")?;
        let activation_fd = ensure_owned_directory(&helm, "activation")?;
        let activation = GenerationRoot { fd: activation_fd };
        let lock = open_lifecycle_lock(&activation.fd)?;
        flock(&lock, FlockOperation::LockExclusive).map_err(|error| error.to_string())?;

        let initialization = (|| {
            revalidate_opened_path(&activation.fd, "lifecycle.lock", &lock)?;
            fsync(&lock).map_err(|error| error.to_string())?;
            fsync(&activation.fd).map_err(|error| error.to_string())?;
            let launches = GenerationRoot {
                fd: ensure_owned_directory(&activation.fd, "launches")?,
            };
            let activation_stat =
                rustix::fs::fstat(&activation.fd).map_err(|error| error.to_string())?;
            let registry = Self {
                activation,
                launches,
                generation_leases,
                lock,
                intra_process_lock: Mutex::new(()),
                device: activation_stat.st_dev,
                inode: activation_stat.st_ino,
            };
            registry.inspect_and_recover_inventory()?;
            Ok(registry)
        })();

        match initialization {
            Ok(registry) => {
                flock(&registry.lock, FlockOperation::Unlock).map_err(|error| error.to_string())?;
                Ok(registry)
            }
            Err(error) => Err(error),
        }
    }

    fn lock(&self) -> Result<RegistryLock<'_>, String> {
        let intra_process = self
            .intra_process_lock
            .lock()
            .map_err(|_| "lifecycle registry lock is poisoned")?;
        flock(&self.lock, FlockOperation::LockExclusive).map_err(|error| error.to_string())?;
        if let Err(error) = validate_locked_lifecycle_path(&self.activation.fd, &self.lock) {
            let _ = flock(&self.lock, FlockOperation::Unlock);
            return Err(error);
        }
        Ok(RegistryLock {
            lock: &self.lock,
            _intra_process: intra_process,
        })
    }

    fn claim_session(&self, request: SessionClaimRequest) -> Result<SessionClaim, String> {
        let _lock = self.lock()?;
        self.inspect_and_recover_inventory()?;
        match statat(&self.activation.fd, "session", AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => return Err("a session claim already exists".into()),
            Err(Errno::NOENT) => {}
            Err(error) => return Err(error.to_string()),
        }

        let (start_time, owner_uid) = linux_process_identity(request.entry_pid)?;
        let current_uid = rustix::process::getuid().as_raw();
        if owner_uid != current_uid {
            return Err("session entry is not owned by the current user".into());
        }
        let boot_id = linux_boot_id()?;
        let record = SessionRecord::claimed(
            request.session,
            request.entry_pid,
            start_time,
            owner_uid,
            boot_id.clone(),
        );
        let temporary = format!(".session-claim-{}", request.session.encode());
        write_exclusive_synced(&self.activation.fd, &temporary, &record.encode())?;

        let publication = (|| {
            let (final_start_time, final_uid) = linux_process_identity(request.entry_pid)?;
            let final_boot_id = linux_boot_id()?;
            if final_start_time != start_time
                || final_uid != owner_uid
                || final_uid != current_uid
                || final_boot_id != boot_id
            {
                return Err("session entry identity changed before publication".into());
            }
            renameat_with(
                &self.activation.fd,
                temporary.as_str(),
                &self.activation.fd,
                "session",
                RenameFlags::NOREPLACE,
            )
            .map_err(|error| {
                if error == Errno::EXIST {
                    "a session claim already exists".to_owned()
                } else {
                    error.to_string()
                }
            })?;
            fsync(&self.activation.fd).map_err(|error| error.to_string())
        })();
        if let Err(error) = publication {
            let _ = unlinkat(&self.activation.fd, temporary.as_str(), AtFlags::empty());
            let _ = fsync(&self.activation.fd);
            return Err(error);
        }
        Ok(SessionClaim { record })
    }

    fn open_active_session(
        &self,
        session: SessionId,
        expected_sequence: u64,
    ) -> Result<ActiveSessionCapability, String> {
        let _lock = self.lock()?;
        self.inspect_and_recover_inventory()?;
        let record = read_session_record(&self.activation.fd, "session")?;
        if record.session != session
            || record.sequence != expected_sequence
            || record.state != SessionState::Active
        {
            return Err("active session identity or sequence does not match".into());
        }
        revalidate_live_session(&record)?;
        Ok(ActiveSessionCapability {
            record,
            registry_device: self.device,
            registry_inode: self.inode,
        })
    }

    fn prepare(
        &self,
        session: &ActiveSessionCapability,
        request: PrepareLaunch,
        selection: &GenerationSelection,
    ) -> Result<PreparedLaunch, String> {
        let _lock = self.lock()?;
        let _generation_lock = self.generation_leases.lock_shared()?;
        validate_selection_store_authority(&self.generation_leases, selection)?;
        let final_name = request.launch.encode();
        let retirement = format!(".launch-retire-{final_name}");
        for reserved in [final_name.as_str(), retirement.as_str()] {
            match statat(&self.launches.fd, reserved, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(_) => return Err("launch id already exists or is retired".into()),
                Err(Errno::NOENT) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        self.inspect_and_recover_inventory()?;
        if session.registry_device != self.device || session.registry_inode != self.inode {
            return Err("active session capability belongs to another registry".into());
        }
        let current_session = read_session_record(&self.activation.fd, "session")?;
        if current_session != session.record || current_session.state != SessionState::Active {
            return Err("active session capability is stale".into());
        }
        revalidate_live_session(&current_session)?;

        let identity = &selection.process_identity;
        if identity.owner_uid != rustix::process::getuid().as_raw()
            || identity.boot_id != linux_boot_id()?
        {
            return Err("generation selection owner identity is stale".into());
        }
        let (owner_start_time, owner_uid) = linux_process_identity(identity.pid)?;
        if owner_uid != identity.owner_uid || owner_start_time != identity.start_time {
            return Err("generation selection process identity is stale".into());
        }
        if !lower_hex_32(&selection.lease_name) {
            return Err("generation selection lease name is malformed".into());
        }
        match selection.read_current_lease()? {
            Some(ParsedLeaseRecord::Process(record)) if record == *identity => {}
            _ => return Err("generation selection lacks its exact process lease".into()),
        }
        let mut launch_entries = 0_usize;
        let mut launch_bytes = 0_usize;
        for name in bounded_directory_entries(
            &self.launches.fd,
            &mut launch_entries,
            &mut launch_bytes,
            |_| true,
        )? {
            let existing = read_launch_record(&self.launches.fd, &name)?;
            if existing.lease == selection.lease_name {
                return Err("generation selection lease is already referenced".into());
            }
        }
        let (unit, process_group) = match request.mode {
            OwnershipMode::Direct => ("none".to_owned(), identity.pid),
            OwnershipMode::Systemd => (format!("helm-launch-{}.scope", request.launch.encode()), 0),
        };
        let record = LaunchRecord {
            launch: request.launch,
            session: current_session.session,
            sequence: 1,
            state: LaunchState::Preparing,
            result: LaunchResult::None,
            owner_uid: identity.owner_uid,
            boot_id: identity.boot_id.clone(),
            generation: selection.generation.clone(),
            manifest_sha256: selection.manifest_seal_digest.clone(),
            lease: selection.lease_name.clone(),
            lease_kind: LeaseKind::Process,
            owner_pid: identity.pid,
            owner_start_time: identity.start_time,
            mode: request.mode,
            unit,
            unit_invocation: "none".into(),
            process_group,
            cgroup: "none".into(),
            cgroup_device: 0,
            cgroup_inode: 0,
            exec_open: false,
            direct_drained: false,
        };
        LaunchRecord::parse(&record.encode())?;
        self.ensure_capacity_for_new_record(record.encode().len())?;

        let nonce = random_id()?;
        let temporary = format!(
            ".launch-create-{}-{}",
            request.launch.encode(),
            encode_id(nonce),
        );
        write_exclusive_synced(&self.launches.fd, &temporary, &record.encode())?;
        let publication = renameat_with(
            &self.launches.fd,
            temporary.as_str(),
            &self.launches.fd,
            final_name.as_str(),
            RenameFlags::NOREPLACE,
        );
        if let Err(error) = publication {
            let _ = unlinkat(&self.launches.fd, temporary.as_str(), AtFlags::empty());
            let _ = fsync(&self.launches.fd);
            return if error == Errno::EXIST {
                Err("launch id already exists".into())
            } else {
                Err(error.to_string())
            };
        }
        fsync(&self.launches.fd).map_err(|error| error.to_string())?;
        Ok(PreparedLaunch { record })
    }

    fn adopt_prepared(
        &self,
        session: &ActiveSessionCapability,
        prepared: PreparedLaunch,
        selection: GenerationSelection,
        evidence: VerifiedOwnership,
    ) -> Result<AdoptedLaunch, String> {
        let mut filesystem = RealLeaseFilesystem;
        self.adopt_prepared_with_filesystem(session, prepared, selection, evidence, &mut filesystem)
    }

    fn adopt_prepared_with_filesystem<F: LeaseFilesystem>(
        &self,
        session: &ActiveSessionCapability,
        prepared: PreparedLaunch,
        mut selection: GenerationSelection,
        evidence: VerifiedOwnership,
        filesystem: &mut F,
    ) -> Result<AdoptedLaunch, String> {
        let _registry_lock = self.lock()?;
        let _generation_lock = self.generation_leases.lock_shared()?;
        if let Err(error) = validate_selection_store_authority(&self.generation_leases, &selection)
        {
            selection.retain_after_authority_rejection();
            return Err(error);
        }
        match selection.transfer_staging_present() {
            Ok(false) => {}
            Ok(true) => {
                selection.retain_for_registry_reconciliation();
                return Err(
                    "generation lease transfer staging requires registry reconciliation".into(),
                );
            }
            Err(error) => {
                selection.retain_for_registry_reconciliation();
                return Err(error);
            }
        }
        self.inspect_and_recover_inventory()?;
        if session.registry_device != self.device || session.registry_inode != self.inode {
            return Err("active session capability belongs to another registry".into());
        }
        let current_session = read_session_record(&self.activation.fd, "session")?;
        if current_session != session.record || current_session.state != SessionState::Active {
            return Err("active session capability is stale".into());
        }
        revalidate_live_session(&current_session)?;

        let final_name = prepared.record.launch.encode();
        let current_launch = read_launch_record(&self.launches.fd, final_name.as_str())?;
        if current_launch != prepared.record
            || current_launch.state != LaunchState::Preparing
            || current_launch.result != LaunchResult::None
            || current_launch.lease_kind != LeaseKind::Process
            || current_launch.sequence == u64::MAX
            || current_launch.session != current_session.session
        {
            return Err("prepared launch identity or sequence is stale".into());
        }
        validate_selection_matches_launch(&selection, &current_launch)?;
        let (lifecycle_lease, adopted_record) =
            transfer_records(&current_launch, &selection, evidence)?;

        selection.revalidate_for_lifecycle_transfer()?;
        validate_selection_matches_launch(&selection, &current_launch)?;
        if let Err(error) =
            selection.replace_with_lifecycle_locked_with_filesystem(lifecycle_lease, filesystem)
        {
            if selection.transfer_staging_present().unwrap_or(true) {
                selection.retain_for_registry_reconciliation();
            }
            return Err(error);
        }

        filesystem.checkpoint(TransferCheckpoint::BeforeAdoptedRecord)?;
        self.replace_launch_record(&current_launch, &adopted_record)?;
        filesystem.checkpoint(TransferCheckpoint::BeforeSelectionDisarm)?;
        selection.disarm_after_transfer();
        Ok(AdoptedLaunch {
            record: adopted_record,
        })
    }

    fn replace_launch_record(
        &self,
        expected: &LaunchRecord,
        replacement: &LaunchRecord,
    ) -> Result<(), String> {
        if replacement.launch != expected.launch
            || replacement.sequence != expected.sequence + 1
            || LaunchRecord::parse(&replacement.encode())? != *replacement
        {
            return Err("launch replacement is not the exact canonical successor".into());
        }
        let final_name = expected.launch.encode();
        if read_launch_record(&self.launches.fd, final_name.as_str())? != *expected {
            return Err("launch record changed before replacement".into());
        }
        let temporary = format!(
            ".launch-update-{}-{}-{}",
            final_name,
            replacement.sequence,
            encode_id(random_id()?),
        );
        write_exclusive_synced(&self.launches.fd, &temporary, &replacement.encode())?;
        let replaced = (|| {
            if read_launch_record(&self.launches.fd, final_name.as_str())? != *expected {
                return Err("launch record changed before replacement".into());
            }
            renameat(
                &self.launches.fd,
                temporary.as_str(),
                &self.launches.fd,
                final_name.as_str(),
            )
            .map_err(|error| error.to_string())?;
            fsync(&self.launches.fd).map_err(|error| error.to_string())
        })();
        if replaced.is_err()
            && statat(
                &self.launches.fd,
                temporary.as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .is_ok()
        {
            let _ = unlinkat(&self.launches.fd, temporary.as_str(), AtFlags::empty());
            let _ = fsync(&self.launches.fd);
        }
        replaced
    }

    fn ensure_capacity_for_new_record(&self, record_bytes: usize) -> Result<(), String> {
        let mut entries = 0_usize;
        let mut bytes = 0_usize;
        bounded_directory_entries(&self.activation.fd, &mut entries, &mut bytes, |raw| {
            !matches!(raw, b"lifecycle.lock" | b"launches")
        })?;
        bounded_directory_entries(&self.launches.fd, &mut entries, &mut bytes, |_| true)?;
        let entries = entries
            .checked_add(1)
            .ok_or("lifecycle inventory entry count overflow")?;
        let bytes = bytes
            .checked_add(record_bytes)
            .ok_or("lifecycle inventory byte count overflow")?;
        inventory_within_bounds(entries, bytes)
    }

    fn reconcile<I: OwnershipInspector>(
        &self,
        inspector: &I,
        controller: Option<&dyn GateClosedController>,
    ) -> Result<ReconciliationReport, String> {
        let mut filesystem = RealReconciliationFilesystem;
        self.reconcile_with_filesystem(inspector, controller, &mut filesystem)
    }

    fn reconcile_with_filesystem<I: OwnershipInspector, F: ReconciliationFilesystem>(
        &self,
        inspector: &I,
        controller: Option<&dyn GateClosedController>,
        filesystem: &mut F,
    ) -> Result<ReconciliationReport, String> {
        self.reconcile_with_filesystem_and_classification_checkpoint(
            inspector,
            controller,
            filesystem,
            || {},
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn reconcile_with_filesystem_and_classification_checkpoint<
        I: OwnershipInspector,
        F: ReconciliationFilesystem,
        H: FnMut(),
    >(
        &self,
        inspector: &I,
        controller: Option<&dyn GateClosedController>,
        filesystem: &mut F,
        mut after_registry_classification: H,
    ) -> Result<ReconciliationReport, String> {
        let _registry_lock = self.lock()?;
        let _generation_lock = self.generation_leases.lock_shared()?;
        let inventory_recovery = self.classify_inventory_recovery()?;
        after_registry_classification();
        let mut records = Vec::new();
        let mut launch_entries = 0_usize;
        let mut launch_bytes = 0_usize;
        for name in bounded_directory_entries(
            &self.launches.fd,
            &mut launch_entries,
            &mut launch_bytes,
            |_| true,
        )? {
            if parse_temporary_name(name.as_bytes()).is_some() {
                continue;
            }
            let name = name
                .to_str()
                .ok_or("launch filename is not UTF-8 after inventory validation")?;
            records.push(read_reconciliation_launch(&self.launches.fd, name)?);
        }
        let mut report = ReconciliationReport::default();
        let mut lease_entries = 0_usize;
        let mut lease_bytes = 0_usize;
        let lease_names = bounded_directory_entries(
            &self.generation_leases.leases,
            &mut lease_entries,
            &mut lease_bytes,
            |_| true,
        )?;
        let has_retirement = lease_names.iter().any(|name| {
            name.to_str()
                .is_some_and(|name| name.starts_with(".lease-retire-"))
        });
        let has_transfer = lease_names.iter().any(|name| {
            name.to_str()
                .is_some_and(|name| name.starts_with(".lease-transfer-"))
        });
        if has_retirement && has_transfer {
            return Err("lease retirement cannot coexist with transfer staging".into());
        }
        let transfer_plan = if has_transfer {
            classify_lease_transfer_staging_locked(&self.generation_leases.leases)
                .map_err(|error| format!("transfer staging recovery failed: {error}"))?
        } else {
            LeaseTransferRecoveryPlan::default()
        };
        let normalized_lease_names: Vec<_> = lease_names
            .iter()
            .filter(|name| {
                !name
                    .to_str()
                    .is_some_and(|name| name.starts_with(".lease-transfer-"))
            })
            .cloned()
            .collect();
        if !reconciliation_lease_inventory_is_safe(
            &self.generation_leases,
            &records,
            &normalized_lease_names,
        ) {
            report.retained = records.len();
            return Ok(report);
        }
        let evidence: Vec<_> = records
            .into_iter()
            .map(|launch| {
                let lease =
                    read_reconciliation_lease(&self.generation_leases, &launch.record.lease);
                (launch, lease)
            })
            .collect();
        if evidence
            .iter()
            .any(|(launch, lease)| !reconciliation_lease_relation_is_known(&launch.record, lease))
        {
            report.retained = evidence.len();
            return Ok(report);
        }
        let inventory_len = evidence.len();
        let mut plans = Vec::with_capacity(inventory_len);
        let mut passive_uncertain = false;
        for (launch, lease) in evidence {
            match self.plan_reconciliation(launch, lease, inspector) {
                Some(plan) => plans.push(plan),
                None => passive_uncertain = true,
            }
        }
        if passive_uncertain {
            report.retained = inventory_len;
            return Ok(report);
        }
        if plans.iter().any(|plan| plan.controller.is_some()) && controller.is_none() {
            report.retained = inventory_len;
            return Ok(report);
        }
        let mut controller_uncertain = false;
        for plan in &plans {
            let Some(action) = &plan.controller else {
                continue;
            };
            let controller = controller.expect("controller availability was checked");
            let empty = match action {
                ReconciliationControllerAction::UnadoptedDirect => {
                    direct_empty_after_abort(controller.abort_unadopted_direct(&plan.launch.record))
                }
                ReconciliationControllerAction::UnadoptedSystemd => {
                    systemd_empty(controller.abort_unadopted_systemd(&plan.launch.record))
                }
                ReconciliationControllerAction::AdoptedSystemd(ownership) => {
                    systemd_empty(controller.abort_adopted_systemd(ownership))
                }
            };
            controller_uncertain |= !empty;
        }
        if controller_uncertain {
            report.retained = inventory_len;
            return Ok(report);
        }
        self.normalize_inventory_recovery(inventory_recovery, || {})?;
        transfer_plan
            .normalize(&self.generation_leases.leases)
            .map_err(|error| format!("transfer staging recovery failed: {error}"))?;
        for plan in plans {
            self.apply_reconciliation_plan(plan, filesystem, &mut report)?;
        }
        Ok(report)
    }

    fn plan_reconciliation<I: OwnershipInspector>(
        &self,
        launch: ValidatedReconciliationLaunch,
        lease: ReconciliationLease,
        inspector: &I,
    ) -> Option<PlannedReconciliation> {
        let record = launch.record.clone();
        let (controller, durable) = match (&record.state, &lease) {
            (LaunchState::Preparing, actual)
                if record.lease_kind == LeaseKind::Process
                    && actual
                        .process()
                        .is_some_and(|process| process_lease_matches(&record, process)) =>
            {
                let controller = match record.mode {
                    OwnershipMode::Direct => ReconciliationControllerAction::UnadoptedDirect,
                    OwnershipMode::Systemd => ReconciliationControllerAction::UnadoptedSystemd,
                };
                (
                    Some(controller),
                    ReconciliationDurableAction::Complete {
                        terminal_result: Some(LaunchResult::Failed),
                        transferred: None,
                        release: true,
                    },
                )
            }
            (LaunchState::Preparing, actual)
                if record.lease_kind == LeaseKind::Process
                    && record.mode == OwnershipMode::Systemd
                    && actual
                        .lifecycle()
                        .is_some_and(|lifecycle| lifecycle_lease_matches(&record, lifecycle)) =>
            {
                let lifecycle = actual.lifecycle().expect("guard validated lifecycle lease");
                (
                    Some(ReconciliationControllerAction::AdoptedSystemd(Box::new(
                        VerifiedSystemd {
                            record: record.clone(),
                            lease: Some(lifecycle.clone()),
                        },
                    ))),
                    ReconciliationDurableAction::Complete {
                        terminal_result: Some(LaunchResult::Failed),
                        transferred: Some(lifecycle.clone()),
                        release: true,
                    },
                )
            }
            (LaunchState::Preparing, ReconciliationLease::Missing)
                if record.lease_kind == LeaseKind::Process =>
            {
                let controller = match record.mode {
                    OwnershipMode::Direct => ReconciliationControllerAction::UnadoptedDirect,
                    OwnershipMode::Systemd => ReconciliationControllerAction::UnadoptedSystemd,
                };
                (
                    Some(controller),
                    ReconciliationDurableAction::Complete {
                        terminal_result: Some(LaunchResult::Failed),
                        transferred: None,
                        release: false,
                    },
                )
            }
            (LaunchState::Adopted, actual)
                if record.lease_kind == LeaseKind::Lifecycle
                    && record.mode == OwnershipMode::Systemd
                    && actual
                        .lifecycle()
                        .is_some_and(|lifecycle| lifecycle_lease_matches(&record, lifecycle)) =>
            {
                let lifecycle = actual.lifecycle().expect("guard validated lifecycle lease");
                (
                    Some(ReconciliationControllerAction::AdoptedSystemd(Box::new(
                        VerifiedSystemd {
                            record: record.clone(),
                            lease: Some(lifecycle.clone()),
                        },
                    ))),
                    ReconciliationDurableAction::Complete {
                        terminal_result: Some(LaunchResult::Failed),
                        transferred: None,
                        release: true,
                    },
                )
            }
            (LaunchState::Running, actual)
                if record.lease_kind == LeaseKind::Lifecycle
                    && actual
                        .lifecycle()
                        .is_some_and(|lifecycle| lifecycle_lease_matches(&record, lifecycle)) =>
            {
                let lifecycle = actual.lifecycle().expect("guard validated lifecycle lease");
                match record.mode {
                    OwnershipMode::Systemd => match inspector.inspect_systemd(&VerifiedSystemd {
                        record: record.clone(),
                        lease: Some(lifecycle.clone()),
                    }) {
                        SystemdObservation::ExactRecursiveEmpty => (
                            None,
                            ReconciliationDurableAction::Complete {
                                terminal_result: Some(LaunchResult::Exited),
                                transferred: None,
                                release: true,
                            },
                        ),
                        SystemdObservation::ExactLive => {
                            (None, ReconciliationDurableAction::Retain)
                        }
                        SystemdObservation::Uncertain => return None,
                    },
                    OwnershipMode::Direct => match inspector.inspect_direct(&record) {
                        DirectObservation::Detached => (
                            None,
                            ReconciliationDurableAction::TerminalizeAndRetain(LaunchResult::Lost),
                        ),
                        DirectObservation::Live => (None, ReconciliationDurableAction::Retain),
                        DirectObservation::Exact { .. } | DirectObservation::Uncertain => {
                            return None;
                        }
                    },
                }
            }
            (LaunchState::Terminal, actual)
                if record.lease_kind == LeaseKind::Process
                    && actual
                        .process()
                        .is_some_and(|process| process_lease_matches(&record, process)) =>
            {
                let controller = match record.mode {
                    OwnershipMode::Direct => ReconciliationControllerAction::UnadoptedDirect,
                    OwnershipMode::Systemd => ReconciliationControllerAction::UnadoptedSystemd,
                };
                (
                    Some(controller),
                    ReconciliationDurableAction::Complete {
                        terminal_result: None,
                        transferred: None,
                        release: true,
                    },
                )
            }
            (LaunchState::Terminal, actual)
                if record.lease_kind == LeaseKind::Lifecycle
                    && actual
                        .lifecycle()
                        .is_some_and(|lifecycle| lifecycle_lease_matches(&record, lifecycle)) =>
            {
                let lifecycle = actual.lifecycle().expect("guard validated lifecycle lease");
                let releasable = match record.mode {
                    OwnershipMode::Systemd => match inspector.inspect_systemd(&VerifiedSystemd {
                        record: record.clone(),
                        lease: Some(lifecycle.clone()),
                    }) {
                        SystemdObservation::ExactRecursiveEmpty => true,
                        SystemdObservation::ExactLive => false,
                        SystemdObservation::Uncertain => return None,
                    },
                    OwnershipMode::Direct => match inspector.inspect_direct(&record) {
                        observation @ DirectObservation::Exact { .. } => {
                            if direct_terminal_releasable(&record, observation) {
                                true
                            } else {
                                return None;
                            }
                        }
                        DirectObservation::Live => false,
                        DirectObservation::Detached | DirectObservation::Uncertain => return None,
                    },
                };
                if releasable {
                    (
                        None,
                        ReconciliationDurableAction::Complete {
                            terminal_result: None,
                            transferred: None,
                            release: true,
                        },
                    )
                } else {
                    (None, ReconciliationDurableAction::Retain)
                }
            }
            (LaunchState::Terminal, ReconciliationLease::Missing)
                if record.mode == OwnershipMode::Direct
                    && record.result == LaunchResult::Failed
                    && record.lease_kind == LeaseKind::Process
                    && !record.exec_open
                    && !record.direct_drained =>
            {
                (
                    Some(ReconciliationControllerAction::UnadoptedDirect),
                    ReconciliationDurableAction::Complete {
                        terminal_result: None,
                        transferred: None,
                        release: false,
                    },
                )
            }
            (LaunchState::Terminal, ReconciliationLease::Missing) => {
                let collectible = match record.mode {
                    OwnershipMode::Systemd => match inspector.inspect_systemd(&VerifiedSystemd {
                        record: record.clone(),
                        lease: None,
                    }) {
                        SystemdObservation::ExactRecursiveEmpty => true,
                        SystemdObservation::ExactLive => false,
                        SystemdObservation::Uncertain => return None,
                    },
                    OwnershipMode::Direct => match inspector.inspect_direct(&record) {
                        observation @ DirectObservation::Exact { .. } => {
                            if direct_terminal_releasable(&record, observation) {
                                true
                            } else {
                                return None;
                            }
                        }
                        DirectObservation::Live => false,
                        DirectObservation::Detached | DirectObservation::Uncertain => return None,
                    },
                };
                if collectible {
                    (
                        None,
                        ReconciliationDurableAction::Complete {
                            terminal_result: None,
                            transferred: None,
                            release: false,
                        },
                    )
                } else {
                    (None, ReconciliationDurableAction::Retain)
                }
            }
            _ => return None,
        };
        Some(PlannedReconciliation {
            launch,
            lease,
            controller,
            durable,
        })
    }

    fn apply_reconciliation_plan<F: ReconciliationFilesystem>(
        &self,
        plan: PlannedReconciliation,
        filesystem: &mut F,
        report: &mut ReconciliationReport,
    ) -> Result<(), String> {
        let PlannedReconciliation {
            launch,
            lease,
            durable,
            ..
        } = plan;
        if matches!(durable, ReconciliationDurableAction::Retain) {
            report.retained += 1;
            return Ok(());
        }
        let record = launch.record.clone();
        let (terminal_result, transferred, release, retain_after_terminal) = match durable {
            ReconciliationDurableAction::TerminalizeAndRetain(result) => {
                (Some(result), None, false, true)
            }
            ReconciliationDurableAction::Complete {
                terminal_result,
                transferred,
                release,
            } => (terminal_result, transferred, release, false),
            ReconciliationDurableAction::Retain => unreachable!(),
        };
        let terminal = terminal_result
            .and_then(|result| self.terminal_successor(&record, result, transferred.as_ref()));
        if terminal_result.is_some() && terminal.is_none() {
            report.retained += 1;
            return Ok(());
        }

        let current = if let Some(successor) = terminal {
            self.replace_launch_record(&record, &successor)?;
            let current =
                read_reconciliation_launch(&self.launches.fd, successor.launch.encode().as_str())?;
            if current.record != successor {
                return Err("terminal launch successor changed after replacement".into());
            }
            filesystem.checkpoint(ReconciliationCheckpoint::AfterTerminalRecordFsync)?;
            report.terminalized += 1;
            current
        } else {
            launch
        };

        if retain_after_terminal {
            report.retained += 1;
            return Ok(());
        }

        if release {
            let ReconciliationLease::Valid(expected) = &lease else {
                return Err("reconciliation release lacks validated lease evidence".into());
            };
            if !self.unlink_exact_lease(&current.record, expected, filesystem)? {
                report.retained += 1;
                return Ok(());
            }
            report.released += 1;
        } else {
            fsync(&self.generation_leases.leases).map_err(|error| error.to_string())?;
            filesystem.checkpoint(ReconciliationCheckpoint::AfterLeaseDirectoryFsync)?;
        }
        filesystem.checkpoint(ReconciliationCheckpoint::BeforeRecordRemoval)?;
        if !self.unlink_exact_launch_record(&current, filesystem)? {
            report.retained += 1;
            return Ok(());
        }
        report.collected += 1;
        Ok(())
    }

    fn terminal_successor(
        &self,
        record: &LaunchRecord,
        result: LaunchResult,
        transferred: Option<&LifecycleLeaseRecord>,
    ) -> Option<LaunchRecord> {
        let mut successor = record.clone();
        successor.sequence = successor.sequence.checked_add(1)?;
        successor.state = LaunchState::Terminal;
        successor.result = result;
        if let Some(lease) = transferred {
            successor.lease_kind = LeaseKind::Lifecycle;
            successor.unit_invocation = lease.unit_invocation.clone();
            successor.cgroup = lease.cgroup.clone();
            successor.cgroup_device = lease.cgroup_device;
            successor.cgroup_inode = lease.cgroup_inode;
        }
        Some(successor)
    }

    fn unlink_exact_lease<F: ReconciliationFilesystem>(
        &self,
        current: &LaunchRecord,
        expected: &ValidatedReconciliationLease,
        filesystem: &mut F,
    ) -> Result<bool, String> {
        self.generation_leases.revalidate()?;
        let ReconciliationLease::Valid(observed) =
            read_reconciliation_lease(&self.generation_leases, &current.lease)
        else {
            return Ok(false);
        };
        let expected_stat =
            rustix::fs::fstat(&expected.descriptor).map_err(|error| error.to_string())?;
        let observed_stat =
            rustix::fs::fstat(&observed.descriptor).map_err(|error| error.to_string())?;
        if observed.record != expected.record
            || observed_stat.st_dev != expected_stat.st_dev
            || observed_stat.st_ino != expected_stat.st_ino
        {
            return Ok(false);
        }
        let retirement = format!(".lease-retire-{}", current.lease);
        if !expected.retired {
            revalidate_opened_path(
                &self.generation_leases.leases,
                expected.storage_name.as_str(),
                &expected.descriptor,
            )?;
            filesystem.checkpoint(ReconciliationCheckpoint::BeforeLeaseRetire)?;
            match renameat_with(
                &self.generation_leases.leases,
                expected.storage_name.as_str(),
                &self.generation_leases.leases,
                retirement.as_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {}
                Err(Errno::EXIST | Errno::NOENT) => return Ok(false),
                Err(error) => return Err(error.to_string()),
            }
            filesystem.checkpoint(ReconciliationCheckpoint::AfterLeaseRetire)?;
        } else if expected.storage_name != retirement {
            return Ok(false);
        }
        let ReconciliationLease::Valid(retired) =
            read_reconciliation_lease(&self.generation_leases, &current.lease)
        else {
            return Ok(false);
        };
        let retired_stat =
            rustix::fs::fstat(&retired.descriptor).map_err(|error| error.to_string())?;
        if !retired.retired
            || retired.record != expected.record
            || retired_stat.st_dev != expected_stat.st_dev
            || retired_stat.st_ino != expected_stat.st_ino
        {
            return Ok(false);
        }
        unlinkat(
            &self.generation_leases.leases,
            retirement.as_str(),
            AtFlags::empty(),
        )
        .map_err(|error| error.to_string())?;
        filesystem.checkpoint(ReconciliationCheckpoint::AfterLeaseUnlink)?;
        fsync(&self.generation_leases.leases).map_err(|error| error.to_string())?;
        filesystem.checkpoint(ReconciliationCheckpoint::AfterLeaseDirectoryFsync)?;
        Ok(true)
    }

    fn unlink_exact_launch_record(
        &self,
        expected: &ValidatedReconciliationLaunch,
        filesystem: &mut impl ReconciliationFilesystem,
    ) -> Result<bool, String> {
        let name = expected.record.launch.encode();
        let observed =
            match read_reconciliation_launch(&self.launches.fd, expected.storage_name.as_str()) {
                Ok(observed) => observed,
                Err(_) => return Ok(false),
            };
        let expected_stat =
            rustix::fs::fstat(&expected.descriptor).map_err(|error| error.to_string())?;
        let observed_stat =
            rustix::fs::fstat(&observed.descriptor).map_err(|error| error.to_string())?;
        if observed.record != expected.record
            || observed_stat.st_dev != expected_stat.st_dev
            || observed_stat.st_ino != expected_stat.st_ino
        {
            return Ok(false);
        }
        let retirement = format!(".launch-retire-{name}");
        if !expected.retired {
            revalidate_opened_path(
                &self.launches.fd,
                expected.storage_name.as_str(),
                &expected.descriptor,
            )?;
            filesystem.checkpoint(ReconciliationCheckpoint::BeforeLaunchRetire)?;
            match renameat_with(
                &self.launches.fd,
                expected.storage_name.as_str(),
                &self.launches.fd,
                retirement.as_str(),
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {}
                Err(Errno::EXIST | Errno::NOENT) => return Ok(false),
                Err(error) => return Err(error.to_string()),
            }
            filesystem.checkpoint(ReconciliationCheckpoint::AfterLaunchRetire)?;
        } else if expected.storage_name != retirement {
            return Ok(false);
        }
        let retired = match read_reconciliation_launch(&self.launches.fd, retirement.as_str()) {
            Ok(retired) => retired,
            Err(_) => return Ok(false),
        };
        let retired_stat =
            rustix::fs::fstat(&retired.descriptor).map_err(|error| error.to_string())?;
        if !retired.retired
            || retired.record != expected.record
            || retired_stat.st_dev != expected_stat.st_dev
            || retired_stat.st_ino != expected_stat.st_ino
        {
            return Ok(false);
        }
        unlinkat(&self.launches.fd, retirement.as_str(), AtFlags::empty())
            .map_err(|error| error.to_string())?;
        fsync(&self.launches.fd).map_err(|error| error.to_string())?;
        Ok(true)
    }

    fn inspect_and_recover_inventory(&self) -> Result<(), String> {
        self.inspect_and_recover_inventory_with_checkpoint(|| {})
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn inspect_and_recover_inventory_with_checkpoint<H>(
        &self,
        mut before_delete: H,
    ) -> Result<(), String>
    where
        H: FnMut(),
    {
        let plan = self.classify_inventory_recovery()?;
        self.normalize_inventory_recovery(plan, &mut before_delete)
    }

    fn classify_inventory_recovery(&self) -> Result<LifecycleInventoryRecoveryPlan, String> {
        let mut entries = 0_usize;
        let mut bytes = 0_usize;
        let activation_entries =
            bounded_directory_entries(&self.activation.fd, &mut entries, &mut bytes, |raw| {
                !matches!(raw, b"lifecycle.lock" | b"launches")
            })?;
        let launch_entries =
            bounded_directory_entries(&self.launches.fd, &mut entries, &mut bytes, |_| true)?;
        let mut session_record = None;
        let mut launch_records = Vec::<LaunchRecord>::new();
        let mut temporaries = Vec::new();

        for name in activation_entries {
            let raw = name.as_bytes();
            if matches!(raw, b"lifecycle.lock" | b"launches") {
                continue;
            }
            let stat = statat(&self.activation.fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            if raw == b"session" {
                session_record = Some(read_session_record(&self.activation.fd, &name)?);
            } else {
                let temporary = parse_temporary_name(raw)
                    .ok_or("activation root contains an unknown or malformed reserved entry")?;
                if !matches!(
                    temporary,
                    TemporaryName::SessionClaim(_) | TemporaryName::SessionUpdate(_, _, _)
                ) {
                    return Err("activation root contains a wrong-namespace reserved entry".into());
                }
                let descriptor = validate_temporary(&self.activation.fd, &name, &stat)?;
                temporaries.push(ValidatedTemporary {
                    directory: InventoryDirectory::Activation,
                    name,
                    kind: temporary,
                    descriptor,
                });
            }
        }

        for name in launch_entries {
            let stat = statat(&self.launches.fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            let raw = name.as_bytes();
            if let Ok(text) = std::str::from_utf8(raw) {
                let parsed = LaunchId::parse(text).map(|id| (id, false)).or_else(|_| {
                    text.strip_prefix(".launch-retire-")
                        .ok_or_else(|| "not a launch retirement".to_owned())
                        .and_then(LaunchId::parse)
                        .map(|id| (id, true))
                });
                if let Ok((id, retired)) = parsed {
                    let record = read_launch_record(&self.launches.fd, &name)?;
                    if record.launch != id {
                        return Err("launch filename and record id disagree".into());
                    }
                    if retired && record.state != LaunchState::Terminal {
                        return Err("launch retirement is not terminal".into());
                    }
                    if launch_records
                        .iter()
                        .any(|existing| existing.launch == record.launch)
                    {
                        return Err("launch inventory contains canonical and retired forms".into());
                    }
                    launch_records.push(record);
                    continue;
                }
            }
            let temporary = parse_temporary_name(raw)
                .ok_or("launch inventory contains an unknown or malformed reserved entry")?;
            if !matches!(
                temporary,
                TemporaryName::LaunchCreate(_, _) | TemporaryName::LaunchUpdate(_, _, _)
            ) {
                return Err("launch inventory contains a wrong-namespace reserved entry".into());
            }
            let descriptor = validate_temporary(&self.launches.fd, &name, &stat)?;
            temporaries.push(ValidatedTemporary {
                directory: InventoryDirectory::Launches,
                name,
                kind: temporary,
                descriptor,
            });
        }
        for temporary in &temporaries {
            match &temporary.kind {
                TemporaryName::SessionClaim(_) | TemporaryName::LaunchCreate(_, _) => {}
                TemporaryName::SessionUpdate(id, _, _) => {
                    if session_record.as_ref().map(|record| record.session) != Some(*id) {
                        return Err(
                            "session update temporary lacks its canonical final record".into()
                        );
                    }
                }
                TemporaryName::LaunchUpdate(id, _, _) => {
                    if !launch_records.iter().any(|record| record.launch == *id) {
                        return Err(
                            "launch update temporary lacks its canonical final record".into()
                        );
                    }
                }
            }
        }

        Ok(LifecycleInventoryRecoveryPlan { temporaries })
    }

    fn normalize_inventory_recovery<H>(
        &self,
        plan: LifecycleInventoryRecoveryPlan,
        mut before_delete: H,
    ) -> Result<(), String>
    where
        H: FnMut(),
    {
        let temporaries = plan.temporaries;
        before_delete();
        for temporary in &temporaries {
            let parent = match temporary.directory {
                InventoryDirectory::Activation => &self.activation.fd,
                InventoryDirectory::Launches => &self.launches.fd,
            };
            revalidate_temporary(parent, &temporary.name, &temporary.descriptor)?;
        }

        let mut activation_changed = false;
        let mut launches_changed = false;
        for temporary in temporaries {
            let parent = match temporary.directory {
                InventoryDirectory::Activation => {
                    activation_changed = true;
                    &self.activation.fd
                }
                InventoryDirectory::Launches => {
                    launches_changed = true;
                    &self.launches.fd
                }
            };
            unlinkat(parent, &temporary.name, AtFlags::empty())
                .map_err(|error| error.to_string())?;
        }
        if activation_changed {
            fsync(&self.activation.fd).map_err(|error| error.to_string())?;
        }
        if launches_changed {
            fsync(&self.launches.fd).map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

impl Drop for RegistryLock<'_> {
    fn drop(&mut self) {
        let _ = flock(self.lock, FlockOperation::Unlock);
    }
}

impl Drop for GenerationLeaseLock<'_> {
    fn drop(&mut self) {
        let _ = flock(self.lock, FlockOperation::Unlock);
    }
}

fn validate_selection_matches_launch(
    selection: &GenerationSelection,
    launch: &LaunchRecord,
) -> Result<(), String> {
    if selection.generation != launch.generation
        || selection.manifest_seal_digest != launch.manifest_sha256
        || selection.lease_name != launch.lease
        || selection.process_identity.generation != launch.generation
        || selection.process_identity.pid != launch.owner_pid
        || selection.process_identity.start_time != launch.owner_start_time
        || selection.process_identity.boot_id != launch.boot_id
        || selection.process_identity.owner_uid != launch.owner_uid
    {
        return Err("generation selection does not match the prepared launch".into());
    }
    Ok(())
}

fn transfer_records(
    preparing: &LaunchRecord,
    selection: &GenerationSelection,
    evidence: VerifiedOwnership,
) -> Result<(LifecycleLeaseRecord, LaunchRecord), String> {
    let (owner_kind, unit, unit_invocation, process_group, cgroup, cgroup_device, cgroup_inode) =
        match (preparing.mode, evidence) {
            (OwnershipMode::Direct, VerifiedOwnership::Direct { binding })
                if binding == OwnershipBinding::from_preparing(preparing)
                    && binding.process_group == preparing.owner_pid =>
            {
                (
                    LifecycleOwnerKind::ProcessGroup,
                    "none".to_owned(),
                    "none".to_owned(),
                    binding.process_group,
                    "none".to_owned(),
                    0,
                    0,
                )
            }
            (
                OwnershipMode::Systemd,
                VerifiedOwnership::Systemd {
                    binding,
                    unit_invocation,
                    cgroup,
                    cgroup_device,
                    cgroup_inode,
                },
            ) if binding == OwnershipBinding::from_preparing(preparing)
                && lower_hex_32(&unit_invocation)
                && normalized_cgroup_path(&cgroup)
                && cgroup_device > 0
                && cgroup_inode > 0 =>
            {
                (
                    LifecycleOwnerKind::SystemdScope,
                    preparing.unit.clone(),
                    unit_invocation,
                    0,
                    cgroup,
                    cgroup_device,
                    cgroup_inode,
                )
            }
            _ => return Err("verified ownership does not match the prepared launch".into()),
        };
    let launch_id = GenerationId::parse(&preparing.launch.encode())?;
    let lease = LifecycleLeaseRecord {
        generation: selection.generation.clone(),
        launch: launch_id,
        pid: selection.process_identity.pid,
        start_time: selection.process_identity.start_time,
        boot_id: selection.process_identity.boot_id.clone(),
        owner_uid: selection.process_identity.owner_uid,
        owner_kind,
        unit: unit.clone(),
        unit_invocation: unit_invocation.clone(),
        process_group,
        cgroup: cgroup.clone(),
        cgroup_device,
        cgroup_inode,
    };
    LifecycleLeaseRecord::parse(&lease.encode())?;

    let mut adopted = preparing.clone();
    adopted.sequence = preparing
        .sequence
        .checked_add(1)
        .ok_or("launch sequence overflow")?;
    adopted.state = LaunchState::Adopted;
    adopted.lease_kind = LeaseKind::Lifecycle;
    adopted.unit = unit;
    adopted.unit_invocation = unit_invocation;
    adopted.process_group = process_group;
    adopted.cgroup = cgroup;
    adopted.cgroup_device = cgroup_device;
    adopted.cgroup_inode = cgroup_inode;
    LaunchRecord::parse(&adopted.encode())?;
    Ok((lease, adopted))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InventoryDirectory {
    Activation,
    Launches,
}

#[derive(Debug)]
struct ValidatedTemporary {
    directory: InventoryDirectory,
    name: OsString,
    kind: TemporaryName,
    descriptor: OwnedFd,
}

#[derive(Debug)]
struct LifecycleInventoryRecoveryPlan {
    temporaries: Vec<ValidatedTemporary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemporaryName {
    SessionClaim(SessionId),
    SessionUpdate(SessionId, u64, [u8; 16]),
    LaunchCreate(LaunchId, [u8; 16]),
    LaunchUpdate(LaunchId, u64, [u8; 16]),
}

fn ensure_owned_directory(parent: &OwnedFd, name: &str) -> Result<OwnedFd, String> {
    match mkdirat(parent, name, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.to_string()),
    }
    let child = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let stat = rustix::fs::fstat(&child).map_err(|error| error.to_string())?;
    validate_owned_mode(
        name,
        &stat,
        FileType::Directory,
        0o700,
        rustix::process::getuid().as_raw(),
    )?;
    fsync(&child).map_err(|error| error.to_string())?;
    fsync(parent).map_err(|error| error.to_string())?;
    Ok(child)
}

fn open_lifecycle_lock(activation: &OwnedFd) -> Result<OwnedFd, String> {
    let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let lock = match openat(
        activation,
        "lifecycle.lock",
        flags | OFlags::CREATE | OFlags::EXCL,
        Mode::RUSR | Mode::WUSR,
    ) {
        Ok(lock) => lock,
        Err(Errno::EXIST) => openat(activation, "lifecycle.lock", flags, Mode::empty())
            .map_err(|error| error.to_string())?,
        Err(error) => return Err(error.to_string()),
    };
    let stat = rustix::fs::fstat(&lock).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "lifecycle.lock",
        &stat,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if stat.st_size != 0 {
        return Err("lifecycle.lock must be empty".into());
    }
    Ok(lock)
}

fn validate_locked_lifecycle_path(activation: &OwnedFd, lock: &OwnedFd) -> Result<(), String> {
    let stat = rustix::fs::fstat(lock).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "lifecycle.lock",
        &stat,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if stat.st_size != 0 {
        return Err("lifecycle.lock must be empty".into());
    }
    revalidate_opened_path(activation, "lifecycle.lock", lock)
}

fn revalidate_opened_path(parent: &OwnedFd, name: &str, opened: &OwnedFd) -> Result<(), String> {
    let path =
        statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
    let descriptor = rustix::fs::fstat(opened).map_err(|error| error.to_string())?;
    if path.st_dev != descriptor.st_dev || path.st_ino != descriptor.st_ino {
        return Err(format!("{name} changed while opening"));
    }
    Ok(())
}

fn bounded_directory_entries<F>(
    directory: &OwnedFd,
    entries: &mut usize,
    bytes: &mut usize,
    mut counts_toward_inventory: F,
) -> Result<Vec<OsString>, String>
where
    F: FnMut(&[u8]) -> bool,
{
    let mut names = Vec::new();
    let mut reader = Dir::read_from(directory).map_err(|error| error.to_string())?;
    while let Some(entry) = reader.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let raw = entry.file_name().to_bytes();
        if matches!(raw, b"." | b"..") {
            continue;
        }
        if counts_toward_inventory(raw) {
            let stat = statat(directory, entry.file_name(), AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            account_inventory(entries, bytes, stat.st_size)?;
            inventory_within_bounds(*entries, *bytes)?;
        }
        names.push(OsString::from_vec(raw.to_vec()));
    }
    canonicalize_inventory_names(&mut names);
    Ok(names)
}

fn canonicalize_inventory_names(names: &mut [OsString]) {
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
}

fn account_inventory(entries: &mut usize, bytes: &mut usize, size: i64) -> Result<(), String> {
    *entries = entries
        .checked_add(1)
        .ok_or("lifecycle inventory entry count overflow")?;
    let size = usize::try_from(size).map_err(|_| "lifecycle inventory has a negative size")?;
    *bytes = bytes
        .checked_add(size)
        .ok_or("lifecycle inventory byte count overflow")?;
    Ok(())
}

fn inventory_within_bounds(entries: usize, bytes: usize) -> Result<(), String> {
    if entries > MAX_INVENTORY_ENTRIES || bytes > MAX_INVENTORY_BYTES {
        return Err("lifecycle inventory exceeds its scan bounds".into());
    }
    Ok(())
}

fn validate_temporary(
    parent: &OwnedFd,
    name: &OsStr,
    stat: &rustix::fs::Stat,
) -> Result<OwnedFd, String> {
    validate_owned_mode(
        "lifecycle temporary",
        stat,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if stat.st_size > MAX_RECORD_BYTES as i64 {
        return Err("lifecycle temporary exceeds 4096 bytes".into());
    }
    let opened = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let opened_stat = rustix::fs::fstat(&opened).map_err(|error| error.to_string())?;
    if opened_stat.st_dev != stat.st_dev
        || opened_stat.st_ino != stat.st_ino
        || opened_stat.st_size > MAX_RECORD_BYTES as i64
    {
        return Err("lifecycle temporary changed during inventory".into());
    }
    Ok(opened)
}

fn revalidate_temporary(parent: &OwnedFd, name: &OsStr, opened: &OwnedFd) -> Result<(), String> {
    let path =
        statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "lifecycle temporary",
        &path,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    let descriptor = rustix::fs::fstat(opened).map_err(|error| error.to_string())?;
    if path.st_dev != descriptor.st_dev
        || path.st_ino != descriptor.st_ino
        || path.st_size != descriptor.st_size
        || path.st_size > MAX_RECORD_BYTES as i64
    {
        return Err("lifecycle temporary changed before deletion".into());
    }
    Ok(())
}

fn parse_temporary_name(raw: &[u8]) -> Option<TemporaryName> {
    let text = std::str::from_utf8(raw).ok()?;
    if let Some(id) = text.strip_prefix(".session-claim-") {
        return SessionId::parse(id).ok().map(TemporaryName::SessionClaim);
    }
    if let Some(rest) = text.strip_prefix(".session-update-") {
        let (id, sequence, nonce) = split_update_name(rest)?;
        return Some(TemporaryName::SessionUpdate(
            SessionId::parse(id).ok()?,
            canonical_positive_u64(sequence, "session update sequence").ok()?,
            parse_id(nonce, "session update nonce").ok()?,
        ));
    }
    if let Some(rest) = text.strip_prefix(".launch-create-") {
        let (id, nonce) = rest.split_once('-')?;
        return Some(TemporaryName::LaunchCreate(
            LaunchId::parse(id).ok()?,
            parse_id(nonce, "launch create nonce").ok()?,
        ));
    }
    if let Some(rest) = text.strip_prefix(".launch-update-") {
        let (id, sequence, nonce) = split_update_name(rest)?;
        return Some(TemporaryName::LaunchUpdate(
            LaunchId::parse(id).ok()?,
            canonical_positive_u64(sequence, "launch update sequence").ok()?,
            parse_id(nonce, "launch update nonce").ok()?,
        ));
    }
    None
}

fn split_update_name(value: &str) -> Option<(&str, &str, &str)> {
    let (id, rest) = value.split_once('-')?;
    let (sequence, nonce) = rest.split_once('-')?;
    if nonce.contains('-') {
        return None;
    }
    Some((id, sequence, nonce))
}

fn read_session_record(parent: &OwnedFd, name: impl AsRef<OsStr>) -> Result<SessionRecord, String> {
    SessionRecord::parse(&read_canonical_file(parent, name.as_ref())?)
}

fn read_launch_record(parent: &OwnedFd, name: impl AsRef<OsStr>) -> Result<LaunchRecord, String> {
    LaunchRecord::parse(&read_canonical_file(parent, name.as_ref())?)
}

fn read_canonical_file(parent: &OwnedFd, name: &OsStr) -> Result<Vec<u8>, String> {
    let before =
        statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(|error| error.to_string())?;
    validate_owned_mode(
        "lifecycle record",
        &before,
        FileType::RegularFile,
        0o600,
        rustix::process::getuid().as_raw(),
    )?;
    if before.st_size > MAX_RECORD_BYTES as i64 {
        return Err("lifecycle record exceeds 4096 bytes".into());
    }
    let fd = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| error.to_string())?;
    let opened = rustix::fs::fstat(&fd).map_err(|error| error.to_string())?;
    if before.st_dev != opened.st_dev || before.st_ino != opened.st_ino {
        return Err("lifecycle record changed during open".into());
    }
    let mut file = std::fs::File::from(fd);
    let mut raw = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_RECORD_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|error| error.to_string())?;
    if raw.len() > MAX_RECORD_BYTES {
        return Err("lifecycle record exceeds 4096 bytes".into());
    }
    Ok(raw)
}

fn write_exclusive_synced(parent: &OwnedFd, name: &str, raw: &[u8]) -> Result<(), String> {
    if raw.len() > MAX_RECORD_BYTES {
        return Err("lifecycle record exceeds 4096 bytes".into());
    }
    let fd = openat(
        parent,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|error| error.to_string())?;
    let mut file = std::fs::File::from(fd);
    file.write_all(raw).map_err(|error| error.to_string())?;
    fsync(file.as_fd()).map_err(|error| error.to_string())
}

fn revalidate_live_session(record: &SessionRecord) -> Result<(), String> {
    if record.owner_uid != rustix::process::getuid().as_raw() || record.boot_id != linux_boot_id()?
    {
        return Err("session owner or boot identity is stale".into());
    }
    let (start_time, owner_uid) = linux_process_identity(record.entry_pid)?;
    if owner_uid != record.owner_uid || start_time != record.entry_start_time {
        return Err("session entry identity is stale".into());
    }
    Ok(())
}

fn parse_id(value: &str, field: &str) -> Result<[u8; 16], String> {
    if !lower_hex_32(value) {
        return Err(format!("{field} must be 128-bit lowercase hex"));
    }
    let mut bytes = [0_u8; 16];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(bytes)
}

fn random_id() -> Result<[u8; 16], String> {
    let mut id = [0_u8; 16];
    getrandom::fill(&mut id).map_err(|error| error.to_string())?;
    Ok(id)
}

fn encode_id(value: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(32);
    for byte in value {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => unreachable!("validated lowercase hexadecimal digit"),
    }
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn parse_yes_no(value: &str, field: &str) -> Result<bool, String> {
    match value {
        "no" => Ok(false),
        "yes" => Ok(true),
        _ => Err(format!("{field} is malformed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::{GenerationGcReport, GenerationPublication, GenerationStore};
    use std::cell::Cell;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::Path;

    fn direct_preparing(process_group: &str) -> Vec<u8> {
        format!(
            "helm-activation-launch-v1\n\
launch 11111111111111111111111111111111\n\
session 22222222222222222222222222222222\n\
sequence 1\n\
state preparing\n\
result none\n\
owner-uid {}\n\
boot-id 12345678-1234-1234-1234-123456789abc\n\
generation 33333333333333333333333333333333\n\
manifest-sha256 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n\
lease 44444444444444444444444444444444\n\
lease-kind process\n\
owner-pid 42\n\
owner-start-time 7\n\
owner-kind process-group\n\
unit none\n\
unit-invocation none\n\
{process_group}\n\
cgroup none\n\
cgroup-device 0\n\
cgroup-inode 0\n\
exec-open no\n\
direct-drained no\n",
            rustix::process::getuid().as_raw(),
        )
        .into_bytes()
    }

    fn systemd_preparing(invocation: &str, unit: &str) -> Vec<u8> {
        let direct = String::from_utf8(direct_preparing("process-group 42")).unwrap();
        direct
            .replace("owner-kind process-group", "owner-kind systemd-scope")
            .replace("unit none", &format!("unit {unit}"))
            .replace(
                "unit-invocation none",
                &format!("unit-invocation {invocation}"),
            )
            .replace("process-group 42", "process-group 0")
            .into_bytes()
    }

    fn exact_session_record(state: &str, sequence: u64) -> Vec<u8> {
        let pid = std::process::id();
        let (start_time, uid) = super::linux_process_identity(pid).unwrap();
        format!(
            "helm-activation-session-v1\n\
session 22222222222222222222222222222222\n\
sequence {sequence}\n\
state {state}\n\
owner-uid {uid}\n\
boot-id {}\n\
entry-pid {pid}\n\
entry-start-time {start_time}\n\
compositor-pid {pid}\n\
compositor-start-time {start_time}\n\
helper-mode systemd\n\
wm-pid 0\n\
wm-start-time 0\n\
wm-process-group 0\n\
bar-pid 0\n\
bar-start-time 0\n\
bar-process-group 0\n",
            super::linux_boot_id().unwrap(),
        )
        .into_bytes()
    }

    fn write_mode(path: &Path, bytes: &[u8], mode: u32) {
        fs::write(path, bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn open_test_registry(state_home: &Path) -> Result<ActivationRegistry, String> {
        let generated = state_home.join(".test-generated");
        if !generated.exists() {
            fs::create_dir(&generated).map_err(|error| error.to_string())?;
            fs::set_permissions(&generated, fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        let store = GenerationStore::open(&generated)?;
        ActivationRegistry::open(state_home, GenerationLeaseCapability::from_store(&store)?)
    }

    struct TestOwnershipVerifier {
        proof: TestOwnershipProof,
    }

    struct TestOwnershipInspector {
        systemd: SystemdObservation,
        direct: DirectObservation,
        systemd_calls: Cell<usize>,
        direct_calls: Cell<usize>,
    }

    impl ownership_inspector::Sealed for TestOwnershipInspector {}

    impl OwnershipInspector for TestOwnershipInspector {
        fn inspect_systemd(&self, _ownership: &VerifiedSystemd) -> SystemdObservation {
            self.systemd_calls.set(self.systemd_calls.get() + 1);
            self.systemd
        }

        fn inspect_direct(&self, _record: &LaunchRecord) -> DirectObservation {
            self.direct_calls.set(self.direct_calls.get() + 1);
            self.direct
        }
    }

    struct TestGateClosedController {
        direct: DirectObservation,
        unadopted_systemd: SystemdObservation,
        adopted_systemd: SystemdObservation,
        direct_calls: Cell<usize>,
        unadopted_systemd_calls: Cell<usize>,
        adopted_systemd_calls: Cell<usize>,
    }

    struct SelectiveSystemdInspector {
        uncertain_launch: LaunchId,
        systemd_calls: Cell<usize>,
    }

    impl ownership_inspector::Sealed for SelectiveSystemdInspector {}

    impl OwnershipInspector for SelectiveSystemdInspector {
        fn inspect_systemd(&self, ownership: &VerifiedSystemd) -> SystemdObservation {
            self.systemd_calls.set(self.systemd_calls.get() + 1);
            if ownership.record.launch == self.uncertain_launch {
                SystemdObservation::Uncertain
            } else {
                SystemdObservation::ExactRecursiveEmpty
            }
        }

        fn inspect_direct(&self, _record: &LaunchRecord) -> DirectObservation {
            DirectObservation::Uncertain
        }
    }

    struct SelectiveDirectController {
        uncertain_launch: LaunchId,
        direct_calls: Cell<usize>,
    }

    impl gate_closed_controller::Sealed for SelectiveDirectController {}

    impl GateClosedController for SelectiveDirectController {
        fn abort_unadopted_direct(&self, record: &LaunchRecord) -> DirectObservation {
            self.direct_calls.set(self.direct_calls.get() + 1);
            if record.launch == self.uncertain_launch {
                DirectObservation::Uncertain
            } else {
                DirectObservation::Exact {
                    owner_written_witness: false,
                    exact_owner_stale: true,
                    recorded_group_empty: true,
                }
            }
        }

        fn abort_unadopted_systemd(&self, _record: &LaunchRecord) -> SystemdObservation {
            SystemdObservation::Uncertain
        }

        fn abort_adopted_systemd(&self, _ownership: &VerifiedSystemd) -> SystemdObservation {
            SystemdObservation::Uncertain
        }
    }

    impl gate_closed_controller::Sealed for TestGateClosedController {}

    impl GateClosedController for TestGateClosedController {
        fn abort_unadopted_direct(&self, _record: &LaunchRecord) -> DirectObservation {
            self.direct_calls.set(self.direct_calls.get() + 1);
            self.direct
        }

        fn abort_unadopted_systemd(&self, _record: &LaunchRecord) -> SystemdObservation {
            self.unadopted_systemd_calls
                .set(self.unadopted_systemd_calls.get() + 1);
            self.unadopted_systemd
        }

        fn abort_adopted_systemd(&self, _ownership: &VerifiedSystemd) -> SystemdObservation {
            self.adopted_systemd_calls
                .set(self.adopted_systemd_calls.get() + 1);
            self.adopted_systemd
        }
    }

    fn exact_empty_inspector() -> TestOwnershipInspector {
        TestOwnershipInspector {
            systemd: SystemdObservation::ExactRecursiveEmpty,
            direct: DirectObservation::Exact {
                owner_written_witness: true,
                exact_owner_stale: true,
                recorded_group_empty: true,
            },
            systemd_calls: Cell::new(0),
            direct_calls: Cell::new(0),
        }
    }

    fn exact_empty_controller() -> TestGateClosedController {
        TestGateClosedController {
            direct: DirectObservation::Exact {
                owner_written_witness: false,
                exact_owner_stale: true,
                recorded_group_empty: true,
            },
            unadopted_systemd: SystemdObservation::ExactRecursiveEmpty,
            adopted_systemd: SystemdObservation::ExactRecursiveEmpty,
            direct_calls: Cell::new(0),
            unadopted_systemd_calls: Cell::new(0),
            adopted_systemd_calls: Cell::new(0),
        }
    }

    enum TestOwnershipProof {
        Direct,
        Systemd {
            unit_invocation: String,
            cgroup: String,
            cgroup_device: u64,
            cgroup_inode: u64,
        },
    }

    impl ownership_verifier::Sealed for TestOwnershipVerifier {}

    impl OwnershipVerifier for TestOwnershipVerifier {
        fn verify(&self, prepared: &PreparedLaunch) -> Result<VerifiedOwnership, String> {
            let binding = OwnershipBinding::from_preparing(&prepared.record);
            Ok(match &self.proof {
                TestOwnershipProof::Direct => VerifiedOwnership::Direct { binding },
                TestOwnershipProof::Systemd {
                    unit_invocation,
                    cgroup,
                    cgroup_device,
                    cgroup_inode,
                } => VerifiedOwnership::Systemd {
                    binding,
                    unit_invocation: unit_invocation.clone(),
                    cgroup: cgroup.clone(),
                    cgroup_device: *cgroup_device,
                    cgroup_inode: *cgroup_inode,
                },
            })
        }
    }

    struct FaultingLeaseFilesystem {
        fail_at: TransferCheckpoint,
        lease_path: std::path::PathBuf,
        launch_path: std::path::PathBuf,
        observations: Vec<(TransferCheckpoint, ParsedLeaseRecord, LaunchRecord)>,
    }

    struct SwappingLeaseFilesystem {
        lease_path: std::path::PathBuf,
        displaced_path: std::path::PathBuf,
    }

    struct ExitAtTransferCheckpointFilesystem {
        checkpoint: TransferCheckpoint,
        code: i32,
    }

    struct FaultingReconciliationFilesystem {
        fail_at: ReconciliationCheckpoint,
    }

    struct SwappingReconciliationFilesystem {
        swap_at: ReconciliationCheckpoint,
        path: std::path::PathBuf,
        displaced: std::path::PathBuf,
    }

    impl ReconciliationFilesystem for FaultingReconciliationFilesystem {
        fn checkpoint(&mut self, checkpoint: ReconciliationCheckpoint) -> Result<(), String> {
            if checkpoint == self.fail_at {
                Err(format!("injected reconciliation fault at {checkpoint:?}"))
            } else {
                Ok(())
            }
        }
    }

    impl ReconciliationFilesystem for SwappingReconciliationFilesystem {
        fn checkpoint(&mut self, checkpoint: ReconciliationCheckpoint) -> Result<(), String> {
            if checkpoint == self.swap_at {
                let bytes = fs::read(&self.path).unwrap();
                fs::rename(&self.path, &self.displaced).unwrap();
                write_mode(&self.path, &bytes, 0o600);
            }
            Ok(())
        }
    }

    impl LeaseFilesystem for ExitAtTransferCheckpointFilesystem {
        fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> Result<(), String> {
            if checkpoint == self.checkpoint {
                std::process::exit(self.code);
            }
            Ok(())
        }
    }

    struct SwapAfterValidationFilesystem {
        lease_path: std::path::PathBuf,
        displaced_path: std::path::PathBuf,
    }

    struct RemoveTargetBeforeExchangeFilesystem {
        lease_path: std::path::PathBuf,
    }

    impl LeaseFilesystem for RemoveTargetBeforeExchangeFilesystem {
        fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> Result<(), String> {
            if checkpoint == TransferCheckpoint::BeforeExchange {
                fs::remove_file(&self.lease_path).unwrap();
            }
            Ok(())
        }
    }

    struct ForcingProcFallbackFilesystem {
        empty_path_attempts: usize,
        proc_attempts: usize,
    }

    impl LeaseFilesystem for ForcingProcFallbackFilesystem {
        fn checkpoint(&mut self, _checkpoint: TransferCheckpoint) -> Result<(), String> {
            Ok(())
        }

        fn link_unnamed_empty_path(
            &mut self,
            _source: &OwnedFd,
            _directory: &OwnedFd,
            _name: &str,
        ) -> Result<(), Errno> {
            self.empty_path_attempts += 1;
            Err(Errno::OPNOTSUPP)
        }

        fn link_unnamed_proc(
            &mut self,
            source: &Path,
            directory: &OwnedFd,
            name: &str,
        ) -> Result<(), Errno> {
            self.proc_attempts += 1;
            rustix::fs::linkat(
                rustix::fs::CWD,
                source,
                directory,
                name,
                AtFlags::SYMLINK_FOLLOW,
            )
        }
    }

    struct MismatchedProcFallbackFilesystem {
        decoy: OwnedFd,
    }

    impl LeaseFilesystem for MismatchedProcFallbackFilesystem {
        fn checkpoint(&mut self, _checkpoint: TransferCheckpoint) -> Result<(), String> {
            Ok(())
        }

        fn link_unnamed_empty_path(
            &mut self,
            _source: &OwnedFd,
            _directory: &OwnedFd,
            _name: &str,
        ) -> Result<(), Errno> {
            Err(Errno::OPNOTSUPP)
        }

        fn proc_fd_source(&mut self, _source: &OwnedFd) -> Result<std::path::PathBuf, String> {
            use std::os::fd::AsRawFd;

            Ok(format!("/proc/self/fd/{}", self.decoy.as_raw_fd()).into())
        }
    }

    impl LeaseFilesystem for SwapAfterValidationFilesystem {
        fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> Result<(), String> {
            if checkpoint == TransferCheckpoint::BeforeExchange {
                let lease_name = self.lease_path.file_name().unwrap().to_str().unwrap();
                let staging = self
                    .lease_path
                    .parent()
                    .unwrap()
                    .join(format!(".lease-transfer-{lease_name}"));
                fs::rename(&staging, &self.displaced_path).unwrap();
                write_mode(&staging, b"replacement evidence", 0o600);
            }
            Ok(())
        }
    }

    impl LeaseFilesystem for SwappingLeaseFilesystem {
        fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> Result<(), String> {
            if checkpoint == TransferCheckpoint::BeforeReplace {
                let lease_name = self.lease_path.file_name().unwrap().to_str().unwrap();
                let staging = self
                    .lease_path
                    .parent()
                    .unwrap()
                    .join(format!(".lease-transfer-{lease_name}"));
                fs::rename(&staging, &self.displaced_path).unwrap();
                write_mode(&staging, b"replacement evidence", 0o600);
                return Err("injected staging inode replacement".into());
            }
            Ok(())
        }
    }

    impl LeaseFilesystem for FaultingLeaseFilesystem {
        fn checkpoint(&mut self, checkpoint: TransferCheckpoint) -> Result<(), String> {
            let lease = ParsedLeaseRecord::parse(&fs::read(&self.lease_path).unwrap()).unwrap();
            let launch = LaunchRecord::parse(&fs::read(&self.launch_path).unwrap()).unwrap();
            self.observations.push((checkpoint, lease, launch));
            if checkpoint == self.fail_at {
                Err(format!("injected transfer fault at {checkpoint:?}"))
            } else {
                Ok(())
            }
        }
    }

    struct TransferFixture {
        _generated: tempfile::TempDir,
        _state: tempfile::TempDir,
        store: GenerationStore,
        registry: ActivationRegistry,
        session: ActiveSessionCapability,
        selection: GenerationSelection,
        prepared: PreparedLaunch,
    }

    impl TransferFixture {
        fn direct() -> Self {
            Self::new("11111111111111111111111111111111", OwnershipMode::Direct)
        }

        fn systemd() -> Self {
            Self::new("11111111111111111111111111111111", OwnershipMode::Systemd)
        }

        fn new(launch_id: &str, mode: OwnershipMode) -> Self {
            Self::new_for_owner(launch_id, mode, std::process::id())
        }

        fn new_for_owner(launch_id: &str, mode: OwnershipMode, owner_pid: u32) -> Self {
            let generated = tempfile::tempdir().unwrap();
            fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let store = GenerationStore::open(generated.path()).unwrap();
            let digest = "a".repeat(64);
            store
                .publish(|| {
                    Ok(GenerationPublication {
                        input_digests: [
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                        ],
                        outputs: vec![("fixture".into(), b"sealed".to_vec())],
                    })
                })
                .unwrap();
            let selection = store.select_current_for_process(owner_pid).unwrap();

            let state = tempfile::tempdir().unwrap();
            drop(
                ActivationRegistry::open(
                    state.path(),
                    GenerationLeaseCapability::from_store(&store).unwrap(),
                )
                .unwrap(),
            );
            write_mode(
                &state.path().join("helm/activation/session"),
                &exact_session_record("active", 7),
                0o600,
            );
            let registry = ActivationRegistry::open(
                state.path(),
                GenerationLeaseCapability::from_store(&store).unwrap(),
            )
            .unwrap();
            let session = registry
                .open_active_session(
                    SessionId::parse("22222222222222222222222222222222").unwrap(),
                    7,
                )
                .unwrap();
            let prepared = registry
                .prepare(
                    &session,
                    PrepareLaunch {
                        launch: LaunchId::parse(launch_id).unwrap(),
                        mode,
                    },
                    &selection,
                )
                .unwrap();
            Self {
                _generated: generated,
                _state: state,
                store,
                registry,
                session,
                selection,
                prepared,
            }
        }

        fn lease_path(&self) -> std::path::PathBuf {
            self._generated
                .path()
                .join("leases")
                .join(&self.selection.lease_name)
        }

        fn launch_path(&self) -> std::path::PathBuf {
            self._state
                .path()
                .join("helm/activation/launches")
                .join(self.prepared.record.launch.encode())
        }

        fn direct_evidence(&self) -> VerifiedOwnership {
            TestOwnershipVerifier {
                proof: TestOwnershipProof::Direct,
            }
            .verify(&self.prepared)
            .unwrap()
        }

        fn systemd_evidence(&self) -> VerifiedOwnership {
            TestOwnershipVerifier {
                proof: TestOwnershipProof::Systemd {
                    unit_invocation: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    cgroup: "app.slice/helm-launch.scope".into(),
                    cgroup_device: 11,
                    cgroup_inode: 13,
                },
            }
            .verify(&self.prepared)
            .unwrap()
        }
    }

    fn matching_foreign_selection(
        fixture: &TransferFixture,
    ) -> (tempfile::TempDir, GenerationStore, GenerationSelection) {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let digest = "a".repeat(64);
        let generation = fixture.selection.generation.clone();
        store
            .publish_with_checkpoint_and_ids(
                || {
                    Ok(GenerationPublication {
                        input_digests: [
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                            digest.clone(),
                        ],
                        outputs: vec![("fixture".into(), b"sealed".to_vec())],
                    })
                },
                |_| Ok(()),
                || Ok(generation.clone()),
            )
            .unwrap();
        let selection = store
            .select_current_for_process(std::process::id())
            .unwrap();
        (generated, store, selection)
    }

    fn add_systemd_running(
        fixture: &TransferFixture,
        launch_id: &str,
    ) -> (LaunchId, std::path::PathBuf, std::path::PathBuf) {
        let selection = fixture
            .store
            .select_current_for_process(std::process::id())
            .unwrap();
        let lease_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(&selection.lease_name);
        let launch = LaunchId::parse(launch_id).unwrap();
        let prepared = fixture
            .registry
            .prepare(
                &fixture.session,
                PrepareLaunch {
                    launch,
                    mode: OwnershipMode::Systemd,
                },
                &selection,
            )
            .unwrap();
        let evidence = TestOwnershipVerifier {
            proof: TestOwnershipProof::Systemd {
                unit_invocation: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                cgroup: format!("app.slice/helm-launch-{}.scope", launch.encode()),
                cgroup_device: 17,
                cgroup_inode: 19,
            },
        }
        .verify(&prepared)
        .unwrap();
        let adopted = fixture
            .registry
            .adopt_prepared(&fixture.session, prepared, selection, evidence)
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let launch_path = fixture
            ._state
            .path()
            .join("helm/activation/launches")
            .join(launch.encode());
        (launch, launch_path, lease_path)
    }

    fn add_direct_preparing(
        fixture: &TransferFixture,
        launch_id: &str,
    ) -> (
        LaunchId,
        std::path::PathBuf,
        std::path::PathBuf,
        GenerationSelection,
    ) {
        let selection = fixture
            .store
            .select_current_for_process(std::process::id())
            .unwrap();
        let lease_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(&selection.lease_name);
        let launch = LaunchId::parse(launch_id).unwrap();
        let prepared = fixture
            .registry
            .prepare(
                &fixture.session,
                PrepareLaunch {
                    launch,
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .unwrap();
        let launch_path = fixture
            ._state
            .path()
            .join("helm/activation/launches")
            .join(prepared.record.launch.encode());
        (launch, launch_path, lease_path, selection)
    }

    fn authorize_and_run(registry: &ActivationRegistry, adopted: &AdoptedLaunch) -> LaunchRecord {
        let mut authorized = adopted.record.clone();
        authorized.sequence += 1;
        authorized.exec_open = true;
        registry
            .replace_launch_record(&adopted.record, &authorized)
            .unwrap();
        let mut running = authorized.clone();
        running.sequence += 1;
        running.state = LaunchState::Running;
        registry
            .replace_launch_record(&authorized, &running)
            .unwrap();
        running
    }

    fn terminal_direct(registry: &ActivationRegistry, running: &LaunchRecord) -> LaunchRecord {
        let mut terminal = running.clone();
        terminal.sequence += 1;
        terminal.state = LaunchState::Terminal;
        terminal.result = LaunchResult::Exited;
        terminal.direct_drained = true;
        registry.replace_launch_record(running, &terminal).unwrap();
        terminal
    }

    #[test]
    fn preparing_direct_record_requires_owner_process_group() {
        assert!(LaunchRecord::parse(&direct_preparing("process-group 0")).is_err());
    }

    #[test]
    fn canonical_preparing_records_accept_only_pre_adoption_ownership() {
        assert!(LaunchRecord::parse(&direct_preparing("process-group 42")).is_ok());
        assert!(LaunchRecord::parse(&systemd_preparing(
            "none",
            "helm-launch-11111111111111111111111111111111.scope",
        ))
        .is_ok());
        assert!(LaunchRecord::parse(&systemd_preparing(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "helm-launch-11111111111111111111111111111111.scope",
        ))
        .is_err());
        assert!(
            LaunchRecord::parse(&systemd_preparing("none", "helm-launch-wrong.scope")).is_err()
        );
    }

    #[test]
    fn terminal_failed_systemd_process_lease_retains_pre_adoption_identity() {
        let terminal = String::from_utf8(systemd_preparing(
            "none",
            "helm-launch-11111111111111111111111111111111.scope",
        ))
        .unwrap()
        .replace("state preparing", "state terminal")
        .replace("result none", "result failed")
        .into_bytes();
        let record = LaunchRecord::parse(&terminal).unwrap();
        assert_eq!(record.lease_kind, LeaseKind::Process);
        assert_eq!(record.unit_invocation, "none");
        assert_eq!(record.cgroup, "none");
    }

    #[test]
    fn launch_parser_rejects_duplicate_cr_unterminated_and_oversized_records() {
        let valid = direct_preparing("process-group 42");
        let duplicate = String::from_utf8(valid.clone())
            .unwrap()
            .replace("sequence 1\n", "sequence 1\nsequence 1\n")
            .into_bytes();
        assert!(LaunchRecord::parse(&duplicate).is_err());

        let mut cr = valid.clone();
        cr.insert(10, b'\r');
        assert!(LaunchRecord::parse(&cr).is_err());

        let mut unterminated = valid.clone();
        unterminated.pop();
        assert!(LaunchRecord::parse(&unterminated).is_err());

        let mut oversized = valid;
        oversized.resize(MAX_RECORD_BYTES + 1, b'x');
        assert!(LaunchRecord::parse(&oversized).is_err());
    }

    #[test]
    fn claimed_session_rejects_prepublished_compositor_or_helpers() {
        assert!(SessionRecord::parse(&exact_session_record("claimed", 1)).is_err());
        let pid = std::process::id();
        let (start_time, uid) = linux_process_identity(pid).unwrap();
        let preparing_without_publication = String::from_utf8(
            SessionRecord::claimed(
                SessionId::parse("22222222222222222222222222222222").unwrap(),
                pid,
                start_time,
                uid,
                linux_boot_id().unwrap(),
            )
            .encode(),
        )
        .unwrap()
        .replace("state claimed", "state preparing")
        .into_bytes();
        assert!(SessionRecord::parse(&preparing_without_publication).is_err());
        let preparing_without_compositor = String::from_utf8(exact_session_record("active", 2))
            .unwrap()
            .replace("state active", "state preparing")
            .replace(
                &format!("compositor-pid {}", std::process::id()),
                "compositor-pid 0",
            )
            .replace(
                &format!(
                    "compositor-start-time {}",
                    linux_process_identity(std::process::id()).unwrap().0
                ),
                "compositor-start-time 0",
            )
            .into_bytes();
        assert!(SessionRecord::parse(&preparing_without_compositor).is_err());
    }

    #[test]
    fn terminal_launch_parser_accepts_only_producer_reachable_tuples() {
        let direct = String::from_utf8(direct_preparing("process-group 42")).unwrap();
        let terminal_process_exited = direct
            .replace("state preparing", "state terminal")
            .replace("result none", "result exited");
        assert!(LaunchRecord::parse(terminal_process_exited.as_bytes()).is_err());

        let terminal_process_open = direct
            .replace("state preparing", "state terminal")
            .replace("result none", "result failed")
            .replace("exec-open no", "exec-open yes");
        assert!(LaunchRecord::parse(terminal_process_open.as_bytes()).is_err());

        let direct_lifecycle_failed = direct
            .replace("state preparing", "state terminal")
            .replace("result none", "result failed")
            .replace("lease-kind process", "lease-kind lifecycle");
        assert!(LaunchRecord::parse(direct_lifecycle_failed.as_bytes()).is_err());

        let systemd_lifecycle_lost = String::from_utf8(systemd_preparing(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "helm-launch-11111111111111111111111111111111.scope",
        ))
        .unwrap()
        .replace("state preparing", "state terminal")
        .replace("result none", "result lost")
        .replace("lease-kind process", "lease-kind lifecycle")
        .replace("cgroup none", "cgroup app.slice/helm")
        .replace("cgroup-device 0", "cgroup-device 1")
        .replace("cgroup-inode 0", "cgroup-inode 2");
        assert!(LaunchRecord::parse(systemd_lifecycle_lost.as_bytes()).is_err());
    }

    #[test]
    fn open_initializes_descriptor_safe_activation_root() {
        let root = tempfile::tempdir().unwrap();
        let registry = open_test_registry(root.path()).unwrap();
        drop(registry);

        for directory in [
            root.path().join("helm"),
            root.path().join("helm/activation"),
            root.path().join("helm/activation/launches"),
        ] {
            let metadata = fs::symlink_metadata(directory).unwrap();
            assert!(metadata.is_dir());
            assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
        }
        let lock =
            fs::symlink_metadata(root.path().join("helm/activation/lifecycle.lock")).unwrap();
        assert!(lock.is_file());
        assert_eq!(lock.len(), 0);
        assert_eq!(lock.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn open_rejects_empty_or_relative_state_home_without_mutation() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        assert!(ActivationRegistry::open(
            Path::new(""),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .is_err());
        let relative = format!("relative-activation-test-{}", std::process::id());
        assert!(ActivationRegistry::open(
            Path::new(&relative),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .is_err());
        assert!(!Path::new(&relative).exists());
    }

    #[test]
    fn open_rejects_symlinked_or_wrong_mode_lock_without_replacement() {
        for symlinked in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let activation = root.path().join("helm/activation");
            fs::create_dir_all(&activation).unwrap();
            fs::set_permissions(root.path().join("helm"), fs::Permissions::from_mode(0o700))
                .unwrap();
            fs::set_permissions(&activation, fs::Permissions::from_mode(0o700)).unwrap();
            let lock = activation.join("lifecycle.lock");
            if symlinked {
                let target = root.path().join("target");
                write_mode(&target, b"sentinel", 0o600);
                symlink(&target, &lock).unwrap();
                assert!(open_test_registry(root.path()).is_err());
                assert_eq!(fs::read(target).unwrap(), b"sentinel");
                assert!(fs::symlink_metadata(lock).unwrap().file_type().is_symlink());
            } else {
                write_mode(&lock, b"", 0o644);
                assert!(open_test_registry(root.path()).is_err());
                assert_eq!(
                    fs::symlink_metadata(lock).unwrap().permissions().mode() & 0o777,
                    0o644
                );
            }
        }
    }

    #[test]
    fn failed_lock_path_revalidation_releases_kernel_lock() {
        let root = tempfile::tempdir().unwrap();
        let registry = open_test_registry(root.path()).unwrap();
        let activation = root.path().join("helm/activation");
        let lock = activation.join("lifecycle.lock");
        let displaced = activation.join("displaced-lock");
        fs::rename(&lock, &displaced).unwrap();
        write_mode(&lock, b"", 0o600);

        let session = SessionId::parse("22222222222222222222222222222222").unwrap();
        assert!(registry.open_active_session(session, 1).is_err());
        let probe = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(displaced)
            .unwrap();
        assert!(flock(&probe, FlockOperation::NonBlockingLockExclusive).is_ok());
    }

    #[test]
    fn session_claim_is_exact_and_active_reopen_revalidates_live_identity() {
        let root = tempfile::tempdir().unwrap();
        let registry = open_test_registry(root.path()).unwrap();
        let session = SessionId::parse("22222222222222222222222222222222").unwrap();
        let claim = registry
            .claim_session(SessionClaimRequest {
                session,
                entry_pid: std::process::id(),
            })
            .unwrap();
        assert_eq!(claim.record.session, session);
        assert_eq!(claim.record.sequence, 1);
        assert!(registry
            .claim_session(SessionClaimRequest {
                session: SessionId::parse("55555555555555555555555555555555").unwrap(),
                entry_pid: std::process::id(),
            })
            .is_err());
        assert!(registry.open_active_session(session, 1).is_err());
        drop(registry);

        write_mode(
            &root.path().join("helm/activation/session"),
            &exact_session_record("active", 7),
            0o600,
        );
        let registry = open_test_registry(root.path()).unwrap();
        assert!(registry.open_active_session(session, 7).is_ok());
        assert!(registry.open_active_session(session, 6).is_err());

        let stale = String::from_utf8(exact_session_record("active", 7))
            .unwrap()
            .replace("entry-start-time ", "entry-start-time 9")
            .into_bytes();
        write_mode(&root.path().join("helm/activation/session"), &stale, 0o600);
        assert!(registry.open_active_session(session, 7).is_err());
    }

    #[test]
    fn inventory_discards_only_exact_safe_unpublished_temporaries() {
        let root = tempfile::tempdir().unwrap();
        drop(open_test_registry(root.path()).unwrap());
        let activation = root.path().join("helm/activation");
        let claim = activation.join(".session-claim-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let create = activation.join(
            "launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        write_mode(&claim, b"", 0o600);
        write_mode(&create, &[0xff, 0xfe], 0o600);
        drop(open_test_registry(root.path()).unwrap());
        assert!(!claim.exists());
        assert!(!create.exists());

        let malformed = activation.join(".session-claim-not-an-id");
        write_mode(&malformed, b"", 0o600);
        assert!(open_test_registry(root.path()).is_err());
        assert!(malformed.exists());
    }

    #[test]
    fn cross_namespace_temporaries_are_retained_and_fail_closed() {
        for activation_name in [true, false] {
            let root = tempfile::tempdir().unwrap();
            drop(open_test_registry(root.path()).unwrap());
            let path = if activation_name {
                root.path().join(
                    "helm/activation/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
                )
            } else {
                root.path().join(
                    "helm/activation/launches/.session-claim-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
            };
            write_mode(&path, b"", 0o600);
            assert!(open_test_registry(root.path()).is_err());
            assert!(path.exists());
        }
    }

    #[test]
    fn inventory_retains_unsafe_temporary_and_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        drop(open_test_registry(root.path()).unwrap());
        let temporary = root.path().join(
            "helm/activation/launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        write_mode(&temporary, b"", 0o644);
        assert!(open_test_registry(root.path()).is_err());
        assert!(temporary.exists());
        assert_eq!(
            fs::symlink_metadata(temporary)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
    }

    #[test]
    fn inventory_retains_reserved_temporary_with_wrong_type() {
        let root = tempfile::tempdir().unwrap();
        drop(open_test_registry(root.path()).unwrap());
        let temporary = root.path().join(
            "helm/activation/launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        fs::create_dir(&temporary).unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(open_test_registry(root.path()).is_err());
        assert!(temporary.is_dir());
    }

    #[test]
    fn temporary_inode_replacement_before_delete_is_retained() {
        let root = tempfile::tempdir().unwrap();
        let registry = open_test_registry(root.path()).unwrap();
        let temporary = root
            .path()
            .join("helm/activation/.session-claim-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let displaced = root.path().join("helm/activation/displaced-temporary");
        write_mode(&temporary, b"original", 0o600);

        let _lock = registry.lock().unwrap();
        let result = registry.inspect_and_recover_inventory_with_checkpoint(|| {
            fs::rename(&temporary, &displaced).unwrap();
            write_mode(&temporary, b"replacement evidence", 0o600);
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&temporary).unwrap(), b"replacement evidence");
        assert_eq!(fs::read(&displaced).unwrap(), b"original");
    }

    #[test]
    fn inventory_bounds_fail_before_discard() {
        assert!(inventory_within_bounds(4097, 0).is_err());
        assert!(inventory_within_bounds(1, MAX_INVENTORY_BYTES + 1).is_err());
    }

    #[test]
    fn bounded_inventory_enumeration_is_canonical_by_name() {
        let mut names = [
            OsString::from("ffffffffffffffffffffffffffffffff"),
            OsString::from("00000000000000000000000000000000"),
        ];

        canonicalize_inventory_names(&mut names);

        assert_eq!(
            names,
            [
                OsString::from("00000000000000000000000000000000"),
                OsString::from("ffffffffffffffffffffffffffffffff"),
            ]
        );
    }

    #[test]
    fn transfer_stage_classifier_rejects_over_bound_inventory() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let record = LeaseRecord::for_process(
            GenerationId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            std::process::id(),
        )
        .unwrap()
        .encode();
        for index in 0..=MAX_INVENTORY_ENTRIES {
            write_mode(
                &generated
                    .path()
                    .join("leases")
                    .join(format!("{index:032x}")),
                &record,
                0o600,
            );
        }

        let error = classify_lease_transfer_staging_locked(&store.leases.fd).unwrap_err();

        assert!(error.contains("bounds"), "{error}");
    }

    #[test]
    fn over_bound_inventory_retains_every_safe_temporary() {
        let root = tempfile::tempdir().unwrap();
        drop(open_test_registry(root.path()).unwrap());
        let launches = root.path().join("helm/activation/launches");
        for index in 0..=MAX_INVENTORY_ENTRIES {
            let temporary = launches.join(format!(
                ".launch-create-{index:032x}-ffffffffffffffffffffffffffffffff"
            ));
            write_mode(&temporary, b"", 0o600);
        }
        assert!(open_test_registry(root.path()).is_err());
        assert_eq!(
            fs::read_dir(launches).unwrap().count(),
            MAX_INVENTORY_ENTRIES + 1
        );
    }

    #[test]
    fn prepare_publishes_exact_gate_closed_selection_identity() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let digest = "a".repeat(64);
        store
            .publish(|| {
                Ok(GenerationPublication {
                    input_digests: [
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                    ],
                    outputs: vec![("fixture".into(), b"sealed".to_vec())],
                })
            })
            .unwrap();
        let selection = store
            .select_current_for_process(std::process::id())
            .unwrap();

        let state = tempfile::tempdir().unwrap();
        drop(
            ActivationRegistry::open(
                state.path(),
                GenerationLeaseCapability::from_store(&store).unwrap(),
            )
            .unwrap(),
        );
        write_mode(
            &state.path().join("helm/activation/session"),
            &exact_session_record("active", 7),
            0o600,
        );
        let registry = ActivationRegistry::open(
            state.path(),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .unwrap();
        let session = registry
            .open_active_session(
                SessionId::parse("22222222222222222222222222222222").unwrap(),
                7,
            )
            .unwrap();
        let launch = LaunchId::parse("11111111111111111111111111111111").unwrap();
        let prepared = registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch,
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .unwrap();

        assert_eq!(prepared.record.launch, launch);
        assert_eq!(prepared.record.generation.as_str(), selection.as_str());
        assert_eq!(
            prepared.record.manifest_sha256,
            selection.manifest_seal_digest
        );
        assert_eq!(prepared.record.lease, selection.lease_name);
        assert_eq!(prepared.record.owner_pid, std::process::id());
        assert_eq!(prepared.record.process_group, std::process::id());
        assert!(!prepared.record.exec_open);
        assert_eq!(
            read_launch_record(
                &registry.launches.fd,
                OsStr::new("11111111111111111111111111111111"),
            )
            .unwrap(),
            prepared.record,
        );
        assert!(registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch,
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .is_err());

        let selection_without_lease = store
            .select_current_for_process(std::process::id())
            .unwrap();
        fs::remove_file(
            generated
                .path()
                .join("leases")
                .join(&selection_without_lease.lease_name),
        )
        .unwrap();
        let absent_lease_launch = LaunchId::parse("66666666666666666666666666666666").unwrap();
        assert!(registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch: absent_lease_launch,
                    mode: OwnershipMode::Direct,
                },
                &selection_without_lease,
            )
            .is_err());
        assert!(!state
            .path()
            .join("helm/activation/launches/66666666666666666666666666666666")
            .exists());
    }

    #[test]
    fn prepare_rejects_a_selection_from_another_generation_store() {
        let fixture = TransferFixture::direct();
        let (_foreign_root, _foreign_store, foreign_selection) =
            matching_foreign_selection(&fixture);
        let foreign_lease = foreign_selection.lease_name.clone();
        assert_eq!(foreign_selection.generation, fixture.selection.generation);
        assert_eq!(
            foreign_selection.manifest_seal_digest,
            fixture.selection.manifest_seal_digest
        );
        assert_eq!(
            foreign_selection.process_identity,
            fixture.selection.process_identity
        );

        let result = fixture.registry.prepare(
            &fixture.session,
            PrepareLaunch {
                launch: LaunchId::parse("99999999999999999999999999999999").unwrap(),
                mode: OwnershipMode::Direct,
            },
            &foreign_selection,
        );

        assert!(result.is_err());
        assert!(statat(
            &foreign_selection.lease_directory,
            foreign_lease.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        )
        .is_ok());
    }

    #[test]
    fn adopt_rejects_a_byte_matching_selection_from_another_generation_store() {
        let fixture = TransferFixture::direct();
        let (_foreign_root, _foreign_store, mut foreign_selection) =
            matching_foreign_selection(&fixture);
        let original_foreign_name = foreign_selection.lease_name.clone();
        let matching_name = fixture.selection.lease_name.clone();
        renameat(
            &foreign_selection.lease_directory,
            original_foreign_name.as_str(),
            &foreign_selection.lease_directory,
            matching_name.as_str(),
        )
        .unwrap();
        foreign_selection.lease_name = matching_name.clone();
        let foreign_lease = _foreign_root.path().join("leases").join(&matching_name);
        validate_selection_matches_launch(&foreign_selection, &fixture.prepared.record).unwrap();
        foreign_selection
            .revalidate_for_lifecycle_transfer()
            .unwrap();
        let evidence = fixture.direct_evidence();
        let launch_path = fixture.launch_path();
        let source_lease = fixture.lease_path();

        let result = fixture.registry.adopt_prepared(
            &fixture.session,
            fixture.prepared,
            foreign_selection,
            evidence,
        );

        assert!(result.is_err());
        assert!(foreign_lease.exists());
        assert_eq!(
            LaunchRecord::parse(&fs::read(launch_path).unwrap())
                .unwrap()
                .state,
            LaunchState::Preparing
        );
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(source_lease).unwrap()),
            Ok(ParsedLeaseRecord::Process(_))
        ));
    }

    #[test]
    fn transfer_never_leaves_an_unleased_generation() {
        for fail_at in [
            TransferCheckpoint::DuringStageWrite,
            TransferCheckpoint::BeforeReplace,
            TransferCheckpoint::BeforeExchange,
            TransferCheckpoint::AfterReplace,
            TransferCheckpoint::AfterLeaseDirectoryFsync,
            TransferCheckpoint::BeforeAdoptedRecord,
            TransferCheckpoint::BeforeSelectionDisarm,
        ] {
            let fixture = TransferFixture::direct();
            let lease_path = fixture.lease_path();
            let launch_path = fixture.launch_path();
            let evidence = fixture.direct_evidence();
            let mut filesystem = FaultingLeaseFilesystem {
                fail_at,
                lease_path: lease_path.clone(),
                launch_path: launch_path.clone(),
                observations: Vec::new(),
            };
            let result = fixture.registry.adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            );
            assert!(result.is_err(), "{fail_at:?} did not inject a failure");
            assert_eq!(filesystem.observations.last().unwrap().0, fail_at);
            for (checkpoint, lease, launch) in &filesystem.observations {
                let process_lease_exists = matches!(lease, ParsedLeaseRecord::Process(_));
                let lifecycle_lease_exists = matches!(lease, ParsedLeaseRecord::Lifecycle(_));
                assert!(process_lease_exists || lifecycle_lease_exists);
                assert_ne!(process_lease_exists, lifecycle_lease_exists);
                match checkpoint {
                    TransferCheckpoint::DuringStageWrite
                    | TransferCheckpoint::BeforeReplace
                    | TransferCheckpoint::BeforeExchange => {
                        assert!(process_lease_exists);
                        assert_eq!(launch.state, LaunchState::Preparing);
                    }
                    TransferCheckpoint::AfterReplace
                    | TransferCheckpoint::AfterLeaseDirectoryFsync
                    | TransferCheckpoint::BeforeAdoptedRecord => {
                        assert!(lifecycle_lease_exists);
                        assert_eq!(launch.state, LaunchState::Preparing);
                    }
                    TransferCheckpoint::BeforeSelectionDisarm => {
                        assert!(lifecycle_lease_exists);
                        assert_eq!(launch.state, LaunchState::Adopted);
                    }
                }
            }
            if matches!(
                fail_at,
                TransferCheckpoint::DuringStageWrite
                    | TransferCheckpoint::BeforeReplace
                    | TransferCheckpoint::BeforeExchange
            ) {
                assert!(!lease_path.exists());
            } else {
                assert!(matches!(
                    ParsedLeaseRecord::parse(&fs::read(&lease_path).unwrap()),
                    Ok(ParsedLeaseRecord::Lifecycle(_))
                ));
            }
        }
    }

    #[test]
    fn post_replace_drop_retains_lifecycle_lease() {
        let fixture = TransferFixture::direct();
        let lease_path = fixture.lease_path();
        let evidence = fixture.direct_evidence();
        let mut filesystem = FaultingLeaseFilesystem {
            fail_at: TransferCheckpoint::AfterReplace,
            lease_path: lease_path.clone(),
            launch_path: fixture.launch_path(),
            observations: Vec::new(),
        };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_err());
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
    }

    #[test]
    fn successful_transfers_persist_exact_direct_and_systemd_evidence() {
        let direct = TransferFixture::direct();
        let direct_lease_path = direct.lease_path();
        let direct_evidence = direct.direct_evidence();
        let adopted = direct
            .registry
            .adopt_prepared(
                &direct.session,
                direct.prepared,
                direct.selection,
                direct_evidence,
            )
            .unwrap();
        assert_eq!(adopted.record.state, LaunchState::Adopted);
        assert_eq!(adopted.record.lease_kind, LeaseKind::Lifecycle);
        assert_eq!(adopted.record.process_group, adopted.record.owner_pid);
        let ParsedLeaseRecord::Lifecycle(direct_lease) =
            ParsedLeaseRecord::parse(&fs::read(direct_lease_path).unwrap()).unwrap()
        else {
            panic!("successful direct adoption did not persist a lifecycle lease");
        };
        assert_eq!(direct_lease.launch.as_str(), adopted.record.launch.encode());
        assert_eq!(direct_lease.process_group, adopted.record.owner_pid);

        let systemd = TransferFixture::systemd();
        let systemd_lease_path = systemd.lease_path();
        let systemd_evidence = systemd.systemd_evidence();
        let adopted = systemd
            .registry
            .adopt_prepared(
                &systemd.session,
                systemd.prepared,
                systemd.selection,
                systemd_evidence,
            )
            .unwrap();
        assert_eq!(adopted.record.state, LaunchState::Adopted);
        assert_eq!(adopted.record.lease_kind, LeaseKind::Lifecycle);
        assert_eq!(
            adopted.record.unit_invocation,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(adopted.record.cgroup, "app.slice/helm-launch.scope");
        assert_eq!(adopted.record.cgroup_device, 11);
        assert_eq!(adopted.record.cgroup_inode, 13);
        let ParsedLeaseRecord::Lifecycle(systemd_lease) =
            ParsedLeaseRecord::parse(&fs::read(systemd_lease_path).unwrap()).unwrap()
        else {
            panic!("successful systemd adoption did not persist a lifecycle lease");
        };
        assert_eq!(systemd_lease.unit, adopted.record.unit);
        assert_eq!(
            systemd_lease.unit_invocation,
            adopted.record.unit_invocation
        );
        assert_eq!(systemd_lease.cgroup, adopted.record.cgroup);
        assert_eq!(systemd_lease.cgroup_device, adopted.record.cgroup_device);
        assert_eq!(systemd_lease.cgroup_inode, adopted.record.cgroup_inode);
    }

    #[test]
    fn ownership_evidence_cannot_be_rebound_to_another_prepared_launch() {
        let source = TransferFixture::systemd();
        let source_evidence = source.systemd_evidence();
        let target =
            TransferFixture::new("77777777777777777777777777777777", OwnershipMode::Systemd);
        assert!(target
            .registry
            .adopt_prepared(
                &target.session,
                target.prepared,
                target.selection,
                source_evidence,
            )
            .is_err());
    }

    #[test]
    fn unnamed_stage_uses_descriptor_validated_proc_fallback() {
        let fixture = TransferFixture::direct();
        let evidence = fixture.direct_evidence();
        let mut filesystem = ForcingProcFallbackFilesystem {
            empty_path_attempts: 0,
            proc_attempts: 0,
        };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_ok());
        assert_eq!(filesystem.empty_path_attempts, 1);
        assert_eq!(filesystem.proc_attempts, 1);
    }

    #[test]
    fn proc_fallback_rejects_a_source_not_matching_the_held_tmpfile() {
        let fixture = TransferFixture::direct();
        let decoy_path = fixture._generated.path().join("proc-fd-decoy");
        write_mode(&decoy_path, b"decoy", 0o600);
        let decoy = fs::File::open(decoy_path).unwrap().into();
        let evidence = fixture.direct_evidence();
        let mut filesystem = MismatchedProcFallbackFilesystem { decoy };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_err());
    }

    #[test]
    fn transfer_rejects_stale_session_launch_selection_and_evidence_without_replacement() {
        fn rejects(mut fixture: TransferFixture, mutate: impl FnOnce(&mut TransferFixture)) {
            let launch_path = fixture.launch_path();
            mutate(&mut fixture);
            let lease_path = fixture.lease_path();
            let evidence = fixture.direct_evidence();
            let result = fixture.registry.adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            );
            assert!(result.is_err());
            if lease_path.exists() {
                assert!(!matches!(
                    ParsedLeaseRecord::parse(&fs::read(&lease_path).unwrap()),
                    Ok(ParsedLeaseRecord::Lifecycle(_))
                ));
            }
            assert_eq!(
                LaunchRecord::parse(&fs::read(launch_path).unwrap())
                    .unwrap()
                    .state,
                LaunchState::Preparing
            );
        }

        rejects(TransferFixture::direct(), |fixture| {
            fixture.session.record.session =
                SessionId::parse("99999999999999999999999999999999").unwrap();
        });
        rejects(TransferFixture::direct(), |fixture| {
            fixture.session.record.sequence += 1;
        });
        rejects(TransferFixture::direct(), |fixture| {
            fixture.prepared.record.sequence += 1;
        });
        rejects(TransferFixture::direct(), |fixture| {
            fixture.prepared.record.launch =
                LaunchId::parse("99999999999999999999999999999999").unwrap();
        });
        rejects(TransferFixture::direct(), |fixture| {
            fixture.prepared.record.generation =
                GenerationId::parse("99999999999999999999999999999999").unwrap();
        });
        rejects(TransferFixture::direct(), |fixture| {
            fixture.prepared.record.manifest_sha256 = "9".repeat(64);
        });

        let mut fixture = TransferFixture::direct();
        let original_lease_path = fixture.lease_path();
        let other = fixture
            .store
            .select_current_for_process(std::process::id())
            .unwrap();
        let evidence = fixture.direct_evidence();
        assert!(fixture
            .registry
            .adopt_prepared(&fixture.session, fixture.prepared, other, evidence,)
            .is_err());
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(original_lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Process(_))
        ));

        fixture = TransferFixture::direct();
        let mut binding = OwnershipBinding::from_preparing(&fixture.prepared.record);
        binding.process_group += 1;
        let evidence = VerifiedOwnership::Direct { binding };
        assert!(fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .is_err());

        let fixture = TransferFixture::systemd();
        let evidence = VerifiedOwnership::Direct {
            binding: OwnershipBinding::from_preparing(&fixture.prepared.record),
        };
        assert!(fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .is_err());
    }

    #[test]
    fn transfer_rejects_absent_cross_kind_and_mismatched_process_leases() {
        let absent = TransferFixture::direct();
        let absent_path = absent.lease_path();
        fs::remove_file(&absent_path).unwrap();
        let evidence = absent.direct_evidence();
        assert!(absent
            .registry
            .adopt_prepared(&absent.session, absent.prepared, absent.selection, evidence,)
            .is_err());
        assert!(!absent_path.exists());

        for mismatched_launch in [false, true] {
            let fixture = TransferFixture::direct();
            let lease_path = fixture.lease_path();
            let launch = if mismatched_launch {
                GenerationId::parse("99999999999999999999999999999999").unwrap()
            } else {
                GenerationId::parse(&fixture.prepared.record.launch.encode()).unwrap()
            };
            let record = LifecycleLeaseRecord {
                generation: fixture.selection.generation.clone(),
                launch,
                pid: fixture.selection.process_identity.pid,
                start_time: fixture.selection.process_identity.start_time,
                boot_id: fixture.selection.process_identity.boot_id.clone(),
                owner_uid: fixture.selection.process_identity.owner_uid,
                owner_kind: super::LifecycleOwnerKind::ProcessGroup,
                unit: "none".into(),
                unit_invocation: "none".into(),
                process_group: fixture.selection.process_identity.pid,
                cgroup: "none".into(),
                cgroup_device: 0,
                cgroup_inode: 0,
            };
            write_mode(&lease_path, &record.encode(), 0o600);
            let evidence = fixture.direct_evidence();
            assert!(fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .is_err());
            assert_eq!(
                ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
                Ok(ParsedLeaseRecord::Lifecycle(record))
            );
        }

        let fixture = TransferFixture::direct();
        let lease_path = fixture.lease_path();
        let mut mismatched = fixture.selection.process_identity.clone();
        mismatched.start_time += 1;
        write_mode(&lease_path, &mismatched.encode(), 0o600);
        let evidence = fixture.direct_evidence();
        assert!(fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .is_err());
        assert_eq!(
            ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Process(mismatched))
        );
    }

    #[test]
    fn transfer_revalidates_the_exact_owner_process_identity() {
        let mut owner = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let fixture = TransferFixture::new_for_owner(
            "11111111111111111111111111111111",
            OwnershipMode::Direct,
            owner.id(),
        );
        owner.kill().unwrap();
        owner.wait().unwrap();

        let evidence = fixture.direct_evidence();
        assert!(fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .is_err());
    }

    #[test]
    fn lease_transfer_never_unlinks_a_replaced_staging_inode() {
        let fixture = TransferFixture::direct();
        let lease_path = fixture.lease_path();
        let process = fixture.selection.process_identity.clone();
        let displaced_path = fixture._generated.path().join("displaced-transfer");
        let evidence = fixture.direct_evidence();
        let mut filesystem = SwappingLeaseFilesystem {
            lease_path: lease_path.clone(),
            displaced_path: displaced_path.clone(),
        };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_err());
        let replacement = fixture._generated.path().join(format!(
            "leases/.lease-transfer-{}",
            filesystem.lease_path.file_name().unwrap().to_str().unwrap()
        ));
        assert_eq!(fs::read(replacement).unwrap(), b"replacement evidence");
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(displaced_path).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
        assert_eq!(
            ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Process(process))
        );
    }

    #[test]
    fn generic_gc_retains_exact_post_exchange_lifecycle_process_pair() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let staging_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{lease_name}"));
        let evidence = fixture.direct_evidence();
        let (lifecycle, _) =
            transfer_records(&fixture.prepared.record, &fixture.selection, evidence).unwrap();
        write_mode(&staging_path, &lifecycle.encode(), 0o600);
        renameat_with(
            &fixture.selection.lease_directory,
            staging_path.file_name().unwrap(),
            &fixture.selection.lease_directory,
            lease_name.as_str(),
            RenameFlags::EXCHANGE,
        )
        .unwrap();
        std::mem::forget(fixture.selection);

        let report = fixture.store.garbage_collect().unwrap();
        assert_eq!(report, GenerationGcReport::default());
        assert!(staging_path.exists());
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(record)) if record == lifecycle
        ));
    }

    #[test]
    fn generic_gc_retains_exact_pre_exchange_process_lifecycle_pair() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let process = fixture.selection.process_identity.clone();
        let staging_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{lease_name}"));
        let evidence = fixture.direct_evidence();
        let (lifecycle, _) =
            transfer_records(&fixture.prepared.record, &fixture.selection, evidence).unwrap();
        write_mode(&staging_path, &lifecycle.encode(), 0o600);
        std::mem::forget(fixture.selection);

        let report = fixture.store.garbage_collect().unwrap();
        assert_eq!(report, GenerationGcReport::default());
        assert!(staging_path.exists());
        assert_eq!(
            ParsedLeaseRecord::parse(&fs::read(lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Process(process))
        );
    }

    #[test]
    fn gc_retains_everything_for_mismatched_transfer_staging_pair() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let staging_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{lease_name}"));
        let evidence = fixture.direct_evidence();
        let (mut lifecycle, _) =
            transfer_records(&fixture.prepared.record, &fixture.selection, evidence).unwrap();
        lifecycle.start_time += 1;
        write_mode(&staging_path, &lifecycle.encode(), 0o600);
        std::mem::forget(fixture.selection);

        let report = fixture.store.garbage_collect().unwrap();
        assert_eq!(report.reclaimed_leases, 0);
        assert!(lease_path.exists());
        assert!(staging_path.exists());
    }

    #[test]
    fn generic_gc_never_normalizes_valid_stage_with_unrelated_fatal_running_absent_row() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let staging_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{lease_name}"));
        let (_, fatal_launch, fatal_lease) =
            add_systemd_running(&fixture, "ffffffffffffffffffffffffffffffff");
        fs::remove_file(&fatal_lease).unwrap();
        let evidence = fixture.direct_evidence();
        let (lifecycle, _) =
            transfer_records(&fixture.prepared.record, &fixture.selection, evidence).unwrap();
        write_mode(&staging_path, &lifecycle.encode(), 0o600);
        let target_before = fs::read(&lease_path).unwrap();
        let stage_before = fs::read(&staging_path).unwrap();
        let fatal_before = fs::read(&fatal_launch).unwrap();
        std::mem::forget(fixture.selection);

        let report = fixture.store.garbage_collect().unwrap();

        assert_eq!(report, GenerationGcReport::default());
        assert_eq!(fs::read(&lease_path).unwrap(), target_before);
        assert_eq!(fs::read(&staging_path).unwrap(), stage_before);
        assert_eq!(fs::read(&fatal_launch).unwrap(), fatal_before);
        assert!(!fatal_lease.exists());
    }

    #[test]
    fn generic_gc_stage_plus_oversized_record_fails_bounded_preflight_without_mutation() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let leases = fixture._generated.path().join("leases");
        let staging_path = leases.join(format!(".lease-transfer-{lease_name}"));
        let oversized_path = leases.join("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        write_mode(&staging_path, b"stage evidence", 0o600);
        write_mode(&oversized_path, &vec![b'x'; 4097], 0o600);
        let target_before = fs::read(&lease_path).unwrap();
        let stage_before = fs::read(&staging_path).unwrap();
        let oversized_before = fs::read(&oversized_path).unwrap();
        std::mem::forget(fixture.selection);

        let error =
            crate::generation::gc_lease_inventory_preflight(&fixture.store.leases.fd).unwrap_err();
        assert!(error.contains("4096 bytes"), "{error}");
        let report = fixture.store.garbage_collect().unwrap();

        assert_eq!(report, GenerationGcReport::default());
        assert_eq!(fs::read(&lease_path).unwrap(), target_before);
        assert_eq!(fs::read(&staging_path).unwrap(), stage_before);
        assert_eq!(fs::read(&oversized_path).unwrap(), oversized_before);
    }

    #[test]
    fn generic_gc_stage_plus_over_count_fails_bounded_preflight_without_mutation() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let leases = fixture._generated.path().join("leases");
        let staging_path = leases.join(format!(".lease-transfer-{lease_name}"));
        write_mode(&staging_path, b"stage evidence", 0o600);
        let mut created = 0_usize;
        let mut index = 0_u64;
        while created < MAX_INVENTORY_ENTRIES {
            let path = leases.join(format!("{index:032x}"));
            index += 1;
            if path.exists() {
                continue;
            }
            write_mode(&path, b"", 0o600);
            created += 1;
        }
        let target_before = fs::read(&lease_path).unwrap();
        let stage_before = fs::read(&staging_path).unwrap();
        std::mem::forget(fixture.selection);

        let error =
            crate::generation::gc_lease_inventory_preflight(&fixture.store.leases.fd).unwrap_err();
        assert!(error.contains("scan bounds"), "{error}");
        let report = fixture.store.garbage_collect().unwrap();

        assert_eq!(report, GenerationGcReport::default());
        assert_eq!(fs::read(&lease_path).unwrap(), target_before);
        assert_eq!(fs::read(&staging_path).unwrap(), stage_before);
        assert_eq!(fs::read_dir(&leases).unwrap().count(), created + 2);
    }

    #[test]
    fn transfer_retry_never_normalizes_valid_stage_with_unrelated_fatal_running_absent_row() {
        let fixture = TransferFixture::direct();
        let lease_name = fixture.selection.lease_name.clone();
        let lease_path = fixture.lease_path();
        let staging_path = fixture
            ._generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{lease_name}"));
        let (_, fatal_launch, fatal_lease) =
            add_systemd_running(&fixture, "ffffffffffffffffffffffffffffffff");
        fs::remove_file(&fatal_lease).unwrap();
        let evidence = fixture.direct_evidence();
        let (lifecycle, _) = transfer_records(
            &fixture.prepared.record,
            &fixture.selection,
            fixture.direct_evidence(),
        )
        .unwrap();
        write_mode(&staging_path, &lifecycle.encode(), 0o600);
        let target_before = fs::read(&lease_path).unwrap();
        let stage_before = fs::read(&staging_path).unwrap();
        let fatal_before = fs::read(&fatal_launch).unwrap();

        let result = fixture.registry.adopt_prepared(
            &fixture.session,
            fixture.prepared,
            fixture.selection,
            evidence,
        );

        assert!(result.is_err());
        assert_eq!(fs::read(&lease_path).unwrap(), target_before);
        assert_eq!(fs::read(&staging_path).unwrap(), stage_before);
        assert_eq!(fs::read(&fatal_launch).unwrap(), fatal_before);
        assert!(!fatal_lease.exists());
    }

    #[test]
    #[ignore = "subprocess crash helper"]
    fn pre_exchange_crash_subprocess() {
        let Ok(generated) = std::env::var("HELM_TEST_CRASH_GENERATED") else {
            return;
        };
        let state = std::env::var("HELM_TEST_CRASH_STATE").unwrap();
        let store = GenerationStore::open(Path::new(&generated)).unwrap();
        let selection = store
            .select_current_for_process(std::process::id())
            .unwrap();
        let registry = ActivationRegistry::open(
            Path::new(&state),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .unwrap();
        let session = registry
            .open_active_session(
                SessionId::parse("22222222222222222222222222222222").unwrap(),
                7,
            )
            .unwrap();
        let prepared = registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch: LaunchId::parse("88888888888888888888888888888888").unwrap(),
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .unwrap();
        let evidence = TestOwnershipVerifier {
            proof: TestOwnershipProof::Direct,
        }
        .verify(&prepared)
        .unwrap();
        let mut filesystem = ExitAtTransferCheckpointFilesystem {
            checkpoint: TransferCheckpoint::BeforeExchange,
            code: 86,
        };
        let result = registry.adopt_prepared_with_filesystem(
            &session,
            prepared,
            selection,
            evidence,
            &mut filesystem,
        );
        panic!("pre-exchange crash checkpoint returned: {result:?}");
    }

    #[test]
    #[ignore = "subprocess crash helper"]
    fn partial_stage_write_crash_subprocess() {
        let Ok(generated) = std::env::var("HELM_TEST_CRASH_GENERATED") else {
            return;
        };
        let state = std::env::var("HELM_TEST_CRASH_STATE").unwrap();
        let store = GenerationStore::open(Path::new(&generated)).unwrap();
        let selection = store
            .select_current_for_process(std::process::id())
            .unwrap();
        let registry = ActivationRegistry::open(
            Path::new(&state),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .unwrap();
        let session = registry
            .open_active_session(
                SessionId::parse("22222222222222222222222222222222").unwrap(),
                7,
            )
            .unwrap();
        let prepared = registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch: LaunchId::parse("77777777777777777777777777777777").unwrap(),
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .unwrap();
        let evidence = TestOwnershipVerifier {
            proof: TestOwnershipProof::Direct,
        }
        .verify(&prepared)
        .unwrap();
        let mut filesystem = ExitAtTransferCheckpointFilesystem {
            checkpoint: TransferCheckpoint::DuringStageWrite,
            code: 87,
        };
        let result = registry.adopt_prepared_with_filesystem(
            &session,
            prepared,
            selection,
            evidence,
            &mut filesystem,
        );
        panic!("partial-stage-write crash checkpoint returned: {result:?}");
    }

    #[test]
    fn real_partial_stage_write_crash_never_publishes_named_stage() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let digest = "a".repeat(64);
        store
            .publish(|| {
                Ok(GenerationPublication {
                    input_digests: [
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                    ],
                    outputs: vec![("fixture".into(), b"sealed".to_vec())],
                })
            })
            .unwrap();
        let state = tempfile::tempdir().unwrap();
        drop(
            ActivationRegistry::open(
                state.path(),
                GenerationLeaseCapability::from_store(&store).unwrap(),
            )
            .unwrap(),
        );
        write_mode(
            &state.path().join("helm/activation/session"),
            &exact_session_record("active", 7),
            0o600,
        );

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--ignored")
            .arg("--exact")
            .arg("generation::lifecycle::tests::partial_stage_write_crash_subprocess")
            .env("HELM_TEST_CRASH_GENERATED", generated.path())
            .env("HELM_TEST_CRASH_STATE", state.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(87));

        let crashed = LaunchRecord::parse(
            &fs::read(
                state
                    .path()
                    .join("helm/activation/launches/77777777777777777777777777777777"),
            )
            .unwrap(),
        )
        .unwrap();
        let target = generated.path().join("leases").join(&crashed.lease);
        let staging = generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{}", crashed.lease));
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(target).unwrap()),
            Ok(ParsedLeaseRecord::Process(_))
        ));
        assert!(!staging.exists());
    }

    #[test]
    fn real_pre_exchange_crash_requires_registry_reconciliation_before_a_new_transfer() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let digest = "a".repeat(64);
        store
            .publish(|| {
                Ok(GenerationPublication {
                    input_digests: [
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                        digest.clone(),
                    ],
                    outputs: vec![("fixture".into(), b"sealed".to_vec())],
                })
            })
            .unwrap();
        let state = tempfile::tempdir().unwrap();
        drop(
            ActivationRegistry::open(
                state.path(),
                GenerationLeaseCapability::from_store(&store).unwrap(),
            )
            .unwrap(),
        );
        write_mode(
            &state.path().join("helm/activation/session"),
            &exact_session_record("active", 7),
            0o600,
        );

        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--ignored")
            .arg("--exact")
            .arg("generation::lifecycle::tests::pre_exchange_crash_subprocess")
            .env("HELM_TEST_CRASH_GENERATED", generated.path())
            .env("HELM_TEST_CRASH_STATE", state.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));

        let crashed_path = state
            .path()
            .join("helm/activation/launches/88888888888888888888888888888888");
        let crashed = LaunchRecord::parse(&fs::read(&crashed_path).unwrap()).unwrap();
        let target = generated.path().join("leases").join(&crashed.lease);
        let staging = generated
            .path()
            .join("leases")
            .join(format!(".lease-transfer-{}", crashed.lease));
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(&target).unwrap()),
            Ok(ParsedLeaseRecord::Process(_))
        ));
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(&staging).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));

        let target_before = fs::read(&target).unwrap();
        let stage_before = fs::read(&staging).unwrap();
        assert_eq!(
            store.garbage_collect().unwrap(),
            GenerationGcReport::default()
        );
        assert_eq!(fs::read(&target).unwrap(), target_before);
        assert_eq!(fs::read(&staging).unwrap(), stage_before);

        let registry = ActivationRegistry::open(
            state.path(),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .unwrap();
        let report = registry
            .reconcile(&exact_empty_inspector(), Some(&exact_empty_controller()))
            .unwrap();
        assert_eq!(report.terminalized, 1);
        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 1);
        assert!(!staging.exists());
        assert!(!target.exists());
        assert!(!crashed_path.exists());

        let selection = store
            .select_current_for_process(std::process::id())
            .unwrap();
        let session = registry
            .open_active_session(
                SessionId::parse("22222222222222222222222222222222").unwrap(),
                7,
            )
            .unwrap();
        let prepared = registry
            .prepare(
                &session,
                PrepareLaunch {
                    launch: LaunchId::parse("99999999999999999999999999999999").unwrap(),
                    mode: OwnershipMode::Direct,
                },
                &selection,
            )
            .unwrap();
        let evidence = TestOwnershipVerifier {
            proof: TestOwnershipProof::Direct,
        }
        .verify(&prepared)
        .unwrap();
        assert!(registry
            .adopt_prepared(&session, prepared, selection, evidence)
            .is_ok());
    }

    #[test]
    fn post_validation_staging_swap_restores_the_exact_process_target() {
        let fixture = TransferFixture::direct();
        let lease_path = fixture.lease_path();
        let process = fixture.selection.process_identity.clone();
        let launch_path = fixture.launch_path();
        let displaced_path = fixture
            ._generated
            .path()
            .join("displaced-validated-transfer");
        let evidence = fixture.direct_evidence();
        let mut filesystem = SwapAfterValidationFilesystem {
            lease_path: lease_path.clone(),
            displaced_path: displaced_path.clone(),
        };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_err());
        assert_eq!(
            ParsedLeaseRecord::parse(&fs::read(&lease_path).unwrap()),
            Ok(ParsedLeaseRecord::Process(process))
        );
        let staging = lease_path.parent().unwrap().join(format!(
            ".lease-transfer-{}",
            lease_path.file_name().unwrap().to_str().unwrap()
        ));
        assert_eq!(fs::read(staging).unwrap(), b"replacement evidence");
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(displaced_path).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
        assert_eq!(
            LaunchRecord::parse(&fs::read(launch_path).unwrap())
                .unwrap()
                .state,
            LaunchState::Preparing
        );
    }

    #[test]
    fn exchange_failure_from_a_missing_target_retains_the_lifecycle_stage() {
        let fixture = TransferFixture::direct();
        let lease_path = fixture.lease_path();
        let launch_path = fixture.launch_path();
        let lease_name = fixture.selection.lease_name.clone();
        let evidence = fixture.direct_evidence();
        let mut filesystem = RemoveTargetBeforeExchangeFilesystem {
            lease_path: lease_path.clone(),
        };
        assert!(fixture
            .registry
            .adopt_prepared_with_filesystem(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
                &mut filesystem,
            )
            .is_err());
        assert!(!lease_path.exists());
        let staging = lease_path
            .parent()
            .unwrap()
            .join(format!(".lease-transfer-{lease_name}"));
        assert!(matches!(
            ParsedLeaseRecord::parse(&fs::read(staging).unwrap()),
            Ok(ParsedLeaseRecord::Lifecycle(_))
        ));
        assert_eq!(
            LaunchRecord::parse(&fs::read(launch_path).unwrap())
                .unwrap()
                .state,
            LaunchState::Preparing
        );
    }

    #[test]
    fn valid_transfer_stage_is_not_normalized_when_unrelated_evidence_is_fatal() {
        for fatal_launch in [
            "00000000000000000000000000000000",
            "ffffffffffffffffffffffffffffffff",
        ] {
            let fixture = TransferFixture::systemd();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let lease_name = fixture.selection.lease_name.clone();
            let staging = lease_path
                .parent()
                .unwrap()
                .join(format!(".lease-transfer-{lease_name}"));
            let evidence = fixture.systemd_evidence();
            let mut filesystem = FaultingLeaseFilesystem {
                fail_at: TransferCheckpoint::AfterReplace,
                lease_path: lease_path.clone(),
                launch_path: launch_path.clone(),
                observations: Vec::new(),
            };
            assert!(fixture
                .registry
                .adopt_prepared_with_filesystem(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                    &mut filesystem,
                )
                .is_err());
            let target_before = fs::read(&lease_path).unwrap();
            let stage_before = fs::read(&staging).unwrap();

            let second_selection = fixture
                .store
                .select_current_for_process(std::process::id())
                .unwrap();
            let second_lease = fixture
                ._generated
                .path()
                .join("leases")
                .join(&second_selection.lease_name);
            fixture
                .registry
                .prepare(
                    &fixture.session,
                    PrepareLaunch {
                        launch: LaunchId::parse(fatal_launch).unwrap(),
                        mode: OwnershipMode::Direct,
                    },
                    &second_selection,
                )
                .unwrap();
            let mut mismatched = second_selection.process_identity.clone();
            mismatched.start_time += 1;
            write_mode(&second_lease, &mismatched.encode(), 0o600);
            let safe_temporary = fixture._state.path().join(
                "helm/activation/launches/.launch-create-22222222222222222222222222222222-33333333333333333333333333333333",
            );
            write_mode(&safe_temporary, b"", 0o600);
            let mut entries = 0_usize;
            let mut bytes = 0_usize;
            let ordered_launches: Vec<_> = bounded_directory_entries(
                &fixture.registry.launches.fd,
                &mut entries,
                &mut bytes,
                |_| true,
            )
            .unwrap()
            .into_iter()
            .filter_map(|name| {
                let name = name.into_string().ok()?;
                LaunchId::parse(&name).ok().map(|_| name)
            })
            .collect();
            let target_position = ordered_launches
                .iter()
                .position(|name| name == "11111111111111111111111111111111")
                .unwrap();
            let fatal_position = ordered_launches
                .iter()
                .position(|name| name == fatal_launch)
                .unwrap();
            if fatal_launch.starts_with('0') {
                assert!(fatal_position < target_position, "{ordered_launches:?}");
            } else {
                assert!(target_position < fatal_position, "{ordered_launches:?}");
            }
            let inspector = exact_empty_inspector();
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.released, 0, "fatal launch {fatal_launch}");
            assert_eq!(report.collected, 0, "fatal launch {fatal_launch}");
            assert_eq!(fs::read(&lease_path).unwrap(), target_before);
            assert_eq!(fs::read(&staging).unwrap(), stage_before);
            assert!(safe_temporary.exists(), "fatal launch {fatal_launch}");
            assert_eq!(controller.direct_calls.get(), 0);
            assert_eq!(controller.unadopted_systemd_calls.get(), 0);
            assert_eq!(controller.adopted_systemd_calls.get(), 0);
        }
    }

    #[test]
    fn reconciliation_classification_holds_the_generation_lock() {
        let fixture = TransferFixture::direct();
        let competing_store = GenerationStore::open(fixture._generated.path()).unwrap();
        let competing_writer_acquired = std::cell::Cell::new(false);
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();
        let mut filesystem = RealReconciliationFilesystem;

        fixture
            .registry
            .reconcile_with_filesystem_and_classification_checkpoint(
                &inspector,
                Some(&controller),
                &mut filesystem,
                || match flock(
                    &competing_store.lock,
                    FlockOperation::NonBlockingLockExclusive,
                ) {
                    Ok(()) => {
                        competing_writer_acquired.set(true);
                        flock(&competing_store.lock, FlockOperation::Unlock).unwrap();
                    }
                    Err(error) => assert_eq!(error, Errno::WOULDBLOCK),
                },
            )
            .unwrap();

        assert!(!competing_writer_acquired.get());
    }

    #[test]
    fn direct_owner_death_without_witness_is_retained() {
        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let mut authorized = adopted.record.clone();
        authorized.sequence += 1;
        authorized.exec_open = true;
        fixture
            .registry
            .replace_launch_record(&adopted.record, &authorized)
            .unwrap();
        let mut running = authorized.clone();
        running.sequence += 1;
        running.state = LaunchState::Running;
        fixture
            .registry
            .replace_launch_record(&authorized, &running)
            .unwrap();

        let inspector = TestOwnershipInspector {
            systemd: SystemdObservation::Uncertain,
            direct: DirectObservation::Exact {
                owner_written_witness: false,
                exact_owner_stale: true,
                recorded_group_empty: true,
            },
            systemd_calls: Cell::new(0),
            direct_calls: Cell::new(0),
        };
        let report = fixture.registry.reconcile(&inspector, None).unwrap();

        assert_eq!(report.released, 0);
        assert!(launch_path.exists());
        assert!(lease_path.exists());
        assert_eq!(inspector.direct_calls.get(), 1);
        assert_eq!(inspector.systemd_calls.get(), 0);
    }

    #[test]
    fn preparing_process_lease_uses_only_matching_gate_closed_controller() {
        for mode in [OwnershipMode::Direct, OwnershipMode::Systemd] {
            let fixture = TransferFixture::new("55555555555555555555555555555555", mode);
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let inspector = exact_empty_inspector();
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(
                report,
                ReconciliationReport {
                    terminalized: 1,
                    released: 1,
                    collected: 1,
                    retained: 0,
                }
            );
            assert!(!launch_path.exists());
            assert!(!lease_path.exists());
            assert_eq!(
                controller.direct_calls.get(),
                usize::from(mode == OwnershipMode::Direct)
            );
            assert_eq!(
                controller.unadopted_systemd_calls.get(),
                usize::from(mode == OwnershipMode::Systemd)
            );
            assert_eq!(controller.adopted_systemd_calls.get(), 0);
            assert_eq!(inspector.direct_calls.get(), 0);
            assert_eq!(inspector.systemd_calls.get(), 0);
        }
    }

    #[test]
    fn transferred_preparing_and_adopted_systemd_use_only_adopted_abort() {
        for stop_before_adopted_record in [true, false] {
            let fixture = TransferFixture::systemd();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let evidence = fixture.systemd_evidence();
            if stop_before_adopted_record {
                let mut filesystem = FaultingLeaseFilesystem {
                    fail_at: TransferCheckpoint::BeforeAdoptedRecord,
                    lease_path: lease_path.clone(),
                    launch_path: launch_path.clone(),
                    observations: Vec::new(),
                };
                assert!(fixture
                    .registry
                    .adopt_prepared_with_filesystem(
                        &fixture.session,
                        fixture.prepared,
                        fixture.selection,
                        evidence,
                        &mut filesystem,
                    )
                    .is_err());
            } else {
                fixture
                    .registry
                    .adopt_prepared(
                        &fixture.session,
                        fixture.prepared,
                        fixture.selection,
                        evidence,
                    )
                    .unwrap();
            }
            let inspector = exact_empty_inspector();
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.terminalized, 1);
            assert_eq!(report.released, 1);
            assert_eq!(report.collected, 1);
            assert!(!launch_path.exists());
            assert!(!lease_path.exists());
            assert_eq!(controller.adopted_systemd_calls.get(), 1);
            assert_eq!(controller.direct_calls.get(), 0);
            assert_eq!(controller.unadopted_systemd_calls.get(), 0);
            assert_eq!(inspector.systemd_calls.get(), 0);
        }
    }

    #[test]
    fn running_systemd_is_never_stopped_and_collects_only_after_recursive_empty_proof() {
        let fixture = TransferFixture::systemd();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.terminalized, 1);
        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 1);
        assert!(!launch_path.exists());
        assert!(!lease_path.exists());
        assert_eq!(inspector.systemd_calls.get(), 1);
        assert_eq!(controller.direct_calls.get(), 0);
        assert_eq!(controller.unadopted_systemd_calls.get(), 0);
        assert_eq!(controller.adopted_systemd_calls.get(), 0);
    }

    #[test]
    fn running_systemd_live_or_uncertain_observation_retains_without_controller_call() {
        for observation in [SystemdObservation::ExactLive, SystemdObservation::Uncertain] {
            let fixture = TransferFixture::systemd();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let evidence = fixture.systemd_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            authorize_and_run(&fixture.registry, &adopted);
            let mut inspector = exact_empty_inspector();
            inspector.systemd = observation;
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.released, 0);
            assert_eq!(report.collected, 0);
            assert!(launch_path.exists());
            assert!(lease_path.exists());
            assert_eq!(inspector.systemd_calls.get(), 1);
            assert_eq!(controller.direct_calls.get(), 0);
            assert_eq!(controller.unadopted_systemd_calls.get(), 0);
            assert_eq!(controller.adopted_systemd_calls.get(), 0);
        }
    }

    #[test]
    fn terminal_direct_requires_durable_witness_stale_owner_and_empty_exact_group() {
        for observation in [
            DirectObservation::Exact {
                owner_written_witness: true,
                exact_owner_stale: true,
                recorded_group_empty: true,
            },
            DirectObservation::Exact {
                owner_written_witness: false,
                exact_owner_stale: true,
                recorded_group_empty: true,
            },
            DirectObservation::Exact {
                owner_written_witness: true,
                exact_owner_stale: false,
                recorded_group_empty: true,
            },
            DirectObservation::Exact {
                owner_written_witness: true,
                exact_owner_stale: true,
                recorded_group_empty: false,
            },
            DirectObservation::Live,
            DirectObservation::Uncertain,
        ] {
            let fixture = TransferFixture::direct();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let evidence = fixture.direct_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            let running = authorize_and_run(&fixture.registry, &adopted);
            terminal_direct(&fixture.registry, &running);
            let mut inspector = exact_empty_inspector();
            inspector.direct = observation;

            let report = fixture.registry.reconcile(&inspector, None).unwrap();
            let releasable = matches!(
                observation,
                DirectObservation::Exact {
                    owner_written_witness: true,
                    exact_owner_stale: true,
                    recorded_group_empty: true,
                }
            );
            assert_eq!(report.released, usize::from(releasable));
            assert_eq!(launch_path.exists(), !releasable);
            assert_eq!(lease_path.exists(), !releasable);
            assert_eq!(inspector.direct_calls.get(), 1);
        }
    }

    #[test]
    fn restart_reconciliation_uses_descriptor_capability_not_state_config_path() {
        let fixture = TransferFixture::direct();
        let state_path = fixture._state.path().to_path_buf();
        let generated_path = fixture._generated.path().to_path_buf();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let lease_name = fixture.prepared.record.lease.clone();
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let running = authorize_and_run(&fixture.registry, &adopted);
        terminal_direct(&fixture.registry, &running);
        let lifecycle_bytes = fs::read(&lease_path).unwrap();

        drop(fixture.registry);
        drop(fixture.store);
        let decoy_generated = state_path.join("helm/generated");
        fs::create_dir(&decoy_generated).unwrap();
        fs::set_permissions(&decoy_generated, fs::Permissions::from_mode(0o700)).unwrap();
        let decoy_leases = decoy_generated.join("leases");
        fs::create_dir(&decoy_leases).unwrap();
        fs::set_permissions(&decoy_leases, fs::Permissions::from_mode(0o700)).unwrap();
        let decoy = decoy_leases.join(&lease_name);
        write_mode(&decoy, &lifecycle_bytes, 0o600);

        let reopened_store = GenerationStore::open(&generated_path).unwrap();
        let reopened = ActivationRegistry::open(
            &state_path,
            GenerationLeaseCapability::from_store(&reopened_store).unwrap(),
        )
        .unwrap();
        let inspector = exact_empty_inspector();
        let report = reopened.reconcile(&inspector, None).unwrap();

        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 1);
        assert!(!launch_path.exists());
        assert!(!lease_path.exists());
        assert_eq!(fs::read(decoy).unwrap(), lifecycle_bytes);
    }

    #[test]
    fn replaced_generation_capability_path_fails_before_observation_or_mutation() {
        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_name = fixture.selection.lease_name.clone();
        let leases_path = fixture._generated.path().join("leases");
        let displaced = fixture._generated.path().join("displaced-leases");
        fs::rename(&leases_path, &displaced).unwrap();
        fs::create_dir(&leases_path).unwrap();
        fs::set_permissions(&leases_path, fs::Permissions::from_mode(0o700)).unwrap();
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        assert!(fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .is_err());
        assert!(launch_path.exists());
        assert!(displaced.join(lease_name).exists());
        assert!(fs::read_dir(&leases_path).unwrap().next().is_none());
        assert_eq!(inspector.direct_calls.get(), 0);
        assert_eq!(inspector.systemd_calls.get(), 0);
        assert_eq!(controller.direct_calls.get(), 0);
        assert_eq!(controller.unadopted_systemd_calls.get(), 0);
        assert_eq!(controller.adopted_systemd_calls.get(), 0);
    }

    #[test]
    fn malformed_cross_kind_and_mismatched_leases_retain_without_adapter_calls() {
        for case in ["malformed", "cross-kind", "mismatched"] {
            let fixture = TransferFixture::direct();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            match case {
                "malformed" => write_mode(&lease_path, b"malformed lease\n", 0o600),
                "cross-kind" => {
                    let evidence = fixture.direct_evidence();
                    let mut filesystem = FaultingLeaseFilesystem {
                        fail_at: TransferCheckpoint::BeforeAdoptedRecord,
                        lease_path: lease_path.clone(),
                        launch_path: launch_path.clone(),
                        observations: Vec::new(),
                    };
                    assert!(fixture
                        .registry
                        .adopt_prepared_with_filesystem(
                            &fixture.session,
                            fixture.prepared,
                            fixture.selection,
                            evidence,
                            &mut filesystem,
                        )
                        .is_err());
                }
                "mismatched" => {
                    let mut process = fixture.selection.process_identity.clone();
                    process.start_time += 1;
                    write_mode(&lease_path, &process.encode(), 0o600);
                }
                _ => unreachable!(),
            }
            let inspector = exact_empty_inspector();
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.released, 0, "case {case}");
            assert_eq!(report.collected, 0, "case {case}");
            assert!(launch_path.exists(), "case {case}");
            assert!(lease_path.exists(), "case {case}");
            assert_eq!(inspector.direct_calls.get(), 0, "case {case}");
            assert_eq!(inspector.systemd_calls.get(), 0, "case {case}");
            assert_eq!(controller.direct_calls.get(), 0, "case {case}");
            assert_eq!(controller.unadopted_systemd_calls.get(), 0, "case {case}");
            assert_eq!(controller.adopted_systemd_calls.get(), 0, "case {case}");
        }
    }

    #[test]
    fn preparing_absent_lease_aborts_then_collects_without_release() {
        for mode in [OwnershipMode::Direct, OwnershipMode::Systemd] {
            let fixture = TransferFixture::new("77777777777777777777777777777777", mode);
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            fs::remove_file(&lease_path).unwrap();
            let inspector = exact_empty_inspector();
            let controller = exact_empty_controller();

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.terminalized, 1);
            assert_eq!(report.released, 0);
            assert_eq!(report.collected, 1);
            assert_eq!(report.retained, 0);
            assert!(!launch_path.exists());
            assert!(!lease_path.exists());
            assert_eq!(
                controller.direct_calls.get(),
                usize::from(mode == OwnershipMode::Direct)
            );
            assert_eq!(
                controller.unadopted_systemd_calls.get(),
                usize::from(mode == OwnershipMode::Systemd)
            );
        }
    }

    #[test]
    fn direct_preexec_abort_crash_with_absent_process_lease_collects_on_retry() {
        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();
        let mut filesystem = FaultingReconciliationFilesystem {
            fail_at: ReconciliationCheckpoint::BeforeRecordRemoval,
        };

        assert!(fixture
            .registry
            .reconcile_with_filesystem(&inspector, Some(&controller), &mut filesystem)
            .is_err());
        let terminal = LaunchRecord::parse(&fs::read(&launch_path).unwrap()).unwrap();
        assert_eq!(terminal.state, LaunchState::Terminal);
        assert_eq!(terminal.result, LaunchResult::Failed);
        assert_eq!(terminal.lease_kind, LeaseKind::Process);
        assert!(!terminal.exec_open);
        assert!(!terminal.direct_drained);
        assert!(!lease_path.exists());
        let prior_controller_calls = controller.direct_calls.get();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.collected, 1);
        assert_eq!(report.released, 0);
        assert_eq!(report.retained, 0);
        assert!(!launch_path.exists());
        assert_eq!(controller.direct_calls.get(), prior_controller_calls + 1);
        assert_eq!(inspector.direct_calls.get(), 0);
    }

    #[test]
    fn exact_running_direct_detachment_writes_terminal_lost_and_retains_lease() {
        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let mut inspector = exact_empty_inspector();
        inspector.direct = DirectObservation::Detached;

        let report = fixture.registry.reconcile(&inspector, None).unwrap();

        assert_eq!(report.terminalized, 1);
        assert_eq!(report.released, 0);
        assert_eq!(report.collected, 0);
        assert_eq!(report.retained, 1);
        let terminal = LaunchRecord::parse(&fs::read(&launch_path).unwrap()).unwrap();
        assert_eq!(terminal.state, LaunchState::Terminal);
        assert_eq!(terminal.result, LaunchResult::Lost);
        assert!(!terminal.direct_drained);
        assert!(lease_path.exists());
    }

    #[test]
    fn empty_registry_with_malformed_transfer_staging_returns_frozen_error() {
        let generated = tempfile::tempdir().unwrap();
        fs::set_permissions(generated.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = GenerationStore::open(generated.path()).unwrap();
        let state = tempfile::tempdir().unwrap();
        let registry = ActivationRegistry::open(
            state.path(),
            GenerationLeaseCapability::from_store(&store).unwrap(),
        )
        .unwrap();
        let staging = generated
            .path()
            .join("leases/.lease-transfer-88888888888888888888888888888888");
        write_mode(&staging, b"malformed retained evidence\n", 0o600);
        let inspector = exact_empty_inspector();

        let error = registry.reconcile(&inspector, None).unwrap_err();

        assert!(error.contains("transfer staging"));
        assert!(staging.exists());
    }

    #[test]
    fn one_mismatched_lease_freezes_all_destructive_reconciliation() {
        let fixture = TransferFixture::direct();
        let first_launch = fixture.launch_path();
        let first_lease = fixture.lease_path();
        let second_selection = fixture
            .store
            .select_current_for_process(std::process::id())
            .unwrap();
        let second_lease = fixture
            ._generated
            .path()
            .join("leases")
            .join(&second_selection.lease_name);
        let second = fixture
            .registry
            .prepare(
                &fixture.session,
                PrepareLaunch {
                    launch: LaunchId::parse("66666666666666666666666666666666").unwrap(),
                    mode: OwnershipMode::Direct,
                },
                &second_selection,
            )
            .unwrap();
        let second_launch = fixture
            ._state
            .path()
            .join("helm/activation/launches")
            .join(second.record.launch.encode());
        let mut mismatched = second_selection.process_identity.clone();
        mismatched.start_time += 1;
        write_mode(&second_lease, &mismatched.encode(), 0o600);
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.retained, 2);
        assert_eq!(report.released, 0);
        assert!(first_launch.exists());
        assert!(first_lease.exists());
        assert!(second_launch.exists());
        assert!(second_lease.exists());
        assert_eq!(controller.direct_calls.get(), 0);
    }

    #[test]
    fn unreferenced_lease_inventory_uncertainty_freezes_all_valid_rows() {
        for case in ["orphan-lifecycle", "unknown", "non-utf8", "malformed"] {
            let fixture = TransferFixture::systemd();
            let first_launch = fixture.launch_path();
            let first_lease = fixture.lease_path();
            let (_, second_launch, second_lease) =
                add_systemd_running(&fixture, "99999999999999999999999999999999");
            let evidence = fixture.systemd_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            authorize_and_run(&fixture.registry, &adopted);
            let leases = fixture._generated.path().join("leases");
            let uncertain = match case {
                "orphan-lifecycle" => {
                    let path = leases.join("88888888888888888888888888888888");
                    write_mode(&path, &fs::read(&first_lease).unwrap(), 0o600);
                    path
                }
                "unknown" => {
                    let path = leases.join("unknown-retained-evidence");
                    write_mode(&path, b"unknown\n", 0o600);
                    path
                }
                "non-utf8" => {
                    let path = leases.join(OsString::from_vec(vec![0xff, 0xfe]));
                    write_mode(&path, b"unknown\n", 0o600);
                    path
                }
                "malformed" => {
                    let path = leases.join("88888888888888888888888888888888");
                    write_mode(&path, b"malformed canonical lease\n", 0o600);
                    path
                }
                _ => unreachable!(),
            };
            let inspector = exact_empty_inspector();

            let report = fixture.registry.reconcile(&inspector, None).unwrap();

            assert_eq!(report.retained, 2, "case {case}");
            assert_eq!(report.released, 0, "case {case}");
            assert_eq!(report.collected, 0, "case {case}");
            assert!(first_launch.exists(), "case {case}");
            assert!(first_lease.exists(), "case {case}");
            assert!(second_launch.exists(), "case {case}");
            assert!(second_lease.exists(), "case {case}");
            assert!(uncertain.exists(), "case {case}");
            assert_eq!(inspector.systemd_calls.get(), 0, "case {case}");
        }
    }

    #[test]
    fn retired_lease_with_nonterminal_record_freezes_unrelated_collectible_state() {
        let fixture = TransferFixture::direct();
        let first_launch = fixture.launch_path();
        let first_lease = fixture.lease_path();
        let (_, second_launch, second_lease) =
            add_systemd_running(&fixture, "99999999999999999999999999999999");
        let retired = first_lease
            .parent()
            .unwrap()
            .join(format!(".lease-retire-{}", fixture.prepared.record.lease));
        fs::rename(&first_lease, &retired).unwrap();
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.retained, 2);
        assert!(first_launch.exists());
        assert!(retired.exists());
        assert!(second_launch.exists());
        assert!(second_lease.exists());
        assert_eq!(inspector.systemd_calls.get(), 0);
        assert_eq!(controller.direct_calls.get(), 0);

        let fixture = TransferFixture::systemd();
        let first_launch = fixture.launch_path();
        let first_lease = fixture.lease_path();
        let (_, second_launch, second_lease) =
            add_systemd_running(&fixture, "99999999999999999999999999999999");
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let retired = first_lease
            .parent()
            .unwrap()
            .join(format!(".lease-retire-{}", adopted.record.lease));
        fs::rename(&first_lease, &retired).unwrap();
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.retained, 2);
        assert!(first_launch.exists());
        assert!(retired.exists());
        assert!(second_launch.exists());
        assert!(second_lease.exists());
        assert_eq!(inspector.systemd_calls.get(), 0);
        assert_eq!(controller.adopted_systemd_calls.get(), 0);
    }

    #[test]
    fn retired_launch_with_present_lease_freezes_unrelated_collectible_state() {
        let fixture = TransferFixture::direct();
        let first_launch = fixture.launch_path();
        let first_lease = fixture.lease_path();
        let (_, second_launch, second_lease) =
            add_systemd_running(&fixture, "99999999999999999999999999999999");
        let mut terminal = fixture.prepared.record.clone();
        terminal.sequence += 1;
        terminal.state = LaunchState::Terminal;
        terminal.result = LaunchResult::Failed;
        fixture
            .registry
            .replace_launch_record(&fixture.prepared.record, &terminal)
            .unwrap();
        let retired = first_launch
            .parent()
            .unwrap()
            .join(format!(".launch-retire-{}", terminal.launch.encode()));
        fs::rename(&first_launch, &retired).unwrap();
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.retained, 2);
        assert!(retired.exists());
        assert!(first_lease.exists());
        assert!(second_launch.exists());
        assert!(second_lease.exists());
        assert_eq!(inspector.systemd_calls.get(), 0);
        assert_eq!(controller.direct_calls.get(), 0);

        let fixture = TransferFixture::systemd();
        let first_launch = fixture.launch_path();
        let first_lease = fixture.lease_path();
        let (_, second_launch, second_lease) =
            add_systemd_running(&fixture, "99999999999999999999999999999999");
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let running = authorize_and_run(&fixture.registry, &adopted);
        let mut terminal = running.clone();
        terminal.sequence += 1;
        terminal.state = LaunchState::Terminal;
        terminal.result = LaunchResult::Exited;
        fixture
            .registry
            .replace_launch_record(&running, &terminal)
            .unwrap();
        let retired = first_launch
            .parent()
            .unwrap()
            .join(format!(".launch-retire-{}", terminal.launch.encode()));
        fs::rename(&first_launch, &retired).unwrap();
        let inspector = exact_empty_inspector();

        let report = fixture.registry.reconcile(&inspector, None).unwrap();

        assert_eq!(report.retained, 2);
        assert!(retired.exists());
        assert!(first_lease.exists());
        assert!(second_launch.exists());
        assert!(second_lease.exists());
        assert_eq!(inspector.systemd_calls.get(), 0);
    }

    #[test]
    fn duplicate_launch_references_to_one_process_lease_freeze_before_controller() {
        let fixture = TransferFixture::direct();
        let first_launch = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let mut duplicate = fixture.prepared.record.clone();
        duplicate.launch = LaunchId::parse("99999999999999999999999999999999").unwrap();
        let second_launch = first_launch
            .parent()
            .unwrap()
            .join(duplicate.launch.encode());
        write_mode(&second_launch, &duplicate.encode(), 0o600);
        let inspector = exact_empty_inspector();
        let controller = exact_empty_controller();

        let report = fixture
            .registry
            .reconcile(&inspector, Some(&controller))
            .unwrap();

        assert_eq!(report.retained, 2);
        assert_eq!(report.terminalized, 0);
        assert_eq!(report.released, 0);
        assert_eq!(report.collected, 0);
        assert!(first_launch.exists());
        assert!(second_launch.exists());
        assert!(lease_path.exists());
        assert_eq!(controller.direct_calls.get(), 0);
    }

    #[test]
    fn prepare_rejects_reusing_a_selection_lease_already_in_a_launch_record() {
        let fixture = TransferFixture::direct();
        let second_launch = LaunchId::parse("99999999999999999999999999999999").unwrap();

        let error = fixture
            .registry
            .prepare(
                &fixture.session,
                PrepareLaunch {
                    launch: second_launch,
                    mode: OwnershipMode::Direct,
                },
                &fixture.selection,
            )
            .unwrap_err();

        assert!(error.contains("lease is already referenced"));
        assert!(!fixture
            ._state
            .path()
            .join("helm/activation/launches")
            .join(second_launch.encode())
            .exists());
        assert!(fixture.launch_path().exists());
        assert!(fixture.lease_path().exists());
    }

    #[test]
    fn prepare_rejects_a_launch_id_reserved_by_a_retirement_record() {
        let fixture = TransferFixture::direct();
        let launch = fixture.prepared.record.launch;
        let launch_path = fixture.launch_path();
        let retirement = launch_path
            .parent()
            .unwrap()
            .join(format!(".launch-retire-{}", launch.encode()));
        let mut terminal = fixture.prepared.record.clone();
        terminal.sequence += 1;
        terminal.state = LaunchState::Terminal;
        terminal.result = LaunchResult::Failed;
        fixture
            .registry
            .replace_launch_record(&fixture.prepared.record, &terminal)
            .unwrap();
        fs::rename(&launch_path, &retirement).unwrap();
        fs::remove_file(fixture.lease_path()).unwrap();
        let selection = fixture
            .store
            .select_current_for_process(std::process::id())
            .unwrap();

        let result = fixture.registry.prepare(
            &fixture.session,
            PrepareLaunch {
                launch,
                mode: OwnershipMode::Direct,
            },
            &selection,
        );

        assert!(result.is_err());
        assert!(retirement.exists());
        assert!(!launch_path.exists());
        assert!(fs::read_dir(launch_path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".launch-create-")));
    }

    #[test]
    fn later_passive_uncertainty_freezes_all_rows_in_both_enumeration_orders() {
        for uncertain_id in [
            "00000000000000000000000000000000",
            "ffffffffffffffffffffffffffffffff",
        ] {
            let fixture = TransferFixture::systemd();
            let first_launch = fixture.launch_path();
            let first_lease = fixture.lease_path();
            let (uncertain_launch, second_launch, second_lease) =
                add_systemd_running(&fixture, uncertain_id);
            let evidence = fixture.systemd_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            authorize_and_run(&fixture.registry, &adopted);
            let inspector = SelectiveSystemdInspector {
                uncertain_launch,
                systemd_calls: Cell::new(0),
            };

            let report = fixture.registry.reconcile(&inspector, None).unwrap();

            assert_eq!(report.retained, 2, "uncertain {uncertain_id}");
            assert_eq!(report.terminalized, 0, "uncertain {uncertain_id}");
            assert_eq!(report.released, 0, "uncertain {uncertain_id}");
            assert_eq!(report.collected, 0, "uncertain {uncertain_id}");
            assert!(first_launch.exists(), "uncertain {uncertain_id}");
            assert!(first_lease.exists(), "uncertain {uncertain_id}");
            assert!(second_launch.exists(), "uncertain {uncertain_id}");
            assert!(second_lease.exists(), "uncertain {uncertain_id}");
            assert_eq!(inspector.systemd_calls.get(), 2, "uncertain {uncertain_id}");
        }
    }

    #[test]
    fn absent_running_lease_fatal_freezes_unrelated_valid_row_in_both_orders() {
        for absent_id in [
            "00000000000000000000000000000000",
            "ffffffffffffffffffffffffffffffff",
        ] {
            let fixture = TransferFixture::systemd();
            let first_launch = fixture.launch_path();
            let first_lease = fixture.lease_path();
            let (_, second_launch, second_lease) = add_systemd_running(&fixture, absent_id);
            let evidence = fixture.systemd_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            authorize_and_run(&fixture.registry, &adopted);
            fs::remove_file(&second_lease).unwrap();
            let inspector = exact_empty_inspector();

            let report = fixture.registry.reconcile(&inspector, None).unwrap();

            assert_eq!(report.retained, 2, "absent {absent_id}");
            assert_eq!(report.terminalized, 0, "absent {absent_id}");
            assert_eq!(report.released, 0, "absent {absent_id}");
            assert_eq!(report.collected, 0, "absent {absent_id}");
            assert!(first_launch.exists(), "absent {absent_id}");
            assert!(first_lease.exists(), "absent {absent_id}");
            assert!(second_launch.exists(), "absent {absent_id}");
            assert!(!second_lease.exists(), "absent {absent_id}");
            assert_eq!(inspector.systemd_calls.get(), 0, "absent {absent_id}");
        }
    }

    #[test]
    fn later_controller_uncertainty_allows_abort_attempts_but_zero_durable_mutation() {
        for uncertain_id in [
            "00000000000000000000000000000000",
            "ffffffffffffffffffffffffffffffff",
        ] {
            let fixture = TransferFixture::direct();
            let first_launch = fixture.launch_path();
            let first_lease = fixture.lease_path();
            let (uncertain_launch, second_launch, second_lease, _second_selection) =
                add_direct_preparing(&fixture, uncertain_id);
            let inspector = exact_empty_inspector();
            let controller = SelectiveDirectController {
                uncertain_launch,
                direct_calls: Cell::new(0),
            };

            let report = fixture
                .registry
                .reconcile(&inspector, Some(&controller))
                .unwrap();

            assert_eq!(report.retained, 2, "uncertain {uncertain_id}");
            assert_eq!(report.terminalized, 0, "uncertain {uncertain_id}");
            assert_eq!(report.released, 0, "uncertain {uncertain_id}");
            assert_eq!(report.collected, 0, "uncertain {uncertain_id}");
            assert!(first_launch.exists(), "uncertain {uncertain_id}");
            assert!(first_lease.exists(), "uncertain {uncertain_id}");
            assert!(second_launch.exists(), "uncertain {uncertain_id}");
            assert!(second_lease.exists(), "uncertain {uncertain_id}");
            assert_eq!(controller.direct_calls.get(), 2, "uncertain {uncertain_id}");
        }
    }

    #[test]
    fn unavailable_or_uncertain_gate_closed_controller_retains_preparing_state() {
        for unavailable in [true, false] {
            let fixture = TransferFixture::direct();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let inspector = exact_empty_inspector();
            let mut controller = exact_empty_controller();
            controller.direct = DirectObservation::Uncertain;
            let controller_ref = if unavailable {
                None
            } else {
                Some(&controller as &dyn GateClosedController)
            };

            let report = fixture
                .registry
                .reconcile(&inspector, controller_ref)
                .unwrap();

            assert_eq!(report.released, 0);
            assert_eq!(report.collected, 0);
            assert!(launch_path.exists());
            assert!(lease_path.exists());
            assert_eq!(controller.direct_calls.get(), usize::from(!unavailable));
        }
    }

    #[test]
    fn terminal_release_crash_boundaries_reopen_into_exact_retry_row() {
        for fail_at in [
            ReconciliationCheckpoint::AfterTerminalRecordFsync,
            ReconciliationCheckpoint::AfterLeaseUnlink,
            ReconciliationCheckpoint::AfterLeaseDirectoryFsync,
            ReconciliationCheckpoint::BeforeRecordRemoval,
        ] {
            let fixture = TransferFixture::systemd();
            let launch_path = fixture.launch_path();
            let lease_path = fixture.lease_path();
            let evidence = fixture.systemd_evidence();
            let adopted = fixture
                .registry
                .adopt_prepared(
                    &fixture.session,
                    fixture.prepared,
                    fixture.selection,
                    evidence,
                )
                .unwrap();
            authorize_and_run(&fixture.registry, &adopted);
            let inspector = exact_empty_inspector();
            let mut filesystem = FaultingReconciliationFilesystem { fail_at };

            assert!(fixture
                .registry
                .reconcile_with_filesystem(&inspector, None, &mut filesystem)
                .is_err());
            let durable = LaunchRecord::parse(&fs::read(&launch_path).unwrap()).unwrap();
            assert_eq!(durable.state, LaunchState::Terminal);
            assert_eq!(durable.result, LaunchResult::Exited);
            assert_eq!(
                lease_path.exists(),
                fail_at == ReconciliationCheckpoint::AfterTerminalRecordFsync
            );

            let report = fixture.registry.reconcile(&inspector, None).unwrap();
            assert_eq!(report.terminalized, 0);
            assert_eq!(
                report.released,
                usize::from(fail_at == ReconciliationCheckpoint::AfterTerminalRecordFsync)
            );
            assert_eq!(report.collected, 1);
            assert!(!launch_path.exists());
            assert!(!lease_path.exists());
        }
    }

    #[test]
    fn retirement_only_crash_states_reopen_into_the_same_exact_proof_row() {
        let fixture = TransferFixture::systemd();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let retired_lease = lease_path
            .parent()
            .unwrap()
            .join(format!(".lease-retire-{}", fixture.prepared.record.lease));
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let inspector = exact_empty_inspector();
        let mut filesystem = FaultingReconciliationFilesystem {
            fail_at: ReconciliationCheckpoint::AfterLeaseRetire,
        };

        assert!(fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut filesystem)
            .is_err());
        assert!(launch_path.exists());
        assert!(!lease_path.exists());
        assert!(retired_lease.exists());
        let report = fixture.registry.reconcile(&inspector, None).unwrap();
        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 1);
        assert!(!launch_path.exists());
        assert!(!retired_lease.exists());

        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let retired_launch = launch_path.parent().unwrap().join(format!(
            ".launch-retire-{}",
            fixture.prepared.record.launch.encode()
        ));
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let running = authorize_and_run(&fixture.registry, &adopted);
        terminal_direct(&fixture.registry, &running);
        let inspector = exact_empty_inspector();
        let mut filesystem = FaultingReconciliationFilesystem {
            fail_at: ReconciliationCheckpoint::AfterLaunchRetire,
        };

        assert!(fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut filesystem)
            .is_err());
        assert!(!launch_path.exists());
        assert!(!lease_path.exists());
        assert!(retired_launch.exists());
        let report = fixture.registry.reconcile(&inspector, None).unwrap();
        assert_eq!(report.released, 0);
        assert_eq!(report.collected, 1);
        assert!(!retired_launch.exists());
    }

    #[test]
    fn reconciliation_never_unlinks_same_byte_replacement_inodes() {
        let fixture = TransferFixture::systemd();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let displaced_lease = fixture._generated.path().join("displaced-reconcile-lease");
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let inspector = exact_empty_inspector();
        let mut lease_swap = SwappingReconciliationFilesystem {
            swap_at: ReconciliationCheckpoint::AfterTerminalRecordFsync,
            path: lease_path.clone(),
            displaced: displaced_lease.clone(),
        };

        let report = fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut lease_swap)
            .unwrap();
        assert_eq!(report.terminalized, 1);
        assert_eq!(report.released, 0);
        assert_eq!(report.collected, 0);
        assert_eq!(report.retained, 1);
        assert!(launch_path.exists());
        assert!(lease_path.exists());
        assert!(displaced_lease.exists());

        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let displaced_launch = fixture._state.path().join("displaced-reconcile-launch");
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let running = authorize_and_run(&fixture.registry, &adopted);
        terminal_direct(&fixture.registry, &running);
        let inspector = exact_empty_inspector();
        let mut launch_swap = SwappingReconciliationFilesystem {
            swap_at: ReconciliationCheckpoint::BeforeRecordRemoval,
            path: launch_path.clone(),
            displaced: displaced_launch.clone(),
        };

        let report = fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut launch_swap)
            .unwrap();
        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 0);
        assert_eq!(report.retained, 1);
        assert!(launch_path.exists());
        assert!(!lease_path.exists());
        assert!(displaced_launch.exists());
        assert_eq!(
            fixture
                .registry
                .reconcile(&inspector, None)
                .unwrap()
                .collected,
            1
        );
        assert!(!launch_path.exists());
    }

    #[test]
    fn exact_final_gap_swaps_are_retained_then_retired_only_after_fresh_proof() {
        let fixture = TransferFixture::systemd();
        let launch_path = fixture.launch_path();
        let lease_path = fixture.lease_path();
        let lease_name = fixture.prepared.record.lease.clone();
        let retired_lease = lease_path
            .parent()
            .unwrap()
            .join(format!(".lease-retire-{lease_name}"));
        let displaced_lease = fixture._generated.path().join("held-original-lease");
        let evidence = fixture.systemd_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        authorize_and_run(&fixture.registry, &adopted);
        let inspector = exact_empty_inspector();
        let mut lease_swap = SwappingReconciliationFilesystem {
            swap_at: ReconciliationCheckpoint::BeforeLeaseRetire,
            path: lease_path.clone(),
            displaced: displaced_lease.clone(),
        };

        let report = fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut lease_swap)
            .unwrap();

        assert_eq!(report.released, 0);
        assert_eq!(report.collected, 0);
        assert_eq!(report.retained, 1);
        assert!(launch_path.exists());
        assert!(!lease_path.exists());
        assert!(retired_lease.exists());
        assert!(displaced_lease.exists());
        let fresh_report = fixture.registry.reconcile(&inspector, None).unwrap();
        assert_eq!(fresh_report.released, 1);
        assert_eq!(fresh_report.collected, 1);
        assert!(!launch_path.exists());
        assert!(!retired_lease.exists());
        assert!(displaced_lease.exists());

        let fixture = TransferFixture::direct();
        let launch_path = fixture.launch_path();
        let launch_name = fixture.prepared.record.launch.encode();
        let retired_launch = launch_path
            .parent()
            .unwrap()
            .join(format!(".launch-retire-{launch_name}"));
        let displaced_launch = fixture._state.path().join("held-original-launch");
        let evidence = fixture.direct_evidence();
        let adopted = fixture
            .registry
            .adopt_prepared(
                &fixture.session,
                fixture.prepared,
                fixture.selection,
                evidence,
            )
            .unwrap();
        let running = authorize_and_run(&fixture.registry, &adopted);
        terminal_direct(&fixture.registry, &running);
        let inspector = exact_empty_inspector();
        let mut launch_swap = SwappingReconciliationFilesystem {
            swap_at: ReconciliationCheckpoint::BeforeLaunchRetire,
            path: launch_path.clone(),
            displaced: displaced_launch.clone(),
        };

        let report = fixture
            .registry
            .reconcile_with_filesystem(&inspector, None, &mut launch_swap)
            .unwrap();

        assert_eq!(report.released, 1);
        assert_eq!(report.collected, 0);
        assert_eq!(report.retained, 1);
        assert!(!launch_path.exists());
        assert!(retired_launch.exists());
        assert!(displaced_launch.exists());
        let fresh_report = fixture.registry.reconcile(&inspector, None).unwrap();
        assert_eq!(fresh_report.released, 0);
        assert_eq!(fresh_report.collected, 1);
        assert!(!retired_launch.exists());
        assert!(displaced_launch.exists());
    }
}
