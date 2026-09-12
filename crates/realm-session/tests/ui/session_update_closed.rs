use realm_session::session::{SessionEventError, SessionUpdate};

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
        | SessionEventError::UnexpectedInitialReplayEvent(_)
        | SessionEventError::InvalidLifecycleOperation { .. } => {}
    }
}

fn main() {
    let _ = accepted_fields_only;
    let _ = accepted_event_errors_only;
}
