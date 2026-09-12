//! Concrete request and poll-loop adapter for the Realm session daemon.

use realm_core::ipc::{Request, Response};
use realm_core::ledger::OrbitId;

use crate::backend::{BackendError, BackendTicket, WmBackend};
use crate::session::{Session, SessionActionError, SessionEventError, SessionUpdate};

/// Closed result of translating one Ready-state control request.
#[derive(Debug, Clone, PartialEq)]
pub enum RequestDispatch {
    /// A response whose Session update is already at a clean boundary.
    Immediate {
        /// Response to queue after consuming the update.
        response: Response,
        /// Synchronous update, including any ordered effects.
        update: SessionUpdate,
    },
    /// Subscription admission with its protected initial state.
    Subscribe(realm_core::state::RealmState),
    /// A mutating request admitted under this original backend ticket.
    Pending {
        /// Ticket whose final clean completion owns the response.
        ticket: BackendTicket,
    },
    /// Direct Quit entered the terminal barrier state.
    Quit(SessionUpdate),
}

/// Fatal failure while translating a Ready-state request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// A Session lifecycle invariant failed.
    #[error(transparent)]
    Session(#[from] SessionEventError),
    /// An action failed in a class that terminates the current incarnation.
    #[error("fatal session action failure: {0}")]
    FatalAction(SessionActionError),
}

fn application_error(message: impl Into<String>) -> RequestDispatch {
    RequestDispatch::Immediate {
        response: Response::Error {
            message: message.into(),
        },
        update: SessionUpdate::unchanged(),
    }
}

fn action_result(
    result: Result<SessionUpdate, SessionActionError>,
) -> Result<RequestDispatch, RuntimeError> {
    match result {
        Ok(update) => match update.pending_action {
            Some(ticket) => Ok(RequestDispatch::Pending { ticket }),
            None => Ok(RequestDispatch::Immediate {
                response: Response::Ok,
                update,
            }),
        },
        Err(error @ (SessionActionError::NotReady | SessionActionError::InvalidSpawnCommand)) => {
            Ok(application_error(error.to_string()))
        }
        Err(error @ SessionActionError::Backend(BackendError::Io { .. }))
        | Err(error @ SessionActionError::Backend(BackendError::Unsupported { .. })) => {
            Ok(application_error(error.to_string()))
        }
        Err(error) => Err(RuntimeError::FatalAction(error)),
    }
}

/// Translate every wire request that can occur after the transport handshake.
pub fn dispatch_request<B: WmBackend>(
    session: &mut Session<B>,
    request: Request,
) -> Result<RequestDispatch, RuntimeError> {
    match request {
        Request::Hello { .. } => Ok(application_error("Hello is only valid during handshake")),
        Request::GetState => Ok(RequestDispatch::Immediate {
            response: Response::State(Box::new(session.state().clone())),
            update: SessionUpdate::unchanged(),
        }),
        Request::GetKeymap => Ok(RequestDispatch::Immediate {
            response: Response::Keymap(Box::new(session.keymap().clone())),
            update: SessionUpdate::unchanged(),
        }),
        Request::Subscribe => Ok(RequestDispatch::Subscribe(session.state().clone())),
        Request::SwitchOrbit(number) => {
            let orbit = match OrbitId::from_human(number) {
                Some(orbit) => orbit,
                None => {
                    return Ok(application_error(format!(
                        "invalid one-based orbit {number}"
                    )))
                }
            };
            action_result(session.switch_orbit(orbit))
        }
        Request::MoveToOrbit(number) => {
            let orbit = match OrbitId::from_human(number) {
                Some(orbit) => orbit,
                None => {
                    return Ok(application_error(format!(
                        "invalid one-based orbit {number}"
                    )))
                }
            };
            action_result(session.move_focused_to_orbit(orbit))
        }
        Request::Focus(direction) => action_result(session.focus_step(direction)),
        Request::Swap(direction) => action_result(session.swap(direction)),
        Request::Banish => action_result(session.request_close_focused()),
        Request::Stow => action_result(session.toggle_stow()),
        Request::Fullscreen => action_result(session.toggle_fullscreen()),
        Request::SetLayout(layout) => action_result(session.set_layout(layout)),
        Request::Undo => action_result(session.undo()),
        Request::ReloadTheme => Ok(application_error(
            "theme reload is retired; apply themes through realmctl",
        )),
        Request::Spawn(argv) => action_result(session.request_spawn(argv)),
        Request::ShowLedger(selected) => {
            let selected = match selected {
                None => None,
                Some(number) => match OrbitId::from_human(number) {
                    Some(orbit) => Some(orbit),
                    None => {
                        return Ok(application_error(format!(
                            "invalid one-based orbit {number}"
                        )));
                    }
                },
            };
            Ok(RequestDispatch::Immediate {
                response: Response::Ledger(session.visible_ledger(selected)),
                update: SessionUpdate::unchanged(),
            })
        }
        Request::Quit => Ok(RequestDispatch::Quit(session.begin_direct_quit()?)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::time::Instant;

    use realm_core::ipc::{Capabilities, Request, Response};
    use realm_core::layout::Workarea;
    use realm_core::ledger::Dir;
    use realm_core::WinId;

    use super::{dispatch_request, RequestDispatch};
    use crate::backend::{
        BackendBindingSpec, BackendContractError, BackendEvent, BackendExitPolicy,
        BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
        BackendPollInterest, BackendReady, BackendResult, BackendSubmission, BackendTicket,
        BackendWindowId, WmBackend,
    };
    use crate::session::Session;

    struct FakeBackend(File);

    impl FakeBackend {
        fn new() -> Self {
            Self(File::open("/dev/null").unwrap())
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

        fn event_fd(&self) -> BorrowedFd<'_> {
            self.0.as_fd()
        }

        fn poll_interest(&self) -> BackendPollInterest {
            BackendPollInterest {
                immediate: false,
                readable: true,
                writable: false,
            }
        }

        fn service(
            &mut self,
            _ready: BackendReady,
            _now: Instant,
        ) -> BackendResult<Option<BackendEvent>> {
            Ok(None)
        }
    }

    fn live_session() -> Session<FakeBackend> {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(BackendEvent::PolicyTurn(BackendPolicyTurn {
                id: BackendPolicyTurnId::new(1).unwrap(),
                drains: None,
                events: vec![BackendPolicyEvent::InitialReplayComplete],
            }))
            .unwrap();
        session
            .handle_backend_event(BackendEvent::PolicyTurn(BackendPolicyTurn {
                id: BackendPolicyTurnId::new(2).unwrap(),
                drains: None,
                events: vec![BackendPolicyEvent::WorkareaChanged(Workarea::new(
                    1920, 1080, 32, 26,
                ))],
            }))
            .unwrap();
        session
    }

    #[test]
    fn request_dispatch_uses_live_state_and_exact_session_keymap() {
        let mut session = live_session();

        assert!(matches!(
            dispatch_request(&mut session, Request::GetState).unwrap(),
            RequestDispatch::Immediate {
                response: Response::State(_),
                ..
            }
        ));
        let RequestDispatch::Immediate {
            response: Response::Keymap(keymap),
            ..
        } = dispatch_request(&mut session, Request::GetKeymap).unwrap()
        else {
            panic!("GetKeymap must return the Session-owned keymap");
        };
        assert_eq!(&*keymap, session.keymap());
    }

    #[test]
    fn invalid_orbits_spawn_and_retired_reload_are_application_errors() {
        let mut session = live_session();
        for request in [
            Request::SwitchOrbit(0),
            Request::MoveToOrbit(7),
            Request::ShowLedger(Some(99)),
            Request::Spawn(Vec::new()),
            Request::ReloadTheme,
        ] {
            assert!(matches!(
                dispatch_request(&mut session, request).unwrap(),
                RequestDispatch::Immediate {
                    response: Response::Error { .. },
                    ..
                }
            ));
        }
    }

    #[test]
    fn request_dispatch_table_maps_each_accepted_typed_action() {
        let mut session = live_session();
        let requests = [
            Request::SwitchOrbit(1),
            Request::MoveToOrbit(1),
            Request::Focus(Dir::Next),
            Request::Swap(Dir::Prev),
            Request::Banish,
            Request::Stow,
            Request::Fullscreen,
            Request::SetLayout(realm_core::layout::Layout::Mono),
            Request::Undo,
            Request::ShowLedger(None),
            Request::Spawn(vec!["/bin/true".to_owned()]),
        ];
        for request in requests {
            let result = dispatch_request(&mut session, request).unwrap();
            assert!(matches!(
                result,
                RequestDispatch::Immediate { .. } | RequestDispatch::Pending { .. }
            ));
        }
        assert!(matches!(
            dispatch_request(&mut session, Request::Subscribe).unwrap(),
            RequestDispatch::Subscribe(_)
        ));
        assert!(matches!(
            dispatch_request(&mut session, Request::Quit).unwrap(),
            RequestDispatch::Quit(_)
        ));
    }
}
