//! One pure backend-work turn for the future event loop.

use std::time::Instant;

use crate::backend::WmBackend;
use crate::session::{Session, SessionEventError, SessionUpdate};

/// Result of one backend-work invocation.
#[derive(Debug, Clone, PartialEq)]
pub enum BackendTurn {
    /// No pending work existed and no ready complete event was available.
    Idle,
    /// Pending repair or one backend event produced this session update.
    Updated(SessionUpdate),
    /// One event was retained and scheduled backend repair for a later turn.
    RetryScheduled(SessionEventError),
}

/// Retry pending work before a future blocking poll, or consume at most one
/// backend event after readiness was reported.
pub fn backend_turn<B: WmBackend>(
    session: &mut Session<B>,
    backend_ready: bool,
    now: Instant,
) -> Result<BackendTurn, SessionEventError> {
    if session.has_pending_backend_work() {
        return session
            .retry_pending_backend_work()
            .map(BackendTurn::Updated);
    }
    if !backend_ready {
        return Ok(BackendTurn::Idle);
    }

    let Some(event) = session.next_backend_event(now)? else {
        return Ok(BackendTurn::Idle);
    };
    match session.handle_backend_event(event) {
        Ok(update) => Ok(BackendTurn::Updated(update)),
        Err(error) if session.has_pending_backend_work() => Ok(BackendTurn::RetryScheduled(error)),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{Placement, Workarea};
    use realm_core::WinId;

    use super::{backend_turn, BackendTurn};
    use crate::backend::{
        BackendBindingSpec, BackendContractError, BackendError, BackendEvent, BackendExitPolicy,
        BackendPolicyResponse, BackendPolicyTurnId, BackendPollInterest, BackendReady,
        BackendResult, BackendSubmission, BackendTicket, BackendWindowId, WmBackend,
    };
    use crate::session::{Session, SessionEventError};

    #[test]
    fn backend_turn_retries_pending_work_without_readiness_before_a_later_read() {
        let reads = Arc::new(AtomicUsize::new(0));
        let deadlines = Arc::new(Mutex::new(Vec::new()));
        let mut backend = FakeBackend::with_reads(reads.clone(), deadlines.clone());
        backend
            .apply_results
            .extend([Ok(()), Ok(()), Err(io_failure("desired apply")), Ok(())]);
        backend
            .events
            .push_back(Ok(Some(BackendEvent::WorkareaChanged(Workarea::new(
                1280, 720, 24, 0,
            )))));
        let mut session = live_session(backend);
        open_window(&mut session);
        session.toggle_stow().unwrap_err();

        assert!(matches!(
            backend_turn(&mut session, false, Instant::now()).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(deadlines.lock().unwrap().is_empty());

        let fixed_now = Instant::now();
        assert!(matches!(
            backend_turn(&mut session, true, fixed_now).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(*deadlines.lock().unwrap(), vec![Some(fixed_now)]);
    }

    #[test]
    fn failed_pending_retry_is_fatal_without_a_read() {
        let reads = Arc::new(AtomicUsize::new(0));
        let deadlines = Arc::new(Mutex::new(Vec::new()));
        let mut backend = FakeBackend::with_reads(reads.clone(), deadlines.clone());
        backend.apply_results.extend([
            Ok(()),
            Ok(()),
            Err(io_failure("desired apply")),
            Err(io_failure("repair retry")),
        ]);
        let mut session = live_session(backend);
        open_window(&mut session);
        session.toggle_stow().unwrap_err();

        let error = backend_turn(&mut session, false, Instant::now()).unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::BackendRetryExhausted(BackendError::Io { ref message })
                if message == "repair retry"
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(deadlines.lock().unwrap().is_empty());
    }

    #[test]
    fn event_that_schedules_repair_returns_without_a_second_read() {
        let reads = Arc::new(AtomicUsize::new(0));
        let deadlines = Arc::new(Mutex::new(Vec::new()));
        let mut backend = FakeBackend::with_reads(reads.clone(), deadlines.clone());
        backend
            .apply_results
            .extend([Ok(()), Err(io_failure("observed apply"))]);
        backend
            .events
            .push_back(Ok(Some(BackendEvent::WindowOpened {
                backend_id: BackendWindowId::new("window-1").unwrap(),
                app_id: "foot".to_owned(),
                title: "one".to_owned(),
            })));
        backend
            .events
            .push_back(Ok(Some(BackendEvent::Disconnected)));
        let mut session = live_session(backend);

        let fixed_now = Instant::now();
        let turn = backend_turn(&mut session, true, fixed_now).unwrap();

        assert!(matches!(
            turn,
            BackendTurn::RetryScheduled(SessionEventError::Backend(
                BackendError::Io { ref message }
            )) if message == "observed apply"
        ));
        assert!(session.has_pending_backend_work());
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(*deadlines.lock().unwrap(), vec![Some(fixed_now)]);

        assert!(matches!(
            backend_turn(&mut session, false, Instant::now()).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(*deadlines.lock().unwrap(), vec![Some(fixed_now)]);
    }

    #[test]
    fn non_ready_turn_without_pending_work_is_idle_without_a_read() {
        let reads = Arc::new(AtomicUsize::new(0));
        let deadlines = Arc::new(Mutex::new(Vec::new()));
        let backend = FakeBackend::with_reads(reads.clone(), deadlines.clone());
        let mut session = live_session(backend);

        assert_eq!(
            backend_turn(&mut session, false, Instant::now()).unwrap(),
            BackendTurn::Idle
        );
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert!(deadlines.lock().unwrap().is_empty());
    }

    fn live_session(backend: FakeBackend) -> Session<FakeBackend> {
        let mut session = Session::connect(backend).unwrap();
        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        session
    }

    fn open_window(session: &mut Session<FakeBackend>) {
        session
            .handle_backend_event(BackendEvent::WindowOpened {
                backend_id: BackendWindowId::new("existing").unwrap(),
                app_id: "foot".to_owned(),
                title: "existing".to_owned(),
            })
            .unwrap();
    }

    fn io_failure(message: &str) -> BackendError {
        BackendError::Io {
            message: message.to_owned(),
        }
    }

    struct FakeBackend {
        reads: Arc<AtomicUsize>,
        deadlines: Arc<Mutex<Vec<Option<Instant>>>>,
        events: VecDeque<BackendResult<Option<BackendEvent>>>,
        apply_results: VecDeque<BackendResult<()>>,
        event_file: File,
    }

    impl FakeBackend {
        fn with_reads(
            reads: Arc<AtomicUsize>,
            deadlines: Arc<Mutex<Vec<Option<Instant>>>>,
        ) -> Self {
            Self {
                reads,
                deadlines,
                events: VecDeque::new(),
                apply_results: VecDeque::new(),
                event_file: File::open("/dev/null").unwrap(),
            }
        }
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
        ) -> Result<(), BackendContractError> {
            Ok(())
        }

        fn configure_bindings(&mut self, _bindings: Vec<BackendBindingSpec>) -> BackendResult<()> {
            Ok(())
        }

        fn request_policy_turn(&mut self) -> BackendResult<()> {
            Ok(())
        }

        fn respond_policy_turn(
            &mut self,
            _turn: BackendPolicyTurnId,
            _ticket: BackendTicket,
            _response: BackendPolicyResponse,
        ) -> BackendResult<BackendSubmission> {
            Ok(BackendSubmission::Complete)
        }

        fn begin_exit_session(&mut self, _policy: BackendExitPolicy) -> BackendResult<()> {
            Ok(())
        }

        fn apply(&mut self, _placements: &[Placement]) -> BackendResult<()> {
            self.apply_results.pop_front().unwrap_or(Ok(()))
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

        fn event_fd(&self) -> BorrowedFd<'_> {
            self.event_file.as_fd()
        }

        fn poll_interest(&self) -> BackendPollInterest {
            BackendPollInterest {
                immediate: !self.events.is_empty(),
                readable: true,
                writable: false,
            }
        }

        fn service(
            &mut self,
            _ready: BackendReady,
            _now: Instant,
        ) -> BackendResult<Option<BackendEvent>> {
            self.events.pop_front().unwrap_or(Ok(None))
        }

        fn next_event(&mut self, deadline: Option<Instant>) -> BackendResult<Option<BackendEvent>> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.deadlines.lock().unwrap().push(deadline);
            self.events.pop_front().unwrap_or(Ok(None))
        }
    }
}
