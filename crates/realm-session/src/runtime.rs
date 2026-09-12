//! Concrete request and poll-loop adapter for the Realm session daemon.

use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use realm_control::{
    production_runtime_dir, ConnectionId, ControlAction, ControlError, ControlServer, ControlToken,
    IpcPathError, ReadyEvent, ResponseReceipt, ResponseSettlement, RuntimeDir,
};
use realm_core::ipc::{Request, Response};
use realm_core::ledger::OrbitId;

use crate::backend::{BackendError, BackendReady, BackendTicket, WmBackend, WorkerCapacityError};
use crate::modules::ClockModule;
use crate::persistence::{PersistCompletion, PersistCompletionError, PersistenceCoordinator};
use crate::session::{
    QuitAfter, Session, SessionActionError, SessionEffect, SessionEventError, SessionUpdate,
};
use crate::timers::SessionTimers;
use crate::turn::{backend_turn, BackendTurn};
use crate::worker::{
    FenceStatus, ProcessJob, Worker, WorkerError, WorkerReservation, WorkerResult,
};

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
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// A Session lifecycle invariant failed.
    #[error(transparent)]
    Session(#[from] SessionEventError),
    /// An action failed in a class that terminates the current incarnation.
    #[error("fatal session action failure: {0}")]
    FatalAction(SessionActionError),
    /// The compositor backend failed outside an admitted action.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// The fixed control endpoint could not be prepared or activated.
    #[error(transparent)]
    Endpoint(#[from] IpcPathError),
    /// A bounded worker capacity was exhausted before acknowledgement.
    #[error(transparent)]
    WorkerCapacity(#[from] WorkerCapacityError),
    /// The worker owner failed.
    #[error(transparent)]
    Worker(#[from] WorkerError),
    /// Timer or poll I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The control transport failed outside a peer-local close boundary.
    #[error(transparent)]
    Control(#[from] ControlError),
    /// A worker reported a persistence sequence the owner did not issue.
    #[error(transparent)]
    Persistence(#[from] PersistCompletionError),
    /// Runtime owner state violated the Accepted one-request invariant.
    #[error("runtime owner invariant failed: {0}")]
    Invariant(&'static str),
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

/// Event-loop-owned runtime authorities for one live daemon incarnation.
pub struct RuntimeOwners<B: WmBackend> {
    session: Session<B>,
    control: ControlServer,
    worker: Worker,
    timers: SessionTimers,
    clock: ClockRuntime,
    persistence: PersistenceCoordinator,
    pending_request: Option<(BackendTicket, ConnectionId)>,
    quit_receipt: Option<ResponseReceipt>,
    worker_sealed: bool,
    shutdown_started: bool,
    exit_started: bool,
}

impl<B: WmBackend> RuntimeOwners<B> {
    /// Assemble the owners after recovery reached Live and listener activation succeeded.
    fn new(
        session: Session<B>,
        control: ControlServer,
        worker: Worker,
        timers: SessionTimers,
        clock: ClockRuntime,
        persistence: PersistenceCoordinator,
    ) -> Self {
        Self {
            session,
            control,
            worker,
            timers,
            clock,
            persistence,
            pending_request: None,
            quit_receipt: None,
            worker_sealed: false,
            shutdown_started: false,
            exit_started: false,
        }
    }

    /// Borrow the Session owner.
    pub fn session(&self) -> &Session<B> {
        &self.session
    }

    /// Borrow the control owner for poll-interest collection.
    pub fn control(&self) -> &ControlServer {
        &self.control
    }

    /// Borrow the worker owner for its event descriptor and immediate state.
    pub fn worker(&self) -> &Worker {
        &self.worker
    }

    /// Borrow the timer owner for its descriptors.
    pub fn timers(&self) -> &SessionTimers {
        &self.timers
    }

    /// Nearest fixed control or persistence deadline.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.control.next_deadline(),
            self.persistence.deadline(),
            self.worker.fence_deadline(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Service at most one backend quantum and consume a public update only when allowed.
    pub fn service_backend(
        &mut self,
        now: Instant,
        ready: BackendReady,
    ) -> Result<bool, RuntimeError> {
        use crate::session::RecoveryPhase;

        if matches!(
            self.session.phase(),
            RecoveryPhase::QuitPending | RecoveryPhase::ShuttingDown
        ) {
            return Ok(false);
        }
        match backend_turn(&mut self.session, ready, now)? {
            BackendTurn::Idle | BackendTurn::Progressed => Ok(false),
            BackendTurn::Updated(update) => {
                self.handle_session_update(now, update)?;
                Ok(false)
            }
            BackendTurn::ExitComplete => Ok(true),
        }
    }

    /// Service at most one ready control quantum.
    pub fn service_control(&mut self, now: Instant, ready: ReadyEvent) -> Result<(), RuntimeError> {
        match self.control.service_one(now, ready) {
            Ok(Some(action)) => self.handle_control_action(now, action),
            Ok(None) => Ok(()),
            Err(ControlError::PeerIo { source, .. }) => {
                eprintln!("realm-wm: control peer I/O failed: {source}");
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Apply control timeouts even when no descriptor is ready.
    pub fn expire_control(&mut self, now: Instant) -> Result<(), RuntimeError> {
        self.control.expire(now)?;
        Ok(())
    }

    /// Apply one action emitted by the bounded control transport.
    pub fn handle_control_action(
        &mut self,
        now: Instant,
        action: ControlAction,
    ) -> Result<(), RuntimeError> {
        let ControlAction::Request {
            connection,
            request,
        } = action;
        match dispatch_request(&mut self.session, request)? {
            RequestDispatch::Immediate { response, update } => {
                self.apply_update(now, update, Some((connection, response)))?;
            }
            RequestDispatch::Subscribe(state) => {
                if let Err(error) = self.control.complete_subscribe(now, connection, state) {
                    if !peer_local_completion(&error) {
                        return Err(error.into());
                    }
                }
            }
            RequestDispatch::Pending { ticket } => {
                if self.pending_request.replace((ticket, connection)).is_some() {
                    return Err(RuntimeError::Invariant(
                        "a second control request replaced an admitted request",
                    ));
                }
            }
            RequestDispatch::Quit(update) => {
                self.apply_update(now, update, Some((connection, Response::Ok)))?;
            }
        }
        Ok(())
    }

    /// Consume one Session update returned by the backend owner.
    pub fn handle_session_update(
        &mut self,
        now: Instant,
        update: SessionUpdate,
    ) -> Result<(), RuntimeError> {
        let update = self
            .clock
            .fold_if_clean(&mut self.session, update, SystemTime::now())?;
        let requester = match &update.action_completion {
            None => None,
            Some(completion) => {
                let Some((ticket, connection)) = self.pending_request.take() else {
                    return Err(RuntimeError::Invariant(
                        "action completion had no pending control requester",
                    ));
                };
                if ticket != completion.ticket {
                    return Err(RuntimeError::Invariant(
                        "action completion ticket did not match its requester",
                    ));
                }
                let response = match &completion.result {
                    Ok(()) => Response::Ok,
                    Err(error) => Response::Error {
                        message: error.to_string(),
                    },
                };
                Some((connection, response))
            }
        };
        self.apply_update(now, update, requester)
    }

    fn apply_update(
        &mut self,
        now: Instant,
        mut update: SessionUpdate,
        requester: Option<(ConnectionId, Response)>,
    ) -> Result<(), RuntimeError> {
        self.timers.apply_repeat(&update.repeat_timer)?;

        let mut jobs = Vec::new();
        let mut quit_after = None;
        for effect in update.effects.drain(..) {
            match effect {
                SessionEffect::Spawn(argv) => jobs.push(ProcessJob::Spawn(argv)),
                SessionEffect::Launcher => {
                    jobs.push(ProcessJob::Spawn(vec!["fuzzel".to_owned()]));
                }
                SessionEffect::ReloadTheme => {
                    eprintln!("realm-wm: ignored retired key-derived theme reload");
                }
                SessionEffect::QuitPending { after } => {
                    if quit_after.replace(after).is_some() {
                        return Err(RuntimeError::Invariant(
                            "one update contained more than one Quit barrier",
                        ));
                    }
                }
            }
        }
        let reservation = (!jobs.is_empty())
            .then(|| self.worker.reserve_process_jobs(jobs.len()))
            .transpose()?;

        self.persistence.observe(update.persistence.take(), now);
        if let Some(state) = update.state.take() {
            if let Err(error) = self.control.publish_state(now, state) {
                if !matches!(error, ControlError::OutboundFrameTooLarge { .. }) {
                    cancel_reservation(&mut self.worker, reservation);
                    return Err(error.into());
                }
            }
        }

        let receipt = match requester {
            Some((connection, response)) => {
                match self.control.complete_request(now, connection, response) {
                    Ok(receipt) => Some(receipt),
                    Err(error) if peer_local_completion(&error) => None,
                    Err(error) => {
                        cancel_reservation(&mut self.worker, reservation);
                        return Err(error.into());
                    }
                }
            }
            None => None,
        };

        if let Some(reservation) = reservation {
            self.worker.commit(reservation, jobs)?;
        }

        if let Some(after) = quit_after {
            self.begin_terminal_worker(now)?;
            match after {
                QuitAfter::NoRequester => self.begin_shutdown(now)?,
                QuitAfter::CurrentControlRequest | QuitAfter::OriginalAction(_) => {
                    if let Some(receipt) = receipt {
                        self.quit_receipt = Some(receipt);
                        let settlement = self.control.begin_response_barrier(receipt)?;
                        if settlement != ResponseSettlement::Pending {
                            self.begin_shutdown(now)?;
                        }
                    } else {
                        self.begin_shutdown(now)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn begin_terminal_worker(&mut self, now: Instant) -> Result<(), RuntimeError> {
        if self.worker_sealed {
            return Ok(());
        }
        if let Some(snapshot) = self.persistence.shutdown_snapshot() {
            self.worker.submit_shutdown_snapshot(snapshot)?;
        }
        self.worker.seal(now)?;
        self.worker_sealed = true;
        Ok(())
    }

    fn begin_shutdown(&mut self, now: Instant) -> Result<(), RuntimeError> {
        if self.shutdown_started {
            return Ok(());
        }
        self.quit_receipt = None;
        self.control.begin_shutdown(now);
        let update = self.session.begin_shutdown();
        self.timers.apply_repeat(&update.repeat_timer)?;
        self.shutdown_started = true;
        Ok(())
    }

    /// Recheck the exact Quit receipt and the external shutdown gates.
    pub fn advance_shutdown(&mut self, now: Instant) -> Result<(), RuntimeError> {
        if let Some(receipt) = self.quit_receipt {
            if self.control.response_settlement(receipt) != ResponseSettlement::Pending {
                self.begin_shutdown(now)?;
            }
        }
        if self.shutdown_started
            && !self.exit_started
            && self.control.is_shutdown_complete()
            && self.worker.fence_status(now) != FenceStatus::Pending
        {
            self.session.begin_exit_session()?;
            self.exit_started = true;
        }
        Ok(())
    }

    /// Submit the latest coalesced live snapshot when its fixed deadline is due.
    pub fn submit_due_snapshot(&mut self, now: Instant) -> Result<(), RuntimeError> {
        if !snapshot_submission_allowed(self.worker_sealed) {
            return Ok(());
        }
        if let Some(request) = self.persistence.take_due(now) {
            self.worker
                .submit_snapshot(request.sequence(), request.snapshot().clone())?;
        }
        Ok(())
    }

    /// Consume at most one worker result.
    pub fn service_worker(&mut self, now: Instant) -> Result<(), RuntimeError> {
        let Some(result) = self.worker.try_result()? else {
            return Ok(());
        };
        match result {
            WorkerResult::SnapshotRead(_) => {
                return Err(RuntimeError::Invariant(
                    "startup snapshot result reached the live loop",
                ));
            }
            WorkerResult::SnapshotRequested => self.worker.answer_snapshot_request()?,
            WorkerResult::SnapshotWritten {
                sequence: Some(sequence),
                result,
            } => {
                let completion = match result {
                    Ok(()) => PersistCompletion::Succeeded,
                    Err(error) => {
                        eprintln!("realm-wm: snapshot write failed: {error}");
                        PersistCompletion::Failed
                    }
                };
                self.persistence.complete(sequence, completion, now)?;
            }
            WorkerResult::SnapshotWritten {
                sequence: None,
                result,
            } => {
                if let Err(error) = result {
                    eprintln!("realm-wm: final snapshot write failed: {error}");
                }
            }
            WorkerResult::Process { result } => {
                if let Err(error) = result {
                    eprintln!("realm-wm: process launch failed: {error}");
                }
            }
            WorkerResult::Fence => {}
        }
        Ok(())
    }

    /// Consume one coalesced repeat expiry.
    pub fn service_repeat(&mut self, now: Instant) -> Result<(), RuntimeError> {
        if self.timers.consume_repeat()? {
            let update = self.session.fire_key_repeat()?;
            self.handle_session_update(now, update)?;
        }
        Ok(())
    }

    /// Consume one coalesced clock expiry and re-arm the next minute boundary.
    pub fn service_clock(&mut self, now: SystemTime) -> Result<(), RuntimeError> {
        if self.timers.consume_clock(now)? {
            let update = self.clock.tick(&mut self.session, now)?;
            self.handle_session_update(Instant::now(), update)?;
        }
        Ok(())
    }
}

fn cancel_reservation(worker: &mut Worker, reservation: Option<WorkerReservation>) {
    if let Some(reservation) = reservation {
        worker.cancel(reservation);
    }
}

fn peer_local_completion(error: &ControlError) -> bool {
    matches!(
        error,
        ControlError::StaleConnection { .. }
            | ControlError::ResponseSequenceExhausted { .. }
            | ControlError::OutboundFrameTooLarge { .. }
    )
}

const NOT_READY: BackendReady = BackendReady {
    readable: false,
    terminal: false,
    writable: false,
};

/// Run the concrete daemon lifecycle with one supported backend constructor.
pub fn run_daemon_with<B, F, R>(
    runtime: RuntimeDir,
    make_backend: F,
    ready: R,
) -> Result<(), RuntimeError>
where
    B: WmBackend,
    F: FnOnce() -> Result<B, BackendError>,
    R: FnOnce() -> std::io::Result<()>,
{
    let mut clock = ClockRuntime::system();
    let snapshot_path = runtime.path().join("realm/ledger.json");
    let bound = runtime.prepare_server_endpoint()?.bind()?;

    let mut worker = Worker::start(snapshot_path)?;
    let recovered = load_startup_snapshot(&mut worker)?;
    let mut persistence = PersistenceCoordinator::new(recovered.clone());

    let backend = make_backend()?;
    let mut session = Session::connect_with_snapshot(backend, recovered)?;
    recover_until_live(&mut session, &mut persistence)?;
    let update = clock.tick(&mut session, SystemTime::now())?;
    persistence.observe(update.persistence, Instant::now());

    let control = bound.activate()?.into_server(Instant::now());
    let timers = SessionTimers::new()?;
    let mut owners = RuntimeOwners::new(session, control, worker, timers, clock, persistence);
    ready()?;
    run_combined_loop(&mut owners)
}

fn update_clock_module<B: WmBackend>(
    session: &mut Session<B>,
    clock: realm_core::state::Module,
) -> SessionUpdate {
    let mut modules = session.state().modules.clone();
    modules.retain(|module| module.id != "clock");
    modules.push(clock);
    session.update_modules(modules)
}

struct ClockRuntime {
    module: ClockModule,
    dirty: bool,
}

impl ClockRuntime {
    fn system() -> Self {
        Self {
            module: ClockModule::system(),
            dirty: false,
        }
    }

    #[cfg(test)]
    fn utc() -> Self {
        Self {
            module: ClockModule::utc(),
            dirty: false,
        }
    }

    fn tick<B: WmBackend>(
        &mut self,
        session: &mut Session<B>,
        now: SystemTime,
    ) -> Result<SessionUpdate, RuntimeError> {
        if session.has_active_backend_transaction() {
            self.dirty = true;
            return Ok(SessionUpdate::unchanged());
        }
        self.dirty = false;
        Ok(update_clock_module(session, self.module.render(now)?))
    }

    fn fold_if_clean<B: WmBackend>(
        &mut self,
        session: &mut Session<B>,
        mut update: SessionUpdate,
        now: SystemTime,
    ) -> Result<SessionUpdate, RuntimeError> {
        if !self.dirty || session.has_active_backend_transaction() {
            return Ok(update);
        }
        self.dirty = false;
        let clock_update = update_clock_module(session, self.module.render(now)?);
        if clock_update.state.is_some() {
            update.state = clock_update.state;
        }
        Ok(update)
    }
}

/// Resolve the sole production runtime input and run a backend incarnation.
pub fn run_production_daemon<B, F>(make_backend: F) -> Result<(), RuntimeError>
where
    B: WmBackend,
    F: FnOnce() -> Result<B, BackendError>,
{
    run_daemon_with(production_runtime_dir()?, make_backend, notify_ready)
}

fn load_startup_snapshot(
    worker: &mut Worker,
) -> Result<Option<crate::session::SessionSnapshotV1>, RuntimeError> {
    use crate::snapshot::{classify_snapshot_read, SnapshotLoad};
    use rustix::event::{poll, PollFd, PollFlags};
    use rustix::io::Errno;

    worker.request_snapshot_read()?;
    loop {
        if let Some(result) = worker.try_result()? {
            let WorkerResult::SnapshotRead(read) = result else {
                return Err(RuntimeError::Invariant(
                    "non-snapshot worker result preceded startup snapshot read",
                ));
            };
            return match classify_snapshot_read(read)? {
                SnapshotLoad::Fresh => Ok(None),
                SnapshotLoad::Recovered(snapshot) => Ok(Some(snapshot)),
                SnapshotLoad::Rejected(error) => {
                    eprintln!("realm-wm: rejected session snapshot: {error}");
                    Ok(None)
                }
            };
        }
        let mut fds = [PollFd::from_borrowed_fd(worker.event_fd(), PollFlags::IN)];
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(Errno::INTR) => continue,
            Err(error) => return Err(std::io::Error::from(error).into()),
        }
        let events = fds[0].revents();
        if events.contains(PollFlags::NVAL) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "worker event descriptor became invalid",
            )
            .into());
        }
        if events.intersects(PollFlags::ERR | PollFlags::HUP) {
            return Err(RuntimeError::Invariant(
                "worker event descriptor became terminal during startup",
            ));
        }
    }
}

fn recover_until_live<B: WmBackend>(
    session: &mut Session<B>,
    persistence: &mut PersistenceCoordinator,
) -> Result<(), RuntimeError> {
    use crate::session::RecoveryPhase;
    use rustix::event::{poll, PollFd, PollFlags};
    use rustix::io::Errno;

    loop {
        if session.phase() == RecoveryPhase::Live {
            return Ok(());
        }
        if !matches!(
            session.phase(),
            RecoveryPhase::InitialReplay | RecoveryPhase::FinalizingReplay
        ) {
            return Err(RuntimeError::Invariant(
                "pre-listener recovery left a startup phase",
            ));
        }

        let interest = session.backend_poll_interest();
        let ready = if interest.immediate {
            NOT_READY
        } else {
            let mut flags = PollFlags::empty();
            if interest.readable {
                flags |= PollFlags::IN;
            }
            if interest.writable {
                flags |= PollFlags::OUT;
            }
            let mut fds = [PollFd::from_borrowed_fd(session.backend_event_fd(), flags)];
            loop {
                match poll(&mut fds, None) {
                    Ok(_) => break,
                    Err(Errno::INTR) => continue,
                    Err(error) => return Err(std::io::Error::from(error).into()),
                }
            }
            backend_ready(fds[0].revents())?
        };

        match backend_turn(session, ready, Instant::now())? {
            BackendTurn::Idle => {}
            BackendTurn::Progressed => {}
            BackendTurn::Updated(mut update) => {
                if update.pending_action.is_some()
                    || update.action_completion.is_some()
                    || !update.effects.is_empty()
                {
                    return Err(RuntimeError::Invariant(
                        "pre-listener recovery exposed action-only work",
                    ));
                }
                persistence.observe(update.persistence.take(), Instant::now());
            }
            BackendTurn::ExitComplete => {
                return Err(RuntimeError::Invariant(
                    "backend exited during pre-listener recovery",
                ));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum PollSource {
    Backend,
    Repeat,
    Clock,
    Worker,
    Control(ControlToken),
}

#[derive(Clone, Copy)]
struct ReadySource {
    source: PollSource,
    events: rustix::event::PollFlags,
}

fn run_combined_loop<B: WmBackend>(owners: &mut RuntimeOwners<B>) -> Result<(), RuntimeError> {
    use crate::session::RecoveryPhase;
    use rustix::event::{poll, PollFd, PollFlags};
    use rustix::io::Errno;

    loop {
        let now = Instant::now();
        owners.expire_control(now)?;
        owners.submit_due_snapshot(now)?;
        owners.advance_shutdown(now)?;

        let phase = owners.session.phase();
        let backend_interest = owners.session.backend_poll_interest();
        let backend_enabled = matches!(phase, RecoveryPhase::Live | RecoveryPhase::Exiting);
        let immediate =
            backend_enabled && backend_interest.immediate || owners.worker.has_results();

        let mut sources = Vec::new();
        let mut poll_fds = Vec::new();
        if backend_enabled {
            let mut flags = PollFlags::empty();
            if backend_interest.readable {
                flags |= PollFlags::IN;
            }
            if backend_interest.writable {
                flags |= PollFlags::OUT;
            }
            sources.push(PollSource::Backend);
            poll_fds.push(PollFd::from_borrowed_fd(
                owners.session.backend_event_fd(),
                flags,
            ));
        }
        sources.push(PollSource::Repeat);
        poll_fds.push(PollFd::from_borrowed_fd(
            owners.timers.repeat_fd(),
            PollFlags::IN,
        ));
        sources.push(PollSource::Clock);
        poll_fds.push(PollFd::from_borrowed_fd(
            owners.timers.clock_fd(),
            PollFlags::IN,
        ));
        sources.push(PollSource::Worker);
        poll_fds.push(PollFd::from_borrowed_fd(
            owners.worker.event_fd(),
            PollFlags::IN,
        ));
        for interest in owners.control.poll_interests() {
            let mut flags = PollFlags::empty();
            if interest.readable {
                flags |= PollFlags::IN;
            }
            if interest.writable {
                flags |= PollFlags::OUT;
            }
            sources.push(PollSource::Control(interest.token));
            poll_fds.push(PollFd::from_borrowed_fd(interest.fd, flags));
        }

        let timeout = poll_timeout(immediate, owners.next_deadline(), now)?;
        loop {
            match poll(&mut poll_fds, timeout.as_ref()) {
                Ok(_) => break,
                Err(Errno::INTR) => continue,
                Err(error) => return Err(std::io::Error::from(error).into()),
            }
        }
        let ready = sources
            .iter()
            .zip(&poll_fds)
            .filter_map(|(source, fd)| {
                (!fd.revents().is_empty()).then_some(ReadySource {
                    source: *source,
                    events: fd.revents(),
                })
            })
            .collect::<Vec<_>>();
        drop(poll_fds);
        drop(sources);

        let backend_events = ready_for(&ready, |source| matches!(source, PollSource::Backend));
        let backend_ready = if backend_interest.immediate {
            NOT_READY
        } else if let Some(events) = backend_events {
            backend_ready(events)?
        } else {
            NOT_READY
        };
        if backend_quantum_due(
            backend_enabled,
            backend_events.is_some(),
            backend_interest.immediate,
        ) {
            if owners.service_backend(Instant::now(), backend_ready)? {
                return Ok(());
            }
            if backend_enabled && owners.session.backend_poll_interest().immediate {
                continue;
            }
        }

        if ready_for(&ready, |source| matches!(source, PollSource::Repeat)).is_some() {
            owners.service_repeat(Instant::now())?;
        }
        if ready_for(&ready, |source| matches!(source, PollSource::Clock)).is_some() {
            owners.service_clock(SystemTime::now())?;
        }
        if owners.worker.has_results()
            || ready_for(&ready, |source| matches!(source, PollSource::Worker)).is_some()
        {
            owners.service_worker(Instant::now())?;
        }
        if let Some(ReadySource {
            source: PollSource::Control(token),
            events,
        }) = ready
            .iter()
            .copied()
            .find(|ready| matches!(ready.source, PollSource::Control(_)))
        {
            validate_poll_events(events)?;
            owners.service_control(
                Instant::now(),
                ReadyEvent {
                    token,
                    readable: events.contains(PollFlags::IN),
                    writable: events.contains(PollFlags::OUT),
                    terminal: events.intersects(PollFlags::ERR | PollFlags::HUP),
                },
            )?;
        }
        owners.advance_shutdown(Instant::now())?;
    }
}

fn ready_for(
    ready: &[ReadySource],
    predicate: impl Fn(PollSource) -> bool,
) -> Option<rustix::event::PollFlags> {
    ready
        .iter()
        .find(|ready| predicate(ready.source))
        .map(|ready| ready.events)
}

fn backend_quantum_due(enabled: bool, ready: bool, immediate: bool) -> bool {
    enabled && (ready || immediate)
}

fn snapshot_submission_allowed(worker_sealed: bool) -> bool {
    !worker_sealed
}

fn backend_ready(events: rustix::event::PollFlags) -> Result<BackendReady, RuntimeError> {
    use rustix::event::PollFlags;
    validate_poll_events(events)?;
    Ok(BackendReady {
        readable: events.contains(PollFlags::IN),
        writable: events.contains(PollFlags::OUT),
        terminal: events.intersects(PollFlags::ERR | PollFlags::HUP),
    })
}

fn validate_poll_events(events: rustix::event::PollFlags) -> Result<(), RuntimeError> {
    if events.contains(rustix::event::PollFlags::NVAL) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "poll reported an invalid runtime descriptor",
        )
        .into());
    }
    Ok(())
}

fn poll_timeout(
    immediate: bool,
    deadline: Option<Instant>,
    now: Instant,
) -> Result<Option<rustix::event::Timespec>, RuntimeError> {
    if immediate {
        return Ok(Some(rustix::event::Timespec::default()));
    }
    deadline
        .map(|deadline| {
            deadline
                .saturating_duration_since(now)
                .try_into()
                .map_err(|_| {
                    RuntimeError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "runtime poll timeout does not fit timespec",
                    ))
                })
        })
        .transpose()
}

/// Send `READY=1` to the systemd notification socket when one was supplied.
pub fn notify_ready() -> std::io::Result<()> {
    use rustix::net::{
        sendto, socket_with, AddressFamily, SendFlags, SocketAddrUnix, SocketFlags, SocketType,
    };

    let Some(raw) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let bytes = raw.as_os_str().as_bytes();
    let address = match bytes {
        [b'@', name @ ..] => SocketAddrUnix::new_abstract_name(name)?,
        [] => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NOTIFY_SOCKET is empty",
            ));
        }
        path => SocketAddrUnix::new(PathBuf::from(std::ffi::OsStr::from_bytes(path)))?,
    };
    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::DGRAM,
        SocketFlags::CLOEXEC,
        None,
    )?;
    sendto(socket.as_fd(), b"READY=1", SendFlags::NOSIGNAL, &address)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::fs::File;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use realm_control::test_runtime_dir;
    use realm_core::ipc::{Capabilities, Request, Response};
    use realm_core::layout::Workarea;
    use realm_core::ledger::Dir;
    use realm_core::WinId;

    use super::{dispatch_request, run_daemon_with, ClockRuntime, RequestDispatch};
    use crate::backend::{
        BackendBindingSpec, BackendContractError, BackendEvent, BackendExitPolicy,
        BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
        BackendPollInterest, BackendReady, BackendResult, BackendSubmission, BackendTicket,
        BackendWindowId, WmBackend,
    };
    use crate::persistence::PersistenceCoordinator;
    use crate::session::{Session, SessionUpdate};
    use crate::timers::SessionTimers;
    use crate::worker::Worker;

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
                events: vec![
                    BackendPolicyEvent::WindowOpened {
                        backend_id: BackendWindowId::new("fixture-window").unwrap(),
                        app_id: "fixture".to_owned(),
                        title: "Fixture".to_owned(),
                    },
                    BackendPolicyEvent::InitialReplayComplete,
                ],
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

    struct DaemonBackend {
        event_fd: UnixStream,
        events: VecDeque<BackendEvent>,
        sticky_immediate: bool,
    }

    impl DaemonBackend {
        fn new() -> Self {
            let (event_fd, _peer) = UnixStream::pair().unwrap();
            Self {
                event_fd,
                events: VecDeque::from([
                    BackendEvent::PolicyTurn(BackendPolicyTurn {
                        id: BackendPolicyTurnId::new(1).unwrap(),
                        drains: None,
                        events: vec![BackendPolicyEvent::InitialReplayComplete],
                    }),
                    BackendEvent::PolicyTurn(BackendPolicyTurn {
                        id: BackendPolicyTurnId::new(2).unwrap(),
                        drains: None,
                        events: vec![BackendPolicyEvent::WorkareaChanged(Workarea::new(
                            1920, 1080, 32, 26,
                        ))],
                    }),
                ]),
                sticky_immediate: false,
            }
        }

        fn with_retained_immediate() -> Self {
            Self {
                sticky_immediate: true,
                ..Self::new()
            }
        }
    }

    impl WmBackend for DaemonBackend {
        fn name(&self) -> &str {
            "daemon-fake"
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
            self.events.push_back(BackendEvent::Disconnected);
            Ok(())
        }

        fn event_fd(&self) -> BorrowedFd<'_> {
            self.event_fd.as_fd()
        }

        fn poll_interest(&self) -> BackendPollInterest {
            BackendPollInterest {
                immediate: self.sticky_immediate || !self.events.is_empty(),
                readable: true,
                writable: false,
            }
        }

        fn service(
            &mut self,
            _ready: BackendReady,
            _now: Instant,
        ) -> BackendResult<Option<BackendEvent>> {
            Ok(self.events.pop_front())
        }
    }

    fn fixture_dir(name: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "realm-runtime-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
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

    #[test]
    fn real_socket_get_state_and_exact_quit_receipt_drive_clean_exit() {
        let root = fixture_dir("get-state-quit");
        let server_runtime = test_runtime_dir(&root).unwrap();
        let client_endpoint = test_runtime_dir(&root).unwrap().client_endpoint();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let daemon = std::thread::spawn(move || {
            run_daemon_with(
                server_runtime,
                || Ok(DaemonBackend::new()),
                || {
                    ready_tx.send(()).map_err(|_| {
                        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "test ready receiver")
                    })
                },
            )
        });

        if let Err(error) = ready_rx.recv_timeout(Duration::from_secs(2)) {
            panic!(
                "daemon did not become ready ({error:?}): {:?}",
                daemon.join()
            );
        }
        let mut client = client_endpoint.connect("runtime-fixture").unwrap();
        let Response::State(state) = client.request(Request::GetState).unwrap() else {
            panic!("GetState did not return state");
        };
        assert!(state
            .modules
            .iter()
            .any(|module| module.id == "clock" && module.text.len() == 5));
        assert_eq!(client.request(Request::Quit).unwrap(), Response::Ok);
        drop(client);

        daemon.join().unwrap().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn quit_masks_retained_backend_immediate_interest_until_exit_is_authorized() {
        assert!(!super::backend_quantum_due(false, true, true));
        assert!(!super::backend_quantum_due(false, false, true));
        assert!(super::backend_quantum_due(true, true, false));
    }

    #[test]
    fn sealed_worker_masks_later_dirty_persistence_deadlines() {
        assert!(!super::snapshot_submission_allowed(true));
        assert!(super::snapshot_submission_allowed(false));
    }

    #[test]
    fn retained_immediate_quit_and_due_dirty_snapshot_reach_exit() {
        let root = fixture_dir("retained-immediate-quit");
        let runtime = test_runtime_dir(&root).unwrap();
        let snapshot_path = runtime.path().join("realm/ledger.json");
        let bound = runtime.prepare_server_endpoint().unwrap().bind().unwrap();
        let worker = Worker::start(snapshot_path).unwrap();
        let mut session = Session::connect(DaemonBackend::with_retained_immediate()).unwrap();
        let mut persistence = PersistenceCoordinator::new(None);
        super::recover_until_live(&mut session, &mut persistence).unwrap();
        let control = bound.activate().unwrap().into_server(Instant::now());
        let timers = SessionTimers::new().unwrap();
        let clock = ClockRuntime::utc();
        let mut owners =
            super::RuntimeOwners::new(session, control, worker, timers, clock, persistence);
        let started = Instant::now();
        let quit = owners.session.begin_direct_quit().unwrap();
        owners.apply_update(started, quit, None).unwrap();

        owners
            .submit_due_snapshot(started + crate::persistence::SNAPSHOT_DELAY)
            .expect("a deadline retained after seal must be masked");
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = done_tx.send(super::run_combined_loop(&mut owners));
        });
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("retained backend immediate interest starved shutdown")
            .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clock_tick_during_transaction_folds_into_the_next_clean_publication() {
        let mut session = live_session();
        let mut clock = ClockRuntime::utc();
        let fixed = UNIX_EPOCH + Duration::from_secs(1_735_689_600);
        assert!(session
            .switch_orbit(realm_core::ledger::OrbitId::from_human(2).unwrap())
            .unwrap()
            .pending_action
            .is_some());

        assert_eq!(
            clock.tick(&mut session, fixed).unwrap(),
            SessionUpdate::unchanged()
        );
        assert!(clock.dirty);

        let update = session
            .handle_backend_event(BackendEvent::PolicyTurn(BackendPolicyTurn {
                id: BackendPolicyTurnId::new(3).unwrap(),
                drains: None,
                events: Vec::new(),
            }))
            .unwrap();
        let folded = clock.fold_if_clean(&mut session, update, fixed).unwrap();
        assert!(folded.action_completion.is_some());
        assert!(folded.persistence.is_some());
        let state = folded.state.unwrap();
        assert!(state
            .modules
            .iter()
            .any(|module| module.id == "clock" && module.text == "00:00"));
        assert!(!clock.dirty);
    }
}
