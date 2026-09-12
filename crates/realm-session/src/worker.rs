//! Bounded filesystem and process worker for the session daemon.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustix::event::{eventfd, EventfdFlags};
use rustix::io::Errno;

use crate::backend::{
    WorkerCapacityError, WorkerCapacityResource, MAX_SNAPSHOT_BYTES, MAX_WORKER_JOBS,
    MAX_WORKER_RESULTS, WORKER_SHUTDOWN_TIMEOUT_MS,
};
use crate::session::SessionSnapshotV1;

/// Process work executed away from the compositor event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessJob {
    /// Spawn one argv vector without a shell.
    Spawn(Vec<String>),
}

#[derive(Debug)]
struct SnapshotJob {
    sequence: u64,
    snapshot: SessionSnapshotV1,
}

#[derive(Debug)]
enum WorkerMessage {
    ReadSnapshot,
    Process(ProcessJob),
    SnapshotAvailable,
    Seal,
}

/// One terminal result returned by the worker.
#[derive(Debug)]
pub enum WorkerResult {
    /// Initial bounded pathname read completed.
    SnapshotRead(io::Result<Vec<u8>>),
    /// The worker is ready to take the latest replaceable snapshot.
    SnapshotRequested,
    /// One process request reached its spawn boundary.
    Process {
        /// Spawn result.
        result: io::Result<()>,
    },
    /// One immutable persistence sequence reached a terminal write result.
    SnapshotWritten {
        /// Persistence sequence.
        sequence: u64,
        /// Atomic write result.
        result: io::Result<()>,
    },
    /// Every message preceding the seal reached a terminal result.
    Fence,
}

/// Non-sliding state of the clean-shutdown worker fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceStatus {
    /// The worker has not yet acknowledged the seal.
    Pending,
    /// Every pre-seal operation produced a terminal result.
    Acknowledged,
    /// The hard worker deadline elapsed before acknowledgement.
    TimedOut,
}

/// A nonblocking reservation made before acknowledging process effects.
#[derive(Debug)]
pub struct WorkerReservation {
    count: usize,
}

/// Invalid worker use after admission or seal.
#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    /// The worker thread or one of its bounded channels disappeared.
    #[error("worker channel disconnected")]
    Disconnected,
    /// Commit did not provide exactly the number of reserved jobs.
    #[error("worker reservation expected {reserved} jobs but received {provided}")]
    ReservationMismatch {
        /// Reserved count.
        reserved: usize,
        /// Submitted count.
        provided: usize,
    },
    /// No work may be admitted after the terminal seal.
    #[error("worker is sealed")]
    Sealed,
}

/// Single-owner handle for the bounded worker thread and its result eventfd.
pub struct Worker {
    messages: SyncSender<WorkerMessage>,
    snapshot_reply: SyncSender<Option<SnapshotJob>>,
    results: Receiver<WorkerResult>,
    event_fd: Arc<OwnedFd>,
    snapshot_path: PathBuf,
    pending_snapshot: Option<SnapshotJob>,
    outstanding_process: usize,
    reserved_process: usize,
    sealed_at: Option<Instant>,
    fence_acknowledged: bool,
    pending_result_notifications: u64,
}

impl Worker {
    /// Start the sole filesystem/process worker for one daemon incarnation.
    pub fn start(snapshot_path: PathBuf) -> io::Result<Self> {
        let event_fd = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
        let (message_tx, message_rx) = mpsc::sync_channel(MAX_WORKER_JOBS + 2);
        let (snapshot_tx, snapshot_rx) = mpsc::sync_channel(1);
        let (result_tx, result_rx) = mpsc::sync_channel(MAX_WORKER_RESULTS);
        let thread_event_fd = event_fd.clone();
        let thread_path = snapshot_path.clone();
        std::thread::Builder::new()
            .name("realm-worker".to_owned())
            .spawn(move || {
                worker_main(
                    thread_path,
                    message_rx,
                    snapshot_rx,
                    result_tx,
                    thread_event_fd,
                );
            })?;
        Ok(Self {
            messages: message_tx,
            snapshot_reply: snapshot_tx,
            results: result_rx,
            event_fd,
            snapshot_path,
            pending_snapshot: None,
            outstanding_process: 0,
            reserved_process: 0,
            sealed_at: None,
            fence_acknowledged: false,
            pending_result_notifications: 0,
        })
    }

    /// Path used for initial reads and later atomic writes.
    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    /// Borrow the result notification descriptor for the combined poll set.
    pub fn event_fd(&self) -> BorrowedFd<'_> {
        self.event_fd.as_fd()
    }

    /// Request the exactly-once startup snapshot read.
    pub fn request_snapshot_read(&self) -> Result<(), WorkerError> {
        if self.sealed_at.is_some() {
            return Err(WorkerError::Sealed);
        }
        self.messages
            .try_send(WorkerMessage::ReadSnapshot)
            .map_err(|_| WorkerError::Disconnected)
    }

    /// Reserve process capacity without exposing work to the worker.
    pub fn reserve_process_jobs(
        &mut self,
        count: usize,
    ) -> Result<WorkerReservation, WorkerCapacityError> {
        let admitted = self
            .outstanding_process
            .saturating_add(self.reserved_process)
            .saturating_add(count);
        if self.sealed_at.is_some() || admitted > MAX_WORKER_JOBS {
            return Err(WorkerCapacityError {
                resource: WorkerCapacityResource::Jobs,
                limit: MAX_WORKER_JOBS,
            });
        }
        self.reserved_process += count;
        Ok(WorkerReservation { count })
    }

    /// Cancel an unused process reservation.
    pub fn cancel(&mut self, reservation: WorkerReservation) {
        self.reserved_process = self
            .reserved_process
            .checked_sub(reservation.count)
            .expect("reservation belongs to this single-owner worker");
    }

    /// Commit exactly the jobs covered by a prior reservation.
    pub fn commit(
        &mut self,
        reservation: WorkerReservation,
        jobs: Vec<ProcessJob>,
    ) -> Result<(), WorkerError> {
        if self.sealed_at.is_some() {
            return Err(WorkerError::Sealed);
        }
        if jobs.len() != reservation.count {
            return Err(WorkerError::ReservationMismatch {
                reserved: reservation.count,
                provided: jobs.len(),
            });
        }
        self.reserved_process = self
            .reserved_process
            .checked_sub(reservation.count)
            .expect("reservation belongs to this single-owner worker");
        for job in jobs {
            self.messages
                .try_send(WorkerMessage::Process(job))
                .map_err(|_| WorkerError::Disconnected)?;
            self.outstanding_process += 1;
        }
        Ok(())
    }

    /// Replace the not-yet-started snapshot slot with the newest value.
    pub fn submit_snapshot(
        &mut self,
        sequence: u64,
        snapshot: SessionSnapshotV1,
    ) -> Result<(), WorkerError> {
        if self.sealed_at.is_some() {
            return Err(WorkerError::Sealed);
        }
        if self.pending_snapshot.is_none() {
            self.messages
                .try_send(WorkerMessage::SnapshotAvailable)
                .map_err(|_| WorkerError::Disconnected)?;
        }
        self.pending_snapshot = Some(SnapshotJob { sequence, snapshot });
        Ok(())
    }

    /// Answer one worker request with the current replaceable snapshot value.
    pub fn answer_snapshot_request(&mut self) -> Result<(), WorkerError> {
        self.snapshot_reply
            .try_send(self.pending_snapshot.take())
            .map_err(|_| WorkerError::Disconnected)
    }

    /// Seal all worker admission and establish the hard non-sliding deadline.
    pub fn seal(&mut self, now: Instant) -> Result<(), WorkerError> {
        if self.sealed_at.is_some() {
            return Ok(());
        }
        self.sealed_at = Some(now);
        self.messages
            .try_send(WorkerMessage::Seal)
            .map_err(|_| WorkerError::Disconnected)
    }

    /// Read at most one terminal result without waiting.
    pub fn try_result(&mut self) -> Result<Option<WorkerResult>, WorkerError> {
        if self.pending_result_notifications == 0 {
            self.pending_result_notifications = drain_eventfd(self.event_fd.as_fd())?;
        }
        let result = match self.results.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => return Err(WorkerError::Disconnected),
        };
        self.pending_result_notifications = self.pending_result_notifications.saturating_sub(1);
        match &result {
            WorkerResult::Process { .. } => {
                self.outstanding_process = self
                    .outstanding_process
                    .checked_sub(1)
                    .expect("each process job has one terminal result");
            }
            WorkerResult::Fence => self.fence_acknowledged = true,
            _ => {}
        }
        Ok(Some(result))
    }

    /// True while more already-queued results require immediate bounded service.
    pub fn has_results(&self) -> bool {
        self.pending_result_notifications > 0
    }

    /// Current terminal fence state at the captured loop time.
    pub fn fence_status(&self, now: Instant) -> FenceStatus {
        if self.fence_acknowledged {
            return FenceStatus::Acknowledged;
        }
        let Some(sealed_at) = self.sealed_at else {
            return FenceStatus::Pending;
        };
        if now >= sealed_at + Duration::from_millis(WORKER_SHUTDOWN_TIMEOUT_MS) {
            FenceStatus::TimedOut
        } else {
            FenceStatus::Pending
        }
    }
}

fn worker_main(
    snapshot_path: PathBuf,
    messages: Receiver<WorkerMessage>,
    snapshots: Receiver<Option<SnapshotJob>>,
    results: SyncSender<WorkerResult>,
    event_fd: Arc<OwnedFd>,
) {
    while let Ok(message) = messages.recv() {
        let sealing = matches!(message, WorkerMessage::Seal);
        let result = match message {
            WorkerMessage::ReadSnapshot => {
                WorkerResult::SnapshotRead(read_snapshot_bounded(&snapshot_path))
            }
            WorkerMessage::Process(ProcessJob::Spawn(argv)) => WorkerResult::Process {
                result: spawn_argv(argv),
            },
            WorkerMessage::SnapshotAvailable => {
                if send_result(&results, &event_fd, WorkerResult::SnapshotRequested).is_err() {
                    return;
                }
                let Ok(snapshot) = snapshots.recv() else {
                    return;
                };
                let Some(snapshot) = snapshot else {
                    continue;
                };
                WorkerResult::SnapshotWritten {
                    sequence: snapshot.sequence,
                    result: write_snapshot_atomically(&snapshot_path, &snapshot.snapshot),
                }
            }
            WorkerMessage::Seal => WorkerResult::Fence,
        };
        if send_result(&results, &event_fd, result).is_err() {
            return;
        }
        if sealing {
            return;
        }
    }
}

fn spawn_argv(argv: Vec<String>) -> io::Result<()> {
    let Some(program) = argv.first() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty argv"));
    };
    Command::new(program).args(&argv[1..]).spawn().map(|_| ())
}

fn send_result(
    results: &SyncSender<WorkerResult>,
    event_fd: &OwnedFd,
    result: WorkerResult,
) -> Result<(), ()> {
    results.send(result).map_err(|_| ())?;
    rustix::io::write(event_fd, &1_u64.to_ne_bytes())
        .map(|_| ())
        .map_err(|_| ())
}

fn drain_eventfd(fd: BorrowedFd<'_>) -> Result<u64, WorkerError> {
    let mut bytes = [0_u8; 8];
    match rustix::io::read(fd, &mut bytes) {
        Ok(8) => Ok(u64::from_ne_bytes(bytes)),
        Ok(_) => Err(WorkerError::Disconnected),
        Err(Errno::AGAIN) => Ok(0),
        Err(_) => Err(WorkerError::Disconnected),
    }
}

/// Read one snapshot without trusting its metadata for allocation sizing.
pub fn read_snapshot_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(MAX_SNAPSHOT_BYTES);
    file.take((MAX_SNAPSHOT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session snapshot exceeds the accepted byte limit",
        ));
    }
    Ok(bytes)
}

/// Replace one snapshot through a complete same-directory temporary file.
pub fn write_snapshot_atomically(path: &Path, snapshot: &SessionSnapshotV1) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "snapshot path has no parent")
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "snapshot path has no file name",
        )
    })?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(".tmp");
    let temporary = parent.join(temporary_name);
    let bytes = snapshot
        .to_json()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use realm_core::Ledger;

    use super::{
        read_snapshot_bounded, write_snapshot_atomically, FenceStatus, ProcessJob, Worker,
        WorkerResult,
    };
    use crate::backend::{
        WorkerCapacityResource, MAX_SNAPSHOT_BYTES, MAX_WORKER_JOBS, WORKER_SHUTDOWN_TIMEOUT_MS,
    };
    use crate::session::SessionSnapshotV1;

    fn fixture_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "realm-session-worker-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn snapshot_worker_reads_at_most_limit_plus_one() {
        let root = fixture_dir("bounded-read");
        let path = root.join("ledger.json");
        fs::write(&path, vec![b'x'; MAX_SNAPSHOT_BYTES]).unwrap();
        assert_eq!(
            read_snapshot_bounded(&path).unwrap().len(),
            MAX_SNAPSHOT_BYTES
        );

        fs::write(&path, vec![b'x'; MAX_SNAPSHOT_BYTES + 1]).unwrap();
        let error = read_snapshot_bounded(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_snapshot_write_replaces_the_complete_record() {
        let root = fixture_dir("atomic-write");
        let path = root.join("ledger.json");
        fs::write(&path, b"old").unwrap();
        let temporary = root.join("ledger.json.tmp");
        fs::write(&temporary, b"stale").unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644)).unwrap();
        let snapshot = SessionSnapshotV1::new(Ledger::new(), Vec::new(), 0).unwrap();

        write_snapshot_atomically(&path, &snapshot).unwrap();

        assert_eq!(fs::read(&path).unwrap(), snapshot.to_json().unwrap());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!temporary.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_process_queue_accepts_256_and_rejects_257_atomically() {
        let root = fixture_dir("job-capacity");
        let mut worker = Worker::start(root.join("ledger.json")).unwrap();
        let reservation = worker.reserve_process_jobs(MAX_WORKER_JOBS).unwrap();

        let error = worker.reserve_process_jobs(1).unwrap_err();
        assert_eq!(error.resource, WorkerCapacityResource::Jobs);
        assert_eq!(error.limit, MAX_WORKER_JOBS);

        worker.cancel(reservation);
        assert!(worker.reserve_process_jobs(1).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_spawn_reports_a_terminal_result() {
        let root = fixture_dir("spawn-result");
        let mut worker = Worker::start(root.join("ledger.json")).unwrap();
        let reservation = worker.reserve_process_jobs(1).unwrap();
        worker
            .commit(
                reservation,
                vec![ProcessJob::Spawn(vec!["/bin/true".into()])],
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(WorkerResult::Process { result }) = worker.try_result().unwrap() {
                assert!(result.is_ok());
                break;
            }
            assert!(Instant::now() < deadline, "worker did not report spawn");
            std::thread::yield_now();
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worker_seal_is_capacity_free_idempotent_and_has_a_fixed_deadline() {
        let root = fixture_dir("seal");
        let mut worker = Worker::start(root.join("ledger.json")).unwrap();
        let started = Instant::now();
        worker.seal(started).unwrap();
        worker.seal(started + Duration::from_millis(500)).unwrap();

        assert_eq!(
            worker.fence_status(started + Duration::from_millis(WORKER_SHUTDOWN_TIMEOUT_MS - 1)),
            FenceStatus::Pending
        );
        assert_eq!(
            worker.fence_status(started + Duration::from_millis(WORKER_SHUTDOWN_TIMEOUT_MS)),
            FenceStatus::TimedOut
        );
        assert!(worker.reserve_process_jobs(1).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
