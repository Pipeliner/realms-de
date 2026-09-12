use realm_session::session::{
    SessionEventError, SessionLifecycleOperation, SessionLifecyclePhase, SessionUpdate,
};

fn accepted_fields_only(update: SessionUpdate) {
    let SessionUpdate {
        repeat_timer,
        persistence,
        state,
        pending_action,
        action_completion,
        effects,
    } = update;
    let _ = (
        repeat_timer,
        persistence,
        state,
        pending_action,
        action_completion,
        effects,
    );
}

fn accepted_event_errors_only(error: SessionEventError) {
    match error {
        SessionEventError::Backend(_)
        | SessionEventError::BackendContract(_)
        | SessionEventError::WindowIdExhausted
        | SessionEventError::BackendTicketExhausted
        | SessionEventError::RepeatedInitialReplayComplete
        | SessionEventError::UnexpectedInitialReplayEvent(_) => {}
        SessionEventError::InvalidLifecycleOperation { operation, phase } => {
            match operation {
                SessionLifecycleOperation::BeginDirectQuit
                | SessionLifecycleOperation::BeginExitSession
                | SessionLifecycleOperation::ServiceAfterExitComplete => {}
            }
            match phase {
                SessionLifecyclePhase::InitialReplay
                | SessionLifecyclePhase::FinalizingReplay
                | SessionLifecyclePhase::Live
                | SessionLifecyclePhase::QuitPending
                | SessionLifecyclePhase::ShuttingDown
                | SessionLifecyclePhase::Exiting
                | SessionLifecyclePhase::ExitComplete => {}
            }
        }
    }
}

fn main() {
    let _ = accepted_fields_only;
    let _ = accepted_event_errors_only;
}
