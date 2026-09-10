//! Pure event-loop ownership of coalesced snapshot persistence state.

use std::time::{Duration, Instant};

use crate::session::SessionSnapshotV1;

/// Maximum delay from the first dirty observation to yielding a snapshot.
pub const SNAPSHOT_DELAY: Duration = Duration::from_millis(250);

/// Result reported for one completed persistence request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersistCompletion {
    /// The complete snapshot became the persisted record.
    Succeeded,
    /// Persistence failed without changing authoritative session state.
    Failed,
}

/// One immutable, monotonically sequenced snapshot ready for a future worker.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistRequest {
    sequence: u64,
    snapshot: SessionSnapshotV1,
}

impl PersistRequest {
    /// Monotonic sequence assigned when this request became due.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Immutable authoritative snapshot carried by this request.
    pub fn snapshot(&self) -> &SessionSnapshotV1 {
        &self.snapshot
    }
}

#[derive(Debug, Clone)]
struct DirtySnapshot {
    snapshot: SessionSnapshotV1,
    deadline: Instant,
}

/// Invalid completion reported to the single-in-flight coordinator.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unexpected persistence completion {actual}; expected {expected:?}")]
pub struct PersistCompletionError {
    expected: Option<u64>,
    actual: u64,
}

/// Pure coalescing state owned exclusively by the future event-loop thread.
#[derive(Debug)]
pub struct PersistenceCoordinator {
    persisted: Option<SessionSnapshotV1>,
    dirty: Option<DirtySnapshot>,
    in_flight: Option<PersistRequest>,
    next_sequence: u64,
}

impl PersistenceCoordinator {
    /// Start with the snapshot already known to be persisted, if any.
    pub fn new(persisted: Option<SessionSnapshotV1>) -> Self {
        Self {
            persisted,
            dirty: None,
            in_flight: None,
            next_sequence: 1,
        }
    }

    /// Observe an authoritative snapshot, retaining only the latest changed value.
    pub fn observe(&mut self, snapshot: Option<SessionSnapshotV1>, now: Instant) -> bool {
        let Some(snapshot) = snapshot else {
            return false;
        };

        if self
            .dirty
            .as_ref()
            .is_some_and(|dirty| dirty.snapshot == snapshot)
        {
            return false;
        }

        if self.in_flight.is_none()
            && self
                .persisted
                .as_ref()
                .is_some_and(|persisted| *persisted == snapshot)
        {
            return self.dirty.take().is_some();
        }

        if let Some(dirty) = &mut self.dirty {
            dirty.snapshot = snapshot;
            return true;
        }

        if self
            .in_flight
            .as_ref()
            .is_some_and(|request| request.snapshot == snapshot)
        {
            return false;
        }

        self.dirty = Some(DirtySnapshot {
            snapshot,
            deadline: now + SNAPSHOT_DELAY,
        });
        true
    }

    /// Fixed deadline for the latest dirty snapshot, if one is waiting.
    pub fn deadline(&self) -> Option<Instant> {
        if self.in_flight.is_some() {
            return None;
        }
        self.dirty.as_ref().map(|dirty| dirty.deadline)
    }

    /// Yield the latest dirty snapshot when due and no older request is in flight.
    pub fn take_due(&mut self, now: Instant) -> Option<PersistRequest> {
        if self.in_flight.is_some() {
            return None;
        }
        let dirty = self.dirty.as_ref()?;
        if now < dirty.deadline {
            return None;
        }

        let dirty = self.dirty.take().expect("dirty snapshot was present");
        let request = PersistRequest {
            sequence: self.next_sequence,
            snapshot: dirty.snapshot,
        };
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("persistence request sequence exhausted");
        self.in_flight = Some(request.clone());
        Some(request)
    }

    /// Apply completion for the sole in-flight request without clearing newer work.
    pub fn complete(
        &mut self,
        sequence: u64,
        completion: PersistCompletion,
        now: Instant,
    ) -> Result<(), PersistCompletionError> {
        let expected = self.in_flight.as_ref().map(PersistRequest::sequence);
        if expected != Some(sequence) {
            return Err(PersistCompletionError {
                expected,
                actual: sequence,
            });
        }
        let completed = self
            .in_flight
            .take()
            .expect("matching in-flight request was present");

        match completion {
            PersistCompletion::Succeeded => {
                if self
                    .dirty
                    .as_ref()
                    .is_some_and(|dirty| dirty.snapshot == completed.snapshot)
                {
                    self.dirty = None;
                }
                self.persisted = Some(completed.snapshot);
            }
            PersistCompletion::Failed => match &mut self.dirty {
                None => {
                    self.dirty = Some(DirtySnapshot {
                        snapshot: completed.snapshot,
                        deadline: now + SNAPSHOT_DELAY,
                    });
                }
                Some(dirty)
                    if self
                        .persisted
                        .as_ref()
                        .is_some_and(|persisted| *persisted == dirty.snapshot) =>
                {
                    self.dirty = None;
                }
                Some(dirty) if dirty.snapshot == completed.snapshot => {
                    dirty.deadline = now + SNAPSHOT_DELAY;
                }
                Some(_) => {}
            },
        }
        Ok(())
    }

    /// Latest unpersisted authoritative value that clean shutdown must flush.
    pub fn shutdown_snapshot(&self) -> Option<SessionSnapshotV1> {
        self.dirty
            .as_ref()
            .map(|dirty| dirty.snapshot.clone())
            .or_else(|| {
                self.in_flight
                    .as_ref()
                    .map(|request| request.snapshot.clone())
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::os::fd::RawFd;
    use std::time::{Duration, Instant};

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{Layout, Placement, Workarea};
    use realm_core::state::Module;
    use realm_core::{Ledger, WinId};

    use super::{PersistCompletion, PersistenceCoordinator, SNAPSHOT_DELAY};
    use crate::backend::{BackendEvent, BackendResult, BackendWindowId, WmBackend};
    use crate::session::{Session, SessionSnapshotV1};

    fn snapshot_with_layout(layout: Layout) -> SessionSnapshotV1 {
        let mut ledger = Ledger::new();
        ledger.set_layout(layout);
        SessionSnapshotV1::new(ledger, Vec::new(), 0).unwrap()
    }

    #[test]
    fn older_completion_cannot_clear_the_latest_snapshot() {
        for completion in [PersistCompletion::Succeeded, PersistCompletion::Failed] {
            let start = Instant::now();
            let snapshot_a = snapshot_with_layout(Layout::Triptych);
            let snapshot_b = snapshot_with_layout(Layout::Mono);
            let snapshot_c = snapshot_with_layout(Layout::Even);
            let mut coordinator = PersistenceCoordinator::new(None);

            assert!(coordinator.observe(Some(snapshot_a.clone()), start));
            let request_a = coordinator.take_due(start + SNAPSHOT_DELAY).unwrap();
            assert_eq!(request_a.sequence(), 1);
            assert_eq!(request_a.snapshot(), &snapshot_a);

            let later = start + SNAPSHOT_DELAY + Duration::from_millis(10);
            assert!(coordinator.observe(Some(snapshot_b), later));
            let fixed_deadline = later + SNAPSHOT_DELAY;
            assert_eq!(coordinator.deadline(), None);
            assert!(
                coordinator.observe(Some(snapshot_c.clone()), later + Duration::from_millis(10))
            );
            assert_eq!(coordinator.deadline(), None);
            assert!(coordinator
                .take_due(fixed_deadline + Duration::from_secs(1))
                .is_none());

            coordinator
                .complete(request_a.sequence(), completion, later)
                .unwrap();
            assert_eq!(coordinator.deadline(), Some(fixed_deadline));
            assert!(coordinator
                .take_due(fixed_deadline - Duration::from_nanos(1))
                .is_none());
            let request_c = coordinator.take_due(fixed_deadline).unwrap();
            assert_eq!(request_c.sequence(), 2);
            assert_eq!(request_c.snapshot(), &snapshot_c);
        }
    }

    #[test]
    fn failed_latest_snapshot_rearms_from_completion() {
        let start = Instant::now();
        let completed_at = start + SNAPSHOT_DELAY + Duration::from_millis(20);
        let snapshot = snapshot_with_layout(Layout::Mono);
        let mut coordinator = PersistenceCoordinator::new(None);
        coordinator.observe(Some(snapshot.clone()), start);
        let request = coordinator.take_due(start + SNAPSHOT_DELAY).unwrap();

        coordinator
            .complete(request.sequence(), PersistCompletion::Failed, completed_at)
            .unwrap();

        assert_eq!(coordinator.deadline(), Some(completed_at + SNAPSHOT_DELAY));
        assert!(coordinator
            .take_due(completed_at + SNAPSHOT_DELAY - Duration::from_nanos(1))
            .is_none());
        assert_eq!(
            coordinator
                .take_due(completed_at + SNAPSHOT_DELAY)
                .unwrap()
                .snapshot(),
            &snapshot
        );
    }

    #[test]
    fn queued_value_reverting_to_persisted_is_not_written() {
        let start = Instant::now();
        let persisted = snapshot_with_layout(Layout::Triptych);
        let changed = snapshot_with_layout(Layout::Mono);
        let mut coordinator = PersistenceCoordinator::new(Some(persisted.clone()));

        assert!(coordinator.observe(Some(changed), start));
        assert_eq!(coordinator.deadline(), Some(start + SNAPSHOT_DELAY));
        assert!(coordinator.observe(Some(persisted), start + Duration::from_millis(10)));

        assert_eq!(coordinator.deadline(), None);
        assert_eq!(coordinator.shutdown_snapshot(), None);
        assert!(coordinator.take_due(start + SNAPSHOT_DELAY).is_none());
    }

    #[test]
    fn in_flight_value_reversion_is_cleared_only_by_success() {
        let start = Instant::now();
        let completed_at = start + SNAPSHOT_DELAY + Duration::from_millis(20);
        let snapshot_a = snapshot_with_layout(Layout::Triptych);
        let snapshot_b = snapshot_with_layout(Layout::Mono);

        let mut successful = PersistenceCoordinator::new(None);
        successful.observe(Some(snapshot_a.clone()), start);
        let request_a = successful.take_due(start + SNAPSHOT_DELAY).unwrap();
        successful.observe(Some(snapshot_b.clone()), completed_at);
        successful.observe(Some(snapshot_a.clone()), completed_at);
        successful
            .complete(
                request_a.sequence(),
                PersistCompletion::Succeeded,
                completed_at,
            )
            .unwrap();
        assert_eq!(successful.deadline(), None);
        assert_eq!(successful.shutdown_snapshot(), None);

        let mut failed = PersistenceCoordinator::new(None);
        failed.observe(Some(snapshot_a.clone()), start);
        let request_a = failed.take_due(start + SNAPSHOT_DELAY).unwrap();
        failed.observe(Some(snapshot_b), completed_at);
        failed.observe(Some(snapshot_a.clone()), completed_at);
        failed
            .complete(
                request_a.sequence(),
                PersistCompletion::Failed,
                completed_at,
            )
            .unwrap();
        assert_eq!(failed.deadline(), Some(completed_at + SNAPSHOT_DELAY));
        assert_eq!(
            failed
                .take_due(completed_at + SNAPSHOT_DELAY)
                .unwrap()
                .snapshot(),
            &snapshot_a
        );
    }

    #[test]
    fn failed_in_flight_write_clears_latest_value_already_persisted() {
        let start = Instant::now();
        let completed_at = start + SNAPSHOT_DELAY + Duration::from_millis(20);
        let persisted = snapshot_with_layout(Layout::Triptych);
        let changed = snapshot_with_layout(Layout::Mono);
        let mut coordinator = PersistenceCoordinator::new(Some(persisted.clone()));

        coordinator.observe(Some(changed), start);
        let request = coordinator.take_due(start + SNAPSHOT_DELAY).unwrap();
        coordinator.observe(Some(persisted), completed_at);
        coordinator
            .complete(request.sequence(), PersistCompletion::Failed, completed_at)
            .unwrap();

        assert_eq!(coordinator.deadline(), None);
        assert_eq!(coordinator.shutdown_snapshot(), None);
    }

    #[test]
    fn shutdown_yields_the_latest_unpersisted_snapshot() {
        let start = Instant::now();
        let snapshot_a = snapshot_with_layout(Layout::Mono);
        let snapshot_b = snapshot_with_layout(Layout::Even);
        let mut coordinator = PersistenceCoordinator::new(None);
        coordinator.observe(Some(snapshot_a), start);
        coordinator.take_due(start + SNAPSHOT_DELAY).unwrap();
        coordinator.observe(Some(snapshot_b.clone()), start + SNAPSHOT_DELAY);

        assert_eq!(coordinator.shutdown_snapshot(), Some(snapshot_b));
    }

    #[test]
    fn only_authoritative_snapshot_value_changes_become_dirty() {
        let start = Instant::now();
        let mut pre_live = Session::connect(FakeBackend::default()).unwrap();
        let mut empty = PersistenceCoordinator::new(None);
        assert!(!empty.observe(pre_live.snapshot(), start));

        pre_live
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        let baseline = pre_live.snapshot().unwrap();
        let mut coordinator = PersistenceCoordinator::new(Some(baseline.clone()));

        pre_live.toggle_whichkey();
        assert!(!coordinator.observe(pre_live.snapshot(), start));
        pre_live.update_modules(vec![Module {
            id: "clock".to_owned(),
            text: "12:00".to_owned(),
            accent: None,
            urgent: false,
        }]);
        assert!(!coordinator.observe(pre_live.snapshot(), start));
        pre_live
            .handle_backend_event(BackendEvent::WorkareaChanged(Workarea::new(
                1280, 720, 24, 0,
            )))
            .unwrap();
        assert!(!coordinator.observe(pre_live.snapshot(), start));

        pre_live
            .handle_backend_event(BackendEvent::WindowOpened {
                backend_id: BackendWindowId("window-1".to_owned()),
                app_id: "foot".to_owned(),
                title: "one".to_owned(),
            })
            .unwrap();
        assert!(coordinator.observe(pre_live.snapshot(), start));
        let after_open = pre_live.snapshot().unwrap();
        pre_live
            .handle_backend_event(BackendEvent::TitleChanged {
                win: WinId(0),
                title: "renamed".to_owned(),
            })
            .unwrap();
        assert_eq!(pre_live.snapshot(), Some(after_open));
        assert!(!coordinator.observe(pre_live.snapshot(), start));

        pre_live
            .handle_backend_event(BackendEvent::WindowClosed(WinId(0)))
            .unwrap();
        assert!(coordinator.observe(pre_live.snapshot(), start));
        pre_live.set_layout(Layout::Mono).unwrap();
        assert!(coordinator.observe(pre_live.snapshot(), start));
        assert_ne!(pre_live.snapshot().unwrap(), baseline);
    }

    #[derive(Default)]
    struct FakeBackend {
        events: VecDeque<BackendEvent>,
    }

    impl WmBackend for FakeBackend {
        fn name(&self) -> &str {
            "fake"
        }

        fn connect(&mut self) -> BackendResult<Capabilities> {
            Ok(Capabilities {
                exact_geometry: true,
                server_side_borders: true,
                hide_show: true,
                explicit_ordering: true,
                fullscreen: true,
                unsupported: Vec::new(),
            })
        }

        fn assign_window(
            &mut self,
            _backend_id: &BackendWindowId,
            _win: WinId,
        ) -> BackendResult<()> {
            Ok(())
        }

        fn apply(&mut self, _placements: &[Placement]) -> BackendResult<()> {
            Ok(())
        }

        fn focus(&mut self, _win: WinId) -> BackendResult<()> {
            Ok(())
        }

        fn close(&mut self, _win: WinId) -> BackendResult<()> {
            Ok(())
        }

        fn workarea(&self) -> Workarea {
            Workarea::new(1920, 1080, 0, 0)
        }

        fn event_fd(&self) -> RawFd {
            -1
        }

        fn next_event(
            &mut self,
            _deadline: Option<Instant>,
        ) -> BackendResult<Option<BackendEvent>> {
            Ok(self.events.pop_front())
        }
    }
}
