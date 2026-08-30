use super::{
    canonical_boot_id, canonical_positive_u64, canonical_u32, canonical_u64, linux_boot_id,
    linux_process_identity, lower_hex_32, normalized_cgroup_path, open_directory_chain,
    record_value, validate_owned_mode, GenerationId, GenerationRoot, GenerationSelection,
    ParsedLeaseRecord,
};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rustix::fs::{
    flock, fsync, mkdirat, openat, renameat_with, statat, unlinkat, AtFlags, Dir, FileType,
    FlockOperation, Mode, OFlags, RenameFlags,
};
use rustix::io::Errno;

const MAX_RECORD_BYTES: usize = 4096;
const MAX_INVENTORY_ENTRIES: usize = 4096;
const MAX_INVENTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct ActivationRegistry {
    activation: GenerationRoot,
    launches: GenerationRoot,
    lock: OwnedFd,
    intra_process_lock: Mutex<()>,
    device: u64,
    inode: u64,
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
enum VerifiedOwnership {
    Direct {
        process_group: u32,
    },
    Systemd {
        unit_invocation: String,
        cgroup: String,
        cgroup_device: u64,
        cgroup_inode: u64,
    },
}

#[derive(Debug)]
struct RegistryTransfer {
    prepared: PreparedLaunch,
    ownership: VerifiedOwnership,
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

impl ActivationRegistry {
    fn open(state_home: &Path) -> Result<Self, String> {
        if state_home.as_os_str().is_empty() || !state_home.is_absolute() {
            return Err("state home must be a non-empty absolute path".into());
        }
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
        let final_name = request.launch.encode();
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

    fn ensure_capacity_for_new_record(&self, record_bytes: usize) -> Result<(), String> {
        let mut entries = 0_usize;
        let mut bytes = 0_usize;
        for name in directory_entries(&self.activation.fd)? {
            if matches!(name.as_bytes(), b"lifecycle.lock" | b"launches") {
                continue;
            }
            let stat = statat(&self.activation.fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            account_inventory(&mut entries, &mut bytes, stat.st_size)?;
        }
        for name in directory_entries(&self.launches.fd)? {
            let stat = statat(&self.launches.fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            account_inventory(&mut entries, &mut bytes, stat.st_size)?;
        }
        let entries = entries
            .checked_add(1)
            .ok_or("lifecycle inventory entry count overflow")?;
        let bytes = bytes
            .checked_add(record_bytes)
            .ok_or("lifecycle inventory byte count overflow")?;
        inventory_within_bounds(entries, bytes)
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
        let activation_entries = directory_entries(&self.activation.fd)?;
        let launch_entries = directory_entries(&self.launches.fd)?;
        let mut entries = 0_usize;
        let mut bytes = 0_usize;
        let mut session_record = None;
        let mut launch_records = Vec::new();
        let mut temporaries = Vec::new();

        for name in activation_entries {
            let raw = name.as_bytes();
            if matches!(raw, b"lifecycle.lock" | b"launches") {
                continue;
            }
            let stat = statat(&self.activation.fd, &name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|error| error.to_string())?;
            account_inventory(&mut entries, &mut bytes, stat.st_size)?;
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
            account_inventory(&mut entries, &mut bytes, stat.st_size)?;
            let raw = name.as_bytes();
            if let Ok(text) = std::str::from_utf8(raw) {
                if let Ok(id) = LaunchId::parse(text) {
                    let record = read_launch_record(&self.launches.fd, &name)?;
                    if record.launch != id {
                        return Err("launch filename and record id disagree".into());
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
        inventory_within_bounds(entries, bytes)?;

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

fn directory_entries(directory: &OwnedFd) -> Result<Vec<OsString>, String> {
    let mut entries = Vec::new();
    let mut reader = Dir::read_from(directory).map_err(|error| error.to_string())?;
    while let Some(entry) = reader.read() {
        let entry = entry.map_err(|error| error.to_string())?;
        let bytes = entry.file_name().to_bytes();
        if !matches!(bytes, b"." | b"..") {
            entries.push(OsString::from_vec(bytes.to_vec()));
        }
    }
    Ok(entries)
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
    use crate::generation::{GenerationPublication, GenerationStore};
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
        let registry = ActivationRegistry::open(root.path()).unwrap();
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
        assert!(ActivationRegistry::open(Path::new("")).is_err());
        let relative = format!("relative-activation-test-{}", std::process::id());
        assert!(ActivationRegistry::open(Path::new(&relative)).is_err());
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
                assert!(ActivationRegistry::open(root.path()).is_err());
                assert_eq!(fs::read(target).unwrap(), b"sentinel");
                assert!(fs::symlink_metadata(lock).unwrap().file_type().is_symlink());
            } else {
                write_mode(&lock, b"", 0o644);
                assert!(ActivationRegistry::open(root.path()).is_err());
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
        let registry = ActivationRegistry::open(root.path()).unwrap();
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
        let registry = ActivationRegistry::open(root.path()).unwrap();
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
        let registry = ActivationRegistry::open(root.path()).unwrap();
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
        drop(ActivationRegistry::open(root.path()).unwrap());
        let activation = root.path().join("helm/activation");
        let claim = activation.join(".session-claim-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let create = activation.join(
            "launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        write_mode(&claim, b"", 0o600);
        write_mode(&create, &[0xff, 0xfe], 0o600);
        drop(ActivationRegistry::open(root.path()).unwrap());
        assert!(!claim.exists());
        assert!(!create.exists());

        let malformed = activation.join(".session-claim-not-an-id");
        write_mode(&malformed, b"", 0o600);
        assert!(ActivationRegistry::open(root.path()).is_err());
        assert!(malformed.exists());
    }

    #[test]
    fn cross_namespace_temporaries_are_retained_and_fail_closed() {
        for activation_name in [true, false] {
            let root = tempfile::tempdir().unwrap();
            drop(ActivationRegistry::open(root.path()).unwrap());
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
            assert!(ActivationRegistry::open(root.path()).is_err());
            assert!(path.exists());
        }
    }

    #[test]
    fn inventory_retains_unsafe_temporary_and_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        drop(ActivationRegistry::open(root.path()).unwrap());
        let temporary = root.path().join(
            "helm/activation/launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        write_mode(&temporary, b"", 0o644);
        assert!(ActivationRegistry::open(root.path()).is_err());
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
        drop(ActivationRegistry::open(root.path()).unwrap());
        let temporary = root.path().join(
            "helm/activation/launches/.launch-create-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-cccccccccccccccccccccccccccccccc",
        );
        fs::create_dir(&temporary).unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(ActivationRegistry::open(root.path()).is_err());
        assert!(temporary.is_dir());
    }

    #[test]
    fn temporary_inode_replacement_before_delete_is_retained() {
        let root = tempfile::tempdir().unwrap();
        let registry = ActivationRegistry::open(root.path()).unwrap();
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
    fn over_bound_inventory_retains_every_safe_temporary() {
        let root = tempfile::tempdir().unwrap();
        drop(ActivationRegistry::open(root.path()).unwrap());
        let launches = root.path().join("helm/activation/launches");
        for index in 0..=MAX_INVENTORY_ENTRIES {
            let temporary = launches.join(format!(
                ".launch-create-{index:032x}-ffffffffffffffffffffffffffffffff"
            ));
            write_mode(&temporary, b"", 0o600);
        }
        assert!(ActivationRegistry::open(root.path()).is_err());
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
        drop(ActivationRegistry::open(state.path()).unwrap());
        write_mode(
            &state.path().join("helm/activation/session"),
            &exact_session_record("active", 7),
            0o600,
        );
        let registry = ActivationRegistry::open(state.path()).unwrap();
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
}
