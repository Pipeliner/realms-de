//! One pure backend-work turn for the future event loop.

use std::time::Instant;

use crate::backend::{BackendReady, WmBackend};
use crate::session::{Session, SessionEventError, SessionUpdate};

/// Result of one backend-work invocation.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)] // The Accepted interface carries SessionUpdate directly.
pub enum BackendTurn {
    /// No immediate work or fresh readiness was available.
    Idle,
    /// One internal backend phase made progress without a public event.
    Progressed,
    /// One public backend event produced this successful update.
    Updated(SessionUpdate),
    /// The expected post-flush disconnect completed logout.
    ExitComplete,
}

/// Service at most one backend quantum from immediate work or fresh readiness.
pub fn backend_turn<B: WmBackend>(
    session: &mut Session<B>,
    ready: BackendReady,
    now: Instant,
) -> Result<BackendTurn, SessionEventError> {
    if session.phase() == crate::session::RecoveryPhase::ExitComplete {
        return Err(SessionEventError::InvalidLifecycleOperation {
            operation: "service_backend",
            phase: crate::session::RecoveryPhase::ExitComplete,
        });
    }

    let interest = session.backend_poll_interest();
    if !interest.immediate && !ready.readable && !ready.terminal && !ready.writable {
        return Ok(BackendTurn::Idle);
    }

    let Some(event) = session.service_backend(ready, now)? else {
        return Ok(BackendTurn::Progressed);
    };
    if event == crate::backend::BackendEvent::Disconnected
        && session.phase() == crate::session::RecoveryPhase::Exiting
    {
        session.complete_expected_exit()?;
        return Ok(BackendTurn::ExitComplete);
    }
    session
        .handle_backend_event(event)
        .map(BackendTurn::Updated)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::Workarea;
    use realm_core::WinId;

    use super::{backend_turn, BackendTurn};
    use crate::backend::{
        BackendBindingSpec, BackendContractError, BackendError, BackendEvent, BackendExitPolicy,
        BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
        BackendPollInterest, BackendReady, BackendResult, BackendSubmission, BackendTicket,
        BackendWindowId, WmBackend,
    };
    use crate::session::{RecoveryPhase, Session, SessionEventError};

    const NOT_READY: BackendReady = BackendReady {
        readable: false,
        terminal: false,
        writable: false,
    };

    #[derive(Debug)]
    struct FakeState {
        interest: BackendPollInterest,
        service_results: VecDeque<BackendResult<Option<BackendEvent>>>,
        response_results: VecDeque<BackendResult<BackendSubmission>>,
        service_calls: Vec<BackendReady>,
        configured: Vec<BackendBindingSpec>,
    }

    struct FakeBackend {
        state: Arc<Mutex<FakeState>>,
        event_file: File,
    }

    impl FakeBackend {
        fn new() -> (Self, Arc<Mutex<FakeState>>) {
            let state = Arc::new(Mutex::new(FakeState {
                interest: BackendPollInterest {
                    immediate: false,
                    readable: true,
                    writable: false,
                },
                service_results: VecDeque::new(),
                response_results: VecDeque::new(),
                service_calls: Vec::new(),
                configured: Vec::new(),
            }));
            (
                Self {
                    state: state.clone(),
                    event_file: File::open("/dev/null").unwrap(),
                },
                state,
            )
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

        fn configure_bindings(&mut self, bindings: Vec<BackendBindingSpec>) -> BackendResult<()> {
            self.state.lock().unwrap().configured = bindings;
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
            self.state
                .lock()
                .unwrap()
                .response_results
                .pop_front()
                .unwrap_or(Ok(BackendSubmission::Complete))
        }

        fn begin_exit_session(&mut self, _policy: BackendExitPolicy) -> BackendResult<()> {
            Ok(())
        }

        fn event_fd(&self) -> BorrowedFd<'_> {
            self.event_file.as_fd()
        }

        fn poll_interest(&self) -> BackendPollInterest {
            self.state.lock().unwrap().interest
        }

        fn service(
            &mut self,
            ready: BackendReady,
            _now: Instant,
        ) -> BackendResult<Option<BackendEvent>> {
            let mut state = self.state.lock().unwrap();
            state.service_calls.push(ready);
            state.service_results.pop_front().unwrap_or(Ok(None))
        }
    }

    fn policy_turn(id: u64, events: Vec<BackendPolicyEvent>) -> BackendEvent {
        BackendEvent::PolicyTurn(BackendPolicyTurn {
            id: BackendPolicyTurnId::new(id).unwrap(),
            drains: None,
            events,
        })
    }

    fn live_session() -> (Session<FakeBackend>, Arc<Mutex<FakeState>>) {
        let (backend, state) = FakeBackend::new();
        let mut session = Session::connect(backend).unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(Workarea::new(
                    1920, 1080, 32, 26,
                ))],
            ))
            .unwrap();
        assert_eq!(session.phase(), RecoveryPhase::Live);
        (session, state)
    }

    fn open_two_windows(session: &mut Session<FakeBackend>) {
        session
            .handle_backend_event(policy_turn(
                3,
                vec![
                    BackendPolicyEvent::WindowOpened {
                        backend_id: BackendWindowId::new("one").unwrap(),
                        app_id: "foot".to_owned(),
                        title: "one".to_owned(),
                    },
                    BackendPolicyEvent::WindowOpened {
                        backend_id: BackendWindowId::new("two").unwrap(),
                        app_id: "foot".to_owned(),
                        title: "two".to_owned(),
                    },
                ],
            ))
            .unwrap();
    }

    fn enqueue(state: &Arc<Mutex<FakeState>>, event: BackendResult<Option<BackendEvent>>) {
        let mut state = state.lock().unwrap();
        state.interest.immediate = true;
        state.service_results.push_back(event);
    }

    #[test]
    fn backend_turn_drives_active_success_transaction() {
        let (mut session, state) = live_session();
        open_two_windows(&mut session);
        state
            .lock()
            .unwrap()
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        let origin = session
            .focus_step(realm_core::ledger::Dir::Prev)
            .unwrap()
            .pending_action
            .unwrap();

        enqueue(&state, Ok(Some(policy_turn(4, Vec::new()))));
        assert!(matches!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert!(session.has_active_backend_transaction());

        enqueue(
            &state,
            Ok(Some(BackendEvent::OperationCompleted {
                ticket: origin,
                result: Ok(()),
            })),
        );
        assert!(matches!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert!(session.has_active_backend_transaction());

        enqueue(
            &state,
            Ok(Some(BackendEvent::RetainedObservationsDrained {
                ticket: origin,
            })),
        );
        let BackendTurn::Updated(finalized) =
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap()
        else {
            panic!("drain must produce the final update");
        };
        assert_eq!(finalized.action_completion.unwrap().ticket, origin);
        assert!(!session.has_active_backend_transaction());

        let repeat_id = state
            .lock()
            .unwrap()
            .configured
            .iter()
            .find(|binding| binding.keysym == "j")
            .map(|binding| binding.id)
            .unwrap();
        enqueue(
            &state,
            Ok(Some(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingPressed(repeat_id)],
            ))),
        );
        backend_turn(&mut session, NOT_READY, Instant::now()).unwrap();
        session.fire_key_repeat().unwrap();
        assert!(session.has_active_backend_transaction());
        enqueue(&state, Ok(Some(policy_turn(6, Vec::new()))));
        assert!(matches!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Updated(_)
        ));
        assert!(!session.has_active_backend_transaction());
    }

    #[test]
    fn backend_turn_services_immediate_and_writable_work_once() {
        let (mut session, state) = live_session();
        enqueue(&state, Ok(None));

        assert_eq!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Progressed
        );
        assert_eq!(state.lock().unwrap().service_calls, [NOT_READY]);

        {
            let mut state = state.lock().unwrap();
            state.interest.immediate = false;
            state.interest.writable = true;
            state.service_results.push_back(Ok(None));
        }
        let writable = BackendReady {
            writable: true,
            ..NOT_READY
        };
        assert_eq!(
            backend_turn(&mut session, writable, Instant::now()).unwrap(),
            BackendTurn::Progressed
        );
        assert_eq!(state.lock().unwrap().service_calls, [NOT_READY, writable]);
    }

    #[test]
    fn immediate_none_progress_is_rechecked_before_blocking() {
        let (mut session, state) = live_session();
        enqueue(&state, Ok(None));

        assert_eq!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Progressed
        );
        state.lock().unwrap().interest.immediate = false;
        assert_eq!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap(),
            BackendTurn::Idle
        );
        assert_eq!(state.lock().unwrap().service_calls.len(), 1);
    }

    #[test]
    fn service_error_abandons_pending_operation_fatally() {
        let (mut session, state) = live_session();
        open_two_windows(&mut session);
        session.focus_step(realm_core::ledger::Dir::Prev).unwrap();
        enqueue(
            &state,
            Err(BackendError::Io {
                message: "service failed".to_owned(),
            }),
        );

        assert!(matches!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap_err(),
            SessionEventError::Backend(BackendError::Io { ref message })
                if message == "service failed"
        ));
        assert!(!session.has_active_backend_transaction());
    }

    #[test]
    fn expected_exit_disconnect_returns_exit_complete_once() {
        let (mut session, state) = live_session();
        session.begin_shutdown();
        session.begin_exit_session().unwrap();
        enqueue(&state, Ok(Some(BackendEvent::Disconnected)));

        assert_eq!(
            backend_turn(
                &mut session,
                BackendReady {
                    terminal: true,
                    ..NOT_READY
                },
                Instant::now(),
            )
            .unwrap(),
            BackendTurn::ExitComplete
        );
        assert!(matches!(
            backend_turn(&mut session, NOT_READY, Instant::now()).unwrap_err(),
            SessionEventError::InvalidLifecycleOperation { .. }
        ));
        assert_eq!(state.lock().unwrap().service_calls.len(), 1);
    }
}
