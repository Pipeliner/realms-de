# Interface contracts

> **Status: Accepted for sections 1 and 4 (2026-09-11; MVP fail-closed
> correction 2026-09-12); provisional elsewhere.** These are the seams named in
> [ARCHITECTURE.md](ARCHITECTURE.md).
> They are written down *before* the crates that implement them so that M1 and
> M2 can be built in parallel without two components inventing the same type
> twice.
>
> Signatures here are a design commitment, not final code. Changing one means
> updating this file in the same commit.

---

## 1. `WmBackend` — the compositor seam (ADR 0002, 0003)

The whole point of this trait is that `realm-session` never learns which
compositor it is talking to. `RiverBackend` implements it in phase 1 by *being*
river's window manager; `NativeBackend` implements it in-process against
`realm-compositor` in M5. Nothing above this line changes when we swap them.

```rust
pub type BackendResult<T> = std::result::Result<T, BackendError>;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendWindowId(String);

/// BackendWindowId exposes `new(text) -> Option<Self>` and `as_str()`. The
/// constructor accepts exactly 1..=32 printable ASCII bytes (0x20..=0x7e).
/// Empty, longer, control, and non-ASCII identities are unrepresentable for
/// every WmBackend implementation, so live policy state always fits the closed
/// snapshot V1 identity contract.

/// Opaque nonzero identity allocated once by Session for one backend operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendTicket(std::num::NonZeroU64);

/// Result of one nonblocking backend submission.
pub enum BackendSubmission {
    /// The complete protocol transaction and its final finish were flushed,
    /// and the admitted post-turn epoch was already sealed empty. No decoder
    /// fragment or queued protocol message admitted before this decision may
    /// later yield a retained observation for the turn.
    Complete,
    /// The response was accepted but either final output remains or admitted
    /// post-turn work has not yet been normalized and sealed. Exactly one
    /// matching terminal BackendEvent will arrive later unless this backend
    /// incarnation terminates or Session begins shutdown first. Even after a
    /// synchronous final flush, admitted post-turn work requires Pending; the
    /// backend emits matching Ok and then one tagged turn or empty marker.
    Pending,
}

/// Dynamic poll interest for the backend's one descriptor.
pub struct BackendPollInterest {
    /// Internal protocol or public event work is already queued.
    pub immediate: bool,
    /// POLLIN is useful. False only after the terminal exit cutoff; poll still
    /// reports POLLERR/POLLHUP independently so disconnect remains observable.
    pub readable: bool,
    /// A nonblocking protocol flush currently needs POLLOUT.
    pub writable: bool,
}

/// Readiness returned by the outer poll call.
pub struct BackendReady {
    /// The outer poll observed POLLIN while read interest was enabled.
    pub readable: bool,
    /// The outer poll observed POLLERR or POLLHUP. Terminal readiness is
    /// reported independently even when read interest is disabled.
    pub terminal: bool,
    pub writable: bool,
}

/// Opaque identity of one compositor policy boundary, allocated by the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendPolicyTurnId(std::num::NonZeroU64);

/// Stable Session-selected identity for one configured binding mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendBindingId(std::num::NonZeroU32);

/// All three opaque ids expose `pub fn new(raw) -> Option<Self>`, `get()`, and
/// `TryFrom` for their integer. The field stays private so zero is
/// unrepresentable, while out-of-crate backend implementors can allocate turn
/// ids. Ticket and turn allocators are strictly monotonic within one backend
/// incarnation; exhaustion is fatal and never wraps.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendModifier {
    Shift,
    Control,
    Alt,
    Super,
}

/// MVP policy envelope. Crossing one of these policy limits is a fatal
/// backend/session capacity error; policy is never truncated or reordered.
pub const MAX_MANAGED_WINDOWS: usize = 256;
pub const MAX_CONFIGURED_BINDINGS: usize = 64;
pub const MAX_POLICY_EVENTS: usize = 256;
pub const MAX_STAGED_EFFECTS: usize = 256;
/// Sum of UTF-8 bytes in every backend id, app id, title, and other String
/// occurrence in one exposed policy turn, including replay.
pub const MAX_POLICY_TEXT_BYTES: usize = 65_535;
/// Live River output objects retained by the production backend.
pub const MAX_BACKEND_OUTPUTS: usize = 16;
/// Live River seat objects retained by the production backend. M2 selects one,
/// but still bounds unselected compositor objects.
pub const MAX_BACKEND_SEATS: usize = 16;
/// Live pre-done or configured input-manager device objects.
pub const MAX_BACKEND_INPUT_DEVICES: usize = 64;
/// Live pre-done or configured libinput device objects.
pub const MAX_BACKEND_LIBINPUT_DEVICES: usize = 64;
/// Maximum JSON-encoded content bytes retained for one visible application id.
/// Quotes are excluded; escape expansion is included.
pub const MAX_VISIBLE_APP_ID_JSON_BYTES: usize = 40;
/// Maximum JSON-encoded content bytes retained for one visible window title.
/// Quotes are excluded; escape expansion is included.
pub const MAX_VISIBLE_TITLE_JSON_BYTES: usize = 80;
pub const KEY_REPEAT_RATE_HZ: u32 = 25;
pub const KEY_REPEAT_DELAY_MS: u64 = 600;
/// Process-job admission is fatal and atomic at this limit. The distinct
/// replaceable snapshot slot consumes none of these entries.
pub const MAX_WORKER_JOBS: usize = 256;
/// A full result queue blocks only the worker while the event loop drains it.
pub const MAX_WORKER_RESULTS: usize = 256;
/// Pathname reads retain at most this many bytes; one extra byte makes the
/// snapshot recoverably Rejected and starts fresh.
pub const MAX_SNAPSHOT_BYTES: usize = 65_535;
/// A non-sliding explicit degraded-shutdown fence deadline.
pub const WORKER_SHUTDOWN_TIMEOUT_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerCapacityResource {
    Jobs,
    Results,
}

#[derive(Debug, thiserror::Error)]
#[error("worker capacity exceeded for {resource:?}: limit {limit}")]
pub struct WorkerCapacityError {
    pub resource: WorkerCapacityResource,
    pub limit: usize,
}
/// Replay permits 256 opens, optional ordinary focus, optional exclusive
/// focus, and the singleton final barrier. A layer-shell workarea cannot exist
/// until a later turn because its object depends on an output first reported
/// by the window-manager replay.
pub const MAX_REPLAY_POLICY_EVENTS: usize = MAX_MANAGED_WINDOWS + 3;

/// Mechanism only: Session privately owns id -> Binding -> Action/Mode and the
/// Binding.repeatable policy. The backend never infers repeatability.
pub struct BackendBindingSpec {
    pub id: BackendBindingId,
    /// Canonical xkbcommon keysym name.
    pub keysym: String,
    /// Canonical sorted unique modifier set.
    pub modifiers: Vec<BackendModifier>,
}

/// A fact delivered before one compositor policy boundary.
pub enum BackendPolicyEvent {
    InitialReplayComplete,
    WindowOpened { backend_id: BackendWindowId, app_id: String, title: String },
    WindowClosed(BackendWindowId),
    TitleChanged { backend_id: BackendWindowId, title: String },
    FocusChanged(Option<BackendWindowId>),
    ExclusiveFocusChanged(bool),
    WorkareaChanged(Workarea),
    GeometryDrifted { backend_id: BackendWindowId, rect: Rect },
    BindingPressed(BackendBindingId),
    BindingReleased(BackendBindingId),
    BindingRepeatStopped(BackendBindingId),
    UnboundKeyEaten,
    ModifiersChanged {
        old: Vec<BackendModifier>,
        new: Vec<BackendModifier>,
    },
}

/// Complete report-order batch preceding one mandatory compositor response.
pub struct BackendPolicyTurn {
    pub id: BackendPolicyTurnId,
    /// This turn replaces the named operation's separate drain marker.
    pub drains: Option<BackendTicket>,
    /// Report order is exact. Live/draining length is at most
    /// MAX_POLICY_EVENTS; initial replay is at most MAX_REPLAY_POLICY_EVENTS.
    /// The sum of UTF-8 bytes across every String occurrence is at most
    /// MAX_POLICY_TEXT_BYTES. ModifiersChanged vectors are canonical sorted
    /// unique subsets of the four BackendModifier values. Backend and Session
    /// both reject violations before partial exposure/reduction.
    pub events: Vec<BackendPolicyEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendNextKeyEdge {
    Preserve,
    Ensure,
    Cancel,
}

pub struct BackendBindingState {
    /// Complete canonical sorted unique desired enabled set.
    pub enabled: Vec<BackendBindingId>,
    /// Complete canonical sorted unique desired modifier-watch set.
    pub watched_modifiers: Vec<BackendModifier>,
    /// Explicit one-shot edge for this response. Failed ordinary responses are
    /// fatal in the MVP and produce no recovery response.
    pub next_key_edge: BackendNextKeyEdge,
}

/// Session's complete policy answer to one offered turn.
pub struct BackendPolicyResponse {
    /// None preserves a clean projection; Some(empty) hides every window.
    pub projection: Option<Vec<Placement>>,
    /// First-occurrence report order, unique, and drawn only from assigned live
    /// windows. Repeated closes for one still-live target collapse to one edge;
    /// a close after WindowClosed emits no edge.
    pub closes: Vec<WinId>,
    pub bindings: BackendBindingState,
}

/// Session-authorized fixed response template for policy sequences that the
/// backend admits after terminal exit begins. Projection and closes are
/// structurally absent; the backend always emits next-key Cancel.
pub struct BackendExitPolicy {
    pub enabled: Vec<BackendBindingId>,
    pub watched_modifiers: Vec<BackendModifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendProtocolErrorKind {
    TruncatedPayload,
    TruncatedAncillary,
    InvalidFraming,
    InvalidObjectOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendCapacityResource {
    ManagedWindows,
    ConfiguredBindings,
    Outputs,
    Seats,
    InputDevices,
    LibinputDevices,
    ObjectOrdinals,
    PolicyFacts,
    PolicyEffects,
    PolicyTextBytes,
    RetainedBytes,
    RetainedDescriptors,
    PolicyTurnIds,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend cannot honour capability {capability}")]
    Unsupported { capability: String },
    #[error("backend disconnected")]
    Disconnected,
    #[error("backend unavailable: {message}")]
    Unavailable { message: String },
    #[error("backend I/O failed: {message}")]
    Io { message: String },
    #[error("backend protocol violation: {kind:?}")]
    Protocol { kind: BackendProtocolErrorKind },
    #[error("backend capacity exceeded for {resource:?}: limit {limit}")]
    Capacity {
        resource: BackendCapacityResource,
        limit: u64,
    },
}

/// Local identity-map contract violation. Assignment performs no protocol I/O;
/// every error is fatal for the current backend incarnation.
#[derive(Debug, thiserror::Error)]
pub enum BackendContractError {
    #[error("backend window is not live in this incarnation: {backend_id:?}")]
    UnknownWindow { backend_id: BackendWindowId },
    #[error("backend identity already has a different Realm assignment")]
    ConflictingBackendIdentity,
    #[error("Realm window id already has a different backend identity")]
    ConflictingRealmIdentity,
    #[error("backend identity was referenced before current-incarnation open")]
    UnknownWindowReference,
    #[error("backend reported an unconfigured binding id: {id:?}")]
    UnknownBinding { id: BackendBindingId },
    #[error("backend modifier vectors are not canonical sorted unique subsets")]
    NonCanonicalModifiers,
    #[error("backend emitted an invalid policy transaction sequence")]
    InvalidPolicySequence,
}

pub enum SessionActionError {
    /// Startup finalization or another successful response chain is active.
    NotReady,
    /// Spawn requires at least one nonempty program string.
    InvalidSpawnCommand,
    /// This backend incarnation has consumed every nonzero ticket.
    BackendTicketExhausted,
    /// The error-atomic turn request failed before admission.
    Backend(BackendError),
}

pub enum SessionEventError {
    BackendTicketExhausted,
    WindowIdExhausted,
    RepeatedInitialReplayComplete,
    UnexpectedInitialReplayEvent(BackendPolicyEvent),
    InvalidLifecycleOperation {
        operation: SessionLifecycleOperation,
        phase: SessionLifecyclePhase,
    },
    BackendContract(BackendContractError),
    Backend(BackendError),
}

pub enum SessionLifecyclePhase {
    InitialReplay,
    FinalizingReplay,
    Live,
    QuitPending,
    ShuttingDown,
    Exiting,
    ExitComplete,
}

pub enum SessionLifecycleOperation {
    BeginDirectQuit,
    BeginExitSession,
    ServiceAfterExitComplete,
}

pub struct ActionCompletion {
    /// The original external origin ticket, never a tagged follow-up ticket.
    pub ticket: BackendTicket,
    pub result: Result<(), SessionActionError>,
}

pub enum QuitAfter {
    /// Key-only Quit: no response frame exists.
    NoRequester,
    /// Quit was derived in a turn that also completes an external action.
    OriginalAction(BackendTicket),
    /// Direct Request::Quit: the adapter uses its current ConnectionId.
    CurrentControlRequest,
}

pub enum SessionEffect {
    Spawn(Vec<String>),
    Launcher,
    Grimoire,
    ReloadTheme,
    /// Stop admission now and apply the typed response-drain barrier.
    QuitPending { after: QuitAfter },
}

pub enum RepeatTimerDirective {
    /// Leave the currently programmed timerfd state unchanged.
    Preserve,
    /// Program/restart the sole target from now with the configured delay and
    /// interval. Every Arm edge restarts even when the target id is unchanged.
    Arm {
        delay: std::time::Duration,
        interval: std::time::Duration,
    },
    /// Disarm immediately; idempotent when already disarmed.
    Disarm,
}

/// One transition result. Consumers process fields in this exact order:
/// repeat timer directive, persistence, state publication, action completion,
/// then effects.
pub struct SessionUpdate {
    /// Immediate edge for the outer timerfd owner; never delayed behind effects.
    pub repeat_timer: RepeatTimerDirective,
    pub persistence: Option<SessionSnapshotV1>,
    pub state: Option<RealmState>,
    /// Present only on successful external turn-request admission.
    pub pending_action: Option<BackendTicket>,
    /// Present only at the origin transaction's final clean boundary.
    pub action_completion: Option<ActionCompletion>,
    /// Bounded by MAX_STAGED_EFFECTS across the entire active transaction and
    /// every tagged follow-up, in report order.
    pub effects: Vec<SessionEffect>,
}

/// Result of one bounded backend-work helper invocation.
pub enum BackendTurn {
    /// No immediate work, readiness, or event was consumed.
    Idle,
    /// One internal backend phase made progress and exposed no public event.
    Progressed,
    /// One public backend event produced a successful Session update.
    Updated(SessionUpdate),
    /// The expected post-flush disconnect completed logout. Emitted once.
    ExitComplete,
}

pub fn backend_turn<B: WmBackend>(
    session: &mut Session<B>,
    ready: BackendReady,
    now: Instant,
) -> Result<BackendTurn, SessionEventError>;

/// The helper queries `session.backend_poll_interest()` itself. A caller uses
/// `BackendReady { readable: false, terminal: false, writable: false }` before poll;
/// immediate interest still runs one bounded phase. After poll,
/// the caller passes only that fresh readiness snapshot. Consumed readiness is
/// never inferred or reused inside a later invocation.

/// Typed desired operations (`switch_orbit`, `set_layout`, `focus_step`,
/// `swap`, `move_focused_to_orbit`, `toggle_stow`, `toggle_fullscreen`, `undo`,
/// and `request_close_focused`) all return
/// `SessionActionResult<SessionUpdate>`. `handle_backend_event` returns
/// `Result<SessionUpdate, SessionEventError>`. The old projection_applied and
/// deferred-event fields and the close method's Option<WinId> result do not
/// survive this contract.
/// `pub fn request_spawn(&mut self, argv: Vec<String>) ->
/// SessionActionResult<SessionUpdate>` is the typed effect-only direct Spawn
/// entry point. It is valid only in Live with no active transaction, rejects an
/// empty vector or empty program as `InvalidSpawnCommand`, allocates no backend
/// ticket/turn, and returns exactly one `SessionEffect::Spawn(argv)`. The #38
/// adapter must reserve one worker process slot before acknowledging it; a
/// queued Ok means the argv is admitted, not that process creation succeeded.
/// `pub fn fire_key_repeat(&mut self) -> Result<SessionUpdate,
/// SessionEventError>` is the timer-origin entry point. It first rechecks that
/// the sole captured binding is still held, armed, configured, marked
/// repeatable, and enabled by committed binding policy. A successfully
/// finalized repeatable press replaces and restarts that sole target; releasing
/// or stopping a noncurrent binding does not disarm it, and releasing the
/// current target never resumes an older held binding. A finalized mode change
/// that disables the target disarms it. While any backend transaction is
/// active, a timer fire is consumed as an unchanged update, retains the target,
/// and allocates or requests nothing. Otherwise a local no-op or effect-only
/// result finalizes synchronously. Work requiring a policy response enters an
/// internal-origin transaction, reserves a response ticket, and requests one
/// turn, but sets neither `pending_action` nor `action_completion`; any request
/// or post-admission failure is fatal. Release or repeat-stop of
/// the current target observed before the call makes it an unchanged no-op, and
/// either event in the response irreversibly disarms that target.
/// `pub fn begin_direct_quit(&mut self) -> Result<SessionUpdate,
/// SessionEventError>` is separate. Its first
/// call in Live needs no
/// backend ticket or turn, abandons private W/origin/effects without
/// completion or backend response, stops admission, and returns
/// `QuitPending { CurrentControlRequest }`. A later call in QuitPending is
/// idempotent and returns `SessionUpdate::unchanged()` with no duplicate effect.
/// Calls in InitialReplay, FinalizingReplay, ShuttingDown, Exiting, or
/// ExitComplete return the exact typed `InvalidLifecycleOperation` without
/// mutation.
/// `pub fn begin_shutdown(&mut self) -> SessionUpdate` is the total,
/// idempotent Session-side transition used after the applicable control
/// response barrier settles. Its first call in InitialReplay,
/// FinalizingReplay, Live, or QuitPending abandons every
/// private candidate, origin, ticket, close, and effect while preserving the
/// consumed-ticket watermark, enters
/// ShuttingDown, and returns only an immediate repeat-timer Disarm. It neither
/// calls the backend and emits no persistence, state, action completion, or
/// ordinary effect. Later calls in ShuttingDown, Exiting, or
/// ExitComplete are unchanged.
/// `pub fn begin_exit_session(&mut self) -> Result<(), SessionEventError>` is
/// legal exactly once in ShuttingDown. #38 MUST first prove its exact response
/// settlement, control drain, and worker-fence/deadline conditions. Session
/// owns no response receipt or clock, and those conditions are not observable
/// by it.
/// It calls the backend method exactly once with `BackendExitPolicy` built from
/// the last committed enabled/watch sets and changes Session to Exiting only on success;
/// a wrong-phase or repeated call is a contract error and a backend failure leaves
/// the process on its fatal path rather than pretending Exiting began.
/// `pub fn visible_ledger(&self, orbit: Option<OrbitId>) -> Vec<OrbitLedger>` is
/// the sole ShowLedger adapter seam. It renders the immutable ledger and window
/// metadata snapshot captured at the same final visible boundary as `state()`;
/// it never reads the newer authoritative `ledger()` / `window_metadata()`
/// values while any successful response/drain chain is active. `None` returns all
/// orbits; the adapter must validate one-based wire input into `OrbitId` before
/// calling this typed seam. At observation ingress, app ids and titles are
/// normalized to the JSON-content byte caps above at a Unicode-scalar boundary;
/// overlong nonempty values end with U+2026 within the cap. The maximum
/// 256-window all-orbit result must bounded-encode inside one
/// `realm_core::ipc::MAX_FRAME_BYTES` frame, so this clone cannot grow with
/// repeated adversarial title changes.
/// `pub fn backend_event_fd(&self) -> BorrowedFd<'_>` and
/// `pub fn backend_poll_interest(&self) -> BackendPollInterest` are the sole
/// read-only registration seams for #38. Session retains exclusive mutable
/// ownership of the backend; the outer driver never extracts it or calls
/// `WmBackend` directly.
/// The control adapter classifies the first typed mutating call exactly: an
/// application-class `Err` is an immediate Error, while a fatal-class `Err`
/// terminates the backend incarnation without an ordinary response;
/// `Ok(update)` with
/// `pending_action == None` is synchronously final and is persisted/published
/// before queuing Ok; `Some(ticket)` is admission only and waits for the later
/// matching `ActionCompletion`. It never manufactures an `ActionCompletion`
/// for a ticketless local completion.

/// A window manager realm can drive.
///
/// Implementations translate realm's ledger operations into whatever the
/// underlying compositor understands, and translate the compositor's events
/// back into ledger deltas. They own no window-management policy: the ledger
/// decides what should happen, and the backend only makes it so. The sole
/// ordering exception is the shared compositor-independent retained-observation
/// helper required to serialize protocol facts behind pending operations.
pub trait WmBackend: Send {
    /// Human-readable name, shown by `realmctl doctor`.
    fn name(&self) -> &str;

    /// Connect, and report what the backend can actually honour.
    fn connect(&mut self) -> BackendResult<Capabilities>;

    /// Bind a compositor-stable window identity to Realm's allocated id.
    ///
    /// This is a local, bounded, nonblocking identity-map installation and
    /// emits no compositor request. Repeating the same pair is a no-op. An
    /// unknown object or conflicting pair is a fatal backend-contract error;
    /// it is never retryable while River waits for a policy response. The
    /// binding must succeed before the response can include the window.
    fn assign_window(
        &mut self,
        backend_id: &BackendWindowId,
        win: WinId,
    ) -> Result<(), BackendContractError>;

    /// Register and bound binding mechanisms without assigning them Realm
    /// policy. This performs no seat-dependent protocol I/O; a River backend
    /// creates the concrete objects only after initial replay selects a seat.
    fn configure_bindings(
        &mut self,
        bindings: Vec<BackendBindingSpec>,
    ) -> BackendResult<()>;

    /// Idempotently request a future policy turn without blocking.
    ///
    /// This operation is error-atomic: Err means no request was registered.
    fn request_policy_turn(&mut self) -> BackendResult<()>;

    /// Consume exactly one offered turn and submit Session's complete answer.
    ///
    /// This is the sole owner of the compositor's open policy sequence. It
    /// applies the complete binding state, close edges, and optional projection,
    /// emits exactly one manage finish, and later emits exactly one render
    /// finish when required. A returned error is fatal because the backend may
    /// otherwise leave the compositor waiting in an open sequence.
    fn respond_policy_turn(
        &mut self,
        turn: BackendPolicyTurnId,
        ticket: BackendTicket,
        response: BackendPolicyResponse,
    ) -> BackendResult<BackendSubmission>;

    /// Begin the terminal user-requested compositor exit after socket shutdown.
    /// This call is the local exit linearization cutoff. It cancels any
    /// prepared read, stops new ingress admission, and suppresses public
    /// exposure of terminal policy work. A turn already consumed by
    /// `respond_policy_turn` keeps every byte already emitted or queued and
    /// finishes any manage/render phase already open or parsed with that
    /// immutable response exactly once. A later render phase still kernel-unread
    /// at the cutoff is abandoned, never replaced. Only an admitted/open/queued
    /// sequence not yet exposed and answered receives the Session-authorized
    /// neutral policy. Every applicable open-phase finish flushes before the
    /// exit request is queued and flushed.
    /// Kernel-unread bytes, superseded
    /// decoded non-policy work, and incomplete decoder fragments are terminally
    /// superseded and every backend-owned descriptor retained by them is closed
    /// exactly once. Poll interest then
    /// disables readable but retains writable plus unconditional terminal
    /// observation. Only a terminal event after full flush may produce the
    /// expected Disconnected. No later public request/respond call is valid.
    fn begin_exit_session(&mut self, policy: BackendExitPolicy) -> BackendResult<()>;

    /// The backend descriptor for the session event loop's poll set.
    fn event_fd(&self) -> std::os::fd::BorrowedFd<'_>;

    /// Return queued-work and conditional read/write interest without side effects.
    fn poll_interest(&self) -> BackendPollInterest;

    /// Perform one bounded nonblocking protocol quantum and expose at most one event.
    fn service(
        &mut self,
        ready: BackendReady,
        now: Instant,
    ) -> BackendResult<Option<BackendEvent>>;
}

/// What a backend can and cannot do, so realm degrades honestly rather than
/// pretending. This serialisable wire type lives in `realm_core::ipc`, and
/// `realmctl doctor` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// True when the *rendered* rectangle is exactly the projected one.
    ///
    /// Not "can we ask for arbitrary rects" — we always can. This asks whether
    /// what lands on screen matches, which `propose_dimensions` alone cannot
    /// promise because clients may quantise. True at `river_window_v1` >= 3,
    /// where `set_content_clip_box` lets realm clip to the exact tile; false
    /// below it, with `"unclipped-dimension-quantisation"` in `unsupported`.
    pub exact_geometry: bool,
    pub server_side_borders: bool,
    pub hide_show: bool,          // is stow expressible?
    pub explicit_ordering: bool,  // can we set stacking order directly?
    pub fullscreen: bool,
    pub unsupported: Vec<String>, // named realm behaviours this backend cannot honour
}

/// Something the compositor did that the ledger needs to know about.
pub enum BackendEvent {
    /// The complete report-order input/observation batch for one response.
    PolicyTurn(BackendPolicyTurn),
    /// The one terminal result for the named pending operation.
    OperationCompleted {
        ticket: BackendTicket,
        result: BackendResult<()>,
    },
    /// No policy turn was retained behind this completion; its empty
    /// observation drain is now sealed.
    RetainedObservationsDrained { ticket: BackendTicket },
    Disconnected,
}
```

The backend allocates strictly increasing nonzero `BackendPolicyTurnId`s; each
offered turn must be consumed exactly once. It may not send `manage_finish`
before Session answers and may not service past an unanswered boundary. Session
folds the complete report-order batch into its current committed or private
working state, resolves binding ids through its private Keymap policy, and
derives one complete response. Even an empty, release-only, modifier-only,
spawn-only, quit, or projection-equal turn receives a response. A stale,
duplicate, or missing response is fatal.

The MVP envelope is deliberately finite: at most 256 managed windows, 64
configured binding mechanisms, 16 live outputs, 16 live seats, 64 live input
devices, 64 live libinput devices, and 256 non-coalesced events in one
live/draining policy turn. Events have a per-turn 256-item cap. Staged effects
have a separate 256-item cap across the entire active transaction, including
the admitted external effect and effects derived in every initial or tagged
follow-up turn. Initial replay permits at most 259 events: 256 opens, one
optional latest ordinary focus, one optional latest exclusive-focus state, and
the singleton final barrier. Session begins with no authoritative workarea.
The first replay response reconciles and binds identities but deliberately
contains no projection and keeps every binding disabled. River reports output
objects only in that replay; only then can the backend request the corresponding
layer-shell output objects. A later policy turn supplies the selected output's
first authoritative workarea. Its successfully finalized complete projection
is the earliest transition to Live and the earliest publication/readiness
boundary. A workarea inside the initial replay, and a duplicated or misplaced
barrier, are typed fatal contract errors before assignment or response. The
backend may coalesce only title, focus, workarea,
modifier, geometry, and lifecycle facts when the resulting report-order
reduction is provably identical. There is one named normalization exception: a
backend-local object opened and closed wholly before either lifecycle fact is
exposed may be elided, and by definition never entered Realm identity
authority. Once an open is exposed, lifecycle reduction includes Realm's
never-reuse watermark and cannot be elided. The backend never coalesces, drops,
or reorders binding
press/release/repeat-stop or unbound-key input. Derived actions preserve report
order until the first Quit. That first Quit is a sticky terminal barrier for
the whole transaction: later derived actions/effects in the same turn and all
tagged follow-up turns are deliberately suppressed while later authoritative
input/compositor facts are still retained. Crossing an
envelope limit terminates the backend incarnation with a typed capacity error;
it never exposes a partial turn or partially extends the transaction effect
list. The response close list is first-occurrence ordered and unique, so it is
also bounded by the 256 live-window envelope. Thus one public dequeue, Session
reduction, and the complete response are bounded independently of peer traffic
or the number of objects the compositor attempts to manage.

`configure_bindings` registers mechanism only. The backend never learns a
Realm `Mode` or `Action`; Session owns the stable id-to-binding mapping and
supplies the complete enabled/watch state in every policy response.
`request_policy_turn` is idempotent, error-atomic, and permits at most one
outstanding request. An error-atomic `Io` or `Unsupported` can therefore be
returned as an immediate application error; `Disconnected` or `Unavailable` is
fatal. An uncertain or already buffered `manage_dirty` must return success and
surface later transport loss through `service`, never return an ambiguous
request error. Success guarantees that `manage_dirty` is already flushed or
that `poll_interest().immediate || poll_interest().writable` remains true until
bounded service flushes it; `poll_interest().readable` then remains true and
awaits the turn until the terminal exit cutoff.
The next untagged `PolicyTurn` consumes any outstanding external-action or
internal-repeat request. A tagged draining turn consumes only the named
successful completion drain and cannot also satisfy a separate outstanding
request. For an initial external request, error-atomic `Io`/`Unsupported` is an
immediate application error. For an initial internal-repeat request, any error
is fatal because no requester can receive an application Error. Once a turn is
offered or a response accepted, every response-call, completion, or service
error is fatal under the fail-closed rule below. Once reserved, an origin ticket
remains consumed even if the error-atomic request fails. No path reuses a ticket
number.
An external desired action that changes required response state stages its
private candidate, reserves its origin
ticket, and requests a turn; it does not submit projection state or commit. A
clean projection-equal action with no binding or edge change finalizes locally
without a ticket or turn. The
eventual turn may also contain compositor observations and binding inputs, all
of which Session folds before producing the sole response. This is why the seam
has no independent projection, focus, or close submission methods.

Session allocates strictly increasing nonzero `BackendTicket`s for policy
responses and permits only one active response transaction. `Complete` is legal
only when every required finish has been flushed. `Pending` requires exactly
one later matching `OperationCompleted` unless the backend incarnation
terminates or #38 externally begins shutdown first. Either transition abandons
the ticket without committing or completing an action; Session emits no
shutdown discard response, and #40 owns the backend cutoff. Unknown, stale,
duplicate, and out-of-order completions are fatal contract failures. Exhausting
`u64` is a typed fatal session error: zero is never submitted, the maximum is
never reused, and the attempted response is not sent.

Every compositor-origin window reference remains a `BackendWindowId` in the
offered batch. In live and draining turns, Session resolves identities in
report order: `WindowOpened` allocates/restores the Realm id and calls local
`assign_window`, after which later title/focus/geometry/close events in that
same turn may resolve it. Initial replay is the sole exception: a focus
reference may resolve against an earlier open in the replay accumulator, but no
Realm id is allocated or assigned until the final barrier preflights the whole
batch and reconciliation succeeds. Referring to an identity before its open, or
before current-incarnation assignment outside that replay exception, is a fatal
contract error. The backend never invents a Realm `WinId` while building a turn.

Returning an error from `respond_policy_turn` is fatal for every error class:
River may otherwise remain blocked inside an open manage sequence. After
admission, matching terminal `Io` or `Unsupported` is also fatal, as are
`Protocol`, `Capacity`, `Disconnected`, `Unavailable`, and service errors.
Session abandons all private desired, key, effect, and compositor-observation
state; it emits no ordinary completion, state, persistence, diagnostic, or
effect and requests no recovery turn. Supervised restart and River replay are
the MVP recovery boundary.

An observed-only live turn has no external desired/close origin and no
key-policy delta or effect. Its compositor facts remain private with every
other transaction component. For a pending response, even matching `Ok` does
not install those facts into committed authority or `snapshot()` while the
drain is outstanding. A tagged observed-only turn likewise remains absent from
committed authority and snapshot until its own response finishes and chain
reaches a clean final boundary. That boundary atomically advances committed
authority, the closed V1 snapshot, visible state, visible ledger, and the
last-clean projection. A terminal error promotes nothing and is fatal.

After every successful pending completion the backend exposes facts it had
already received behind that operation, then seals them in exactly one of two
ways: a `PolicyTurn` whose `drains` names the completed ticket, or an empty
`RetainedObservationsDrained` marker. The two forms are mutually exclusive for
that ticket. The empty marker is not eligible until every decoder fragment and
queued protocol message admitted before the completion was normalized; a fact
still unread in the kernel may form a later ordinary turn. A draining policy
turn both closes the prior observation drain and
opens the next mandatory response; Session reduces its batch into the prior
private working state and responds with a fresh ticket. The single-flight gate
and any original action result remain active through this chain. A completion
reporting any terminal error, a standalone `Disconnected`, or a response/service
error terminates the incarnation and abandons
the drain.

The retained turn is bounded by managed windows plus fixed focus/workarea/input
state and may coalesce only when its deterministic report-order reduction is
equivalent. All semantic and observed working state remains private from
admission through the whole successful chain; there is no response-local
commit checkpoint. A final clean boundary is either an empty marker requiring
no more turn, or `Complete` from the final policy response. It atomically
co-releases the committed authority, visible state, persistence value, original
action result, repeat directive, bounded effect vector, and any `QuitAfter` in
one `SessionUpdate`; none is exposed early. Across the vector, effects before
the first Quit retain report order and later derived effects are suppressed.
Task 3 proves that closed library result and ordering only. #38 owns temporal
consumption as repeat directive, persistence, publication, requester response,
then effects, including the exact response-before-shutdown handoff. Key-only
Quit has no response receipt and is consumed after the same successful final
boundary.

`BackendBindingState::next_key_edge` is not a Boolean desired state.
`Preserve` emits neither edge, `Ensure` emits exactly one
`ensure_next_key_eaten`, and `Cancel` emits exactly one
`cancel_ensure_next_key_eaten`. The MVP emits these edges only in successful
ordinary responses and the fixed exit policy; any post-admission failure is
fatal. Deciding whether an uncertain failed Ensure needs Cancel, or whether a
consumed prior Ensure must be restored after rollback-to-continue, is explicit
post-MVP recovery work.

**Post-MVP only:** an Accepted recovery amendment may add `RetryReady` /
`AwaitingRepairTurn`, one bounded repair, repair-ticket exhaustion,
second-failure handling, rollback-to-continue, early observed-only authority,
uncertain-edge recovery, continuing failure diagnostics, and shutdown discard
sequencing. Those states, ticket origins, and outcomes are not part of the MVP
`SessionUpdate` or `BackendTurn` shapes and Task 4 must not depend on them.

Any such recovery must record provenance per attempted response component. An
`Unsupported` projection, binding, next-key, or close component is fatal under
that future design only when replay or an authoritative compositor/default
obligation required that same component. An unrelated authoritative fact, such
as a title observation, does not poison a desired-only component failure; if
desired and authoritative causes jointly require one component, authoritative
provenance wins for that component. Deferred Session evidence is
`desired_projection_unsupported_with_unrelated_title_observation_is_repairable`,
`desired_projection_unsupported_with_authoritative_workarea_need_is_fatal`, and
`desired_binding_unsupported_with_authoritative_binding_need_is_fatal`. The MVP
does not branch on provenance: every post-admission `Unsupported` is fatal.

A backend may omit a requested placement only when that exact window identity
was successfully assigned in the current backend incarnation and was then
concurrently tombstoned. It must retain exactly one corresponding
`WindowClosed` behind that operation. A placement for an identity that was
never assigned in the current incarnation remains a contract failure. This
narrow exception makes a close arriving after one drain boundary and before the
next response safe without weakening the assigned-window invariant.

`poll_interest().immediate` means `service` must run before blocking in poll;
it is true for queued internal/public work and whenever no prepared-read guard
exists and one preparation attempt is eligible. A successful preparation
clears that reason for immediate service. Preparation that reports queued work
or no guard is itself progress and leaves another immediate quantum eligible.
`readable` means the outer poll set includes `POLLIN`; it remains true until the
terminal exit cutoff, then becomes false so superseded input cannot hot-loop.
`writable` means the outer poll set includes `POLLOUT`. `POLLNVAL` is an
immediate fatal backend-incarnation error and is never translated into ordinary
readable readiness. `POLLIN`, `POLLERR`, and `POLLHUP` map separately into
`BackendReady::readable` and `BackendReady::terminal` so a live prepared guard
is consumed and post-cutoff payload cannot masquerade as terminal completion.
Under [ADR 0021](adr/0021-bounded-wayland-ingress-and-dispatch.md),
one `service` invocation selects exactly one phase: dequeue one normalized
public event, dispatch at most one queued protocol event, admit one bounded
unit of socket ingress, attempt one flush, or attempt one read preparation. It
never calls a stock drain-all dispatch/read path, never waits, and returns at
most one public event. The ingress phase is exactly one nonblocking `recvmsg`
attempt with at most 16,384 bytes and ancillary capacity for 253 descriptors;
it does not retry, retains partial frames in at most 65,535 bytes of decoder
storage, and the backend owns at most 253 received descriptors across decoder,
queued protocol messages, and public-event staging. Byte/control truncation,
invalid framing, or either retention-cap overflow closes every descriptor still
owned by the failed ingress state exactly once and is fatal. `None`
after an internal phase is progress. A live
prepared-read guard is consumed or cancelled exactly once; no dispatch,
roundtrip, or second preparation occurs while it is live. Flush `WouldBlock`
sets writable interest and ends the quantum. A service error terminates the
backend incarnation and abandons any pending ticket; retryable operation errors
arrive as matching terminal events instead. After every quantum the outer
driver rechecks interest before it may block.

Eligible phases have fixed priority: pending output flush, one normalized
public dequeue, one queued protocol dispatch, one prepared readable/error/hangup
ingress attempt, then one read preparation. A flush previously returning
`WouldBlock` is ineligible until a later poll result supplies fresh writable
readiness. Simultaneous readable+writable readiness therefore flushes first;
continuous peer input cannot starve the finite output containing a sequence
finish or its `OperationCompleted`. Consumed readiness is never reused without
an interest recomputation and a fresh poll result.

The exit cutoff narrows this priority table. After
`begin_exit_session` succeeds, public dequeue, protocol dispatch, socket
ingress, and read preparation are permanently ineligible. `immediate` may then
name only finite internal neutral-finish or exit-flush work, `readable` is
false, and readable-only readiness performs no work. Writable progress may
flush the already ordered finishes and exit request; independent terminal
readiness may produce the sole `Disconnected` only after that output has fully
flushed.

Standalone focus is deliberately absent: projection focus is
`Placement::focused`. Close edges exist only inside a policy response because
they are manage-sequence state.
The outer #38 driver calls `Session::begin_shutdown` after the applicable exact
Quit response is Drained or Closed. The total idempotent Session transition
abandons every private candidate, origin, ticket, and effect without commit or
completion, preserves the consumed-ticket watermark, emits only repeat Disarm,
and enters `ShuttingDown`. It allocates no fresh ticket, requests no turn, and
answers no shutdown-discard turn. #38 owns response receipts, control draining,
the fixed shutdown deadline, worker fences, and the proof that it is safe to
authorize exit; Session owns no clock or receipt.

`Session::begin_exit_session` is terminal and nonblocking. #38 calls it exactly
once after those external conditions settle. Success enters
backend `Exiting` at one local cutoff: it cancels any prepared read, stops new
ingress admission, and suppresses public exposure of terminal
policy/completion events. A sequence whose turn was already consumed by
`respond_policy_turn` keeps every byte already emitted or queued and finishes
any manage/render phase already open or parsed at the cutoff exactly once from
that accepted immutable response, including a partially flushed phase. Its
public completion/drain is suppressed. A future render phase whose start is
still kernel-unread is abandoned and never replaced. Only an
admitted/open/queued sequence not yet exposed and answered is answered exactly
once with no projection/closes, the authorized committed enable/watch sets,
and next-key Cancel. Kernel-unread
bytes and incomplete decoder fragments are terminally superseded. Every
applicable open-phase finish is queued and flushed before the exit request is queued and
flushed. It permits no later `request_policy_turn` or `respond_policy_turn`.
Bounded `service` then returns only `None` until the one expected
`Disconnected`. Post-cutoff readable readiness cannot admit payload or produce
ExitComplete; terminal readiness is distinct and remains observable. Any other
exposed public event is a fatal backend-contract violation. The backend may emit
that success-path `Disconnected` only after
every applicable open-phase finish and every byte through the exit request has been flushed;
disconnect or I/O loss before that is a service error and fatal. The post-flush
disconnect completes logout and is not a restartable backend failure. The
backend-cutoff mechanics and evidence are #40-owned; the receipt/deadline/fence
and poll-loop handoff are #38-owned.

### Why river fits: realm *is* the window manager

river 0.4 removed window-management policy from the compositor entirely and
defers it to an external process over `river-window-management-v1`. realm is that
process. The protocol's vocabulary is close enough to the ledger's that the
backend is a translation rather than an approximation:

| realm concept | river request | Fidelity |
|---|---|---|
| Placement rectangle | `river_node_v1::set_position` + `river_window_v1::propose_dimensions` | **Approximate** — see the quantisation note below |
| Ledger order | *realm's own*, expressed through the positions it computes | Exact, because realm owns it outright |
| Stacking (mono occlusion, overlays) | `river_node_v1::place_top` / `place_bottom` / `place_above` / `place_below` | Exact |
| Stow | `river_window_v1::hide` / `show` — *rendering* state, so the window stays managed and stays in the ledger | Exact, and a closer match to `Orbit::stowed` than we expected |
| Focus | `river_seat_v1::focus_window` / `clear_focus` | Exact. Note `focus_exclusive` / `focus_non_exclusive` / `focus_none` are **events**, not requests: realm is told about exclusive focus, it does not grant it |
| 1px seams | `set_borders`, drawn by the compositor | Exact |
| Fullscreen | `fullscreen` / `exit_fullscreen` | Exact. Whether the bar draws over a fullscreen window is decided by the bar's chosen *layer*, not by node ordering: `river-layer-shell-v1` exposes no node and no ordering request at all |
| Window identity | `river_window_v1` `identifier` (up to 32 printable ASCII bytes) | **Requires a mapping.** `WindowOpened` carries `BackendWindowId`; realm-session restores or allocates a `WinId`, then calls `assign_window` before using that window. The never-reused property comes from realm's persisted counter, keyed by river's never-reused string |
| Workarea | `river_layer_shell_output_v1::non_exclusive_area` | Exact — arrives as an event. It is a free rectangle in global coordinates, *not* the `Workarea::new(w, h, top, bottom)` shape, so the backend converts |
| Atomic relayout | the `manage` **and** `render` sequences, in that order | Exact, but it is **two** phases and realm must respect the boundary — see below |

The backend selects the first complete `river_output_v1` in River creation
order and keeps it selected while present. It creates exactly one dependent
layer-shell output object per output and sends `set_default` for the selected
object in an applicable manage response. Additional outputs are retained but do
not split Realm's single workspace projection. Selected removal fails over to
the earliest surviving creation ordinal with complete geometry and retained
non-exclusive area, sends `set_default` for it, and exposes that output's
latest complete workarea. If no such output survives, the backend returns
`BackendError::Unavailable`.
Because River creates output objects inside the initial window-manager turn,
that turn cannot contain a layer-shell workarea. Session answers it without a
projection or enabled bindings and remains unready until the selected output's
later workarea turn has projected successfully.

Selected-default application is backend-private and is not expressible in
`BackendPolicyResponse`, so a Session fake cannot prove which `set_default`
request RiverBackend emitted. A failed response that attempted `set_default`
is fatal to the MVP Session and causes no neutral Session response, but the
production backend must leave that default dirty. A future same-incarnation
recovery response, including a neutral response, must therefore re-emit it when
applicable. This post-MVP cache evidence belongs to #40 as
`backend::tests::failed_selected_default_is_reemitted_by_next_applicable_response`.

M2 selects the first `river_seat_v1` in River creation order as its only Realm
seat. It alone receives one xkb-bindings-seat, one layer-shell-seat, all
registered binding objects, focus/chord requests, modifier observation, and
layer-shell exclusive-focus observation. Additional seats remain untouched.
No seat at the replay barrier or later selected-seat removal is
`BackendError::Unavailable`; if a manage turn is open, the backend answers it
neutrally before surfacing that failure. `configure_bindings` merely validates
and stores the mechanism specs until this replay-time selection exists.

Production River state is bounded independently of exposed policy turns:
`MAX_BACKEND_OUTPUTS = 16`, `MAX_BACKEND_SEATS = 16`,
`MAX_BACKEND_INPUT_DEVICES = 64`, and
`MAX_BACKEND_LIBINPUT_DEVICES = 64` count live objects, including incomplete
pre-`done` devices. Each object kind uses a checked nonzero `u64` creation
ordinal that never wraps or reuses a value in one backend incarnation. Before
installing a newly reported object or creating any dependent child, the backend
checks both its live-object cap and ordinal. Crossing a cap, exhausting an
ordinal, or exceeding bounded per-object enum/fixed-field state is fatal and
installs no map entry or child; the just-received proxy and every already-owned
child/descriptor are destroyed exactly once. Removal frees the live slot and
destroys unfinished policy state without reusing its ordinal. These bounds
apply during registry/input sync and between manage boundaries, so repeated
bounded service quanta cannot accumulate unbounded backend-local state.

**A placement spans both phases.** `propose_dimensions` is window-management
state; `set_position` is *rendering* state. So a single projection is not one
manage sequence: sizes go between `manage_start` and `manage_finish`, then
positions go after `render_start`.

The reason is a **data dependency**, not a prohibition. `propose_dimensions` is
manage-only; the resulting `dimensions` events arrive before `render_start`; and
a position cannot be finalised until then, because a window may not take the
size it was offered — the same quantisation problem flagged above. So realm keeps
positions in the render phase because that is where it first knows enough to
compute them.

(An earlier draft of this document asserted that a position submitted during the
manage phase raises `error::sequence_order`. The XML does not support that: the
`river_window_manager_v1` description permits rendering state to be modified
during *either* sequence and errors only outside both, and `set_position`'s own
text defers to that description by explicit cross-reference. The protocol is
arguably self-contradictory here; realm's behaviour is correct under either
reading, which is why the conclusion survived the correction.)

`place_*` orders the **render list**, not the ledger. The ledger is *layout*
order, which realm computes itself and expresses as positions; the `place_*`
requests exist for mono's occlusion stack and for overlay surfaces. Faithful
either way, but not for the reason a first reading suggests.

**The one genuinely approximate row.** `propose_dimensions` is a *proposal*: the
protocol explicitly anticipates clients quantising it, terminals to their cell
size being the named case. A terminal that rounds 700×580 down to 696×576 puts a
4×4 hole in a layout whose entire premise is exact tiling, and
`every_layout_tiles_exactly_for_every_plausible_size` would still pass while the
screen showed cracks — the test checks the projection, not what the client did
with it. river offers `set_content_clip_box`, which clips content to a rect and
draws borders around the intersection, so realm can propose at or above the tile
and clip to the exact rectangle. That is the plan; it is an M2 experiment with
its own guard, not a solved problem.

### What realm must implement, not merely call

Under River, a window manager is not only a client of the WM protocol. Realm's
MVP uses four companion protocols. The first is load-bearing; the last two make
the fixed keyboard-repeat and support-gated tap policy explicit without
guessing any other user preference:

| Protocol | What realm owes it | Consequence if unimplemented |
|---|---|---|
| `river-layer-shell-v1` | Serve layer-shell on river's behalf | **The bar does not appear at all.** `wlr-layer-shell` works under river only if the window manager implements it |
| `river-xkb-bindings-v1` | The entire keymap, **and key repeat for bound keys** | No keybinding works. `ensure_next_key_eaten` and `ate_unbound_key` (on `river_xkb_bindings_seat_v1`, reached via `get_seat`) are purpose-built for chorded submaps, which is exactly realm's chord model. `stop_repeat` establishes that repeat for bound keys is the window manager's job, so realm owns a second timer — armed only between `pressed` and `released`, which is the justification ADR 0009's no-timers rule requires |
| `river-input-management-v1` | Use the existing `default` seat and set 25 Hz / 600 ms application-key repeat after each keyboard's v2 `done`; do not change scroll factor, mapping, or seat assignment | Keyboard repeat otherwise depends on an unstated compositor default |
| `river-libinput-config-v1` | After each v2 `done`, enable tap-to-click only when support is positive and current state is disabled; preserve every other libinput setting | A supported laptop touchpad otherwise has no Realm-owned tap policy |

`river-xkb-config-v1` and runtime layout switching are post-MVP. Realm preserves
the River-created keymap/layout until a later Accepted contract defines the
layout source, action, persistence, state publication, and UI.

This is a materially larger phase-1 surface than "write a backend", and M2 is
scoped accordingly.

Two consequences worth stating plainly, because they cut both ways:

1. **A projection maps onto one manage/render transaction.** River applies sizes
   atomically between `manage_start` and `manage_finish`, then Realm applies
   positions and visibility before `render_finish`. The resulting relayout is
   never observed half-done.
2. **`realm-session` is now on the compositor's input path, with a hard liveness
   requirement.** Under niri, a crashed session daemon left a working if
   unmanaged desktop. Under river it leaves windows unplaced and keys dead, and
   the protocol warns that the compositor's input buffering is finite. River
   v0.4.8 queues 1024 seat events and then drops new input; it declares but does
   not post its `unresponsive` protocol error. **A stall is a session failure,
   not a slow frame.** Nothing in `realm-session` may block — not a theme apply,
   not a socket write to a wedged subscriber. This promotes the frame budgets
   in ARCHITECTURE §4 from performance goals to correctness requirements. See
   ADR 0013.

On stability: `river-window-management-v1` is **declared stable** as of river
0.4.0, with a forward-compatibility pledge to 1.0.0 — no `z` prefix, no
`unstable/` directory, interfaces already at v5. (An earlier draft of this
document called it registry-classified unstable. That was wrong: the
work-in-progress language came from a tracking issue that predates the release.)
The residual risk is not a protocol classification but trust in a single
maintainer of a pre-1.0 project, which is a different and smaller thing. realm
pins a tested river and treats a protocol bump as a tracked event.

ADR 0002 records the superseded plan to ship on niri, and the mapping table that
argued us out of it — worth reading before anyone proposes going back.

---

## 2. Theme generation contract — `realm-theme` (ADR 0005, ADR 0017)

One captured input set in, one sealed immutable generation selected for future
launches. [SPEC 0011](specs/0011-theme-activation-generations.md) supersedes the
former mutable target and reload interface for the supported path.

```rust
/// A file realm generates from the palette.
pub struct Template {
    /// Stable id, e.g. "gtk4", "foot", "yazi".
    pub id: &'static str,
    /// Source text with `{{ path.to.value }}` placeholders.
    pub source: &'static str,
    /// Normalized output path within a sealed generation.
    pub target: PathBuf,
    /// Catalogue metadata for a possible future generation-aware live upgrade.
    /// The supported apply path does not execute it.
    pub reload: Reload,
}

/// Canonical catalogue metadata. A current-pointer switch never executes this.
pub enum Reload {
    /// Catalogue declares that the consumer reads only at next start.
    None,
    /// Catalogue records a possible signal for a future owned-process protocol.
    Signal { process: &'static str, signal: i32 },
    /// Catalogue records a possible command for that future protocol.
    Command(Vec<String>),
    /// Catalogue identifies Realm-owned clients; apply sends no notification.
    RealmClients,
}

/// Publication result returned directly by the supported apply boundary.
pub enum GenerationPublicationOutcome {
    Committed(GenerationId),
    CommittedWithCleanupPending { generation: GenerationId, cause: String },
    OutcomeAmbiguous { candidate: GenerationId, cause: String },
}

/// A difference between candidate normalized outputs and a validated current
/// generation. Results are sorted by path and omit unchanged outputs.
pub enum ThemeOutputChange {
    Added(PathBuf),
    Removed(PathBuf),
    ByteDifferent(PathBuf),
}
```

The supported apply seam accepts safe input locators (or a test-only snapshot
builder), captures and renders them once under the exclusive generation lock,
and returns `GenerationPublicationOutcome` directly. It does not accept a
`Reloader`, inspect or write mutable target files, report `written` /
`unchanged` / `reloaded` lists, or notify a process. `Committed` and
`CommittedWithCleanupPending` identify the generation selected for future
launches; `OutcomeAmbiguous` is not reported as activated. Applying identical
inputs may publish a new generation; no no-op result is promised.
SPEC 0006 maps the variants deterministically: both committed variants are CLI
success (with a durable-selection cleanup warning for the latter), while
`OutcomeAmbiguous` is exit 6, names only an unconfirmed candidate, and cannot
trigger automatic recovery or retry.

The supported diff seam captures and renders the same inputs, then compares
their normalized output set with the manifest-listed bytes of a fully validated
`current` generation. It returns only sorted `Added`, `Removed`, and
`ByteDifferent` paths. It is read-only: no generated-root or lock
initialization, recovery, lease, GC, staging, publication, pointer replacement,
output write, signal, command, or session notification is permitted. Missing or
invalid current state is an error, not an empty baseline.

These names describe the public semantic contract, not a wire-compatibility
promise. Live upgrade and wire protocol design remain outside this interface.
Any future #22 upgrade must prove the selected generation of an owned process;
it cannot restore direct reload on pointer switch.

**Placeholder vocabulary.** Templates address the *derived* palette, so
`contrast` is already folded in and no template ever applies it itself:

| Form | Example | Yields |
|---|---|---|
| `{{ accent.violet }}` | | `#a692ec` |
| `{{ accent.violet.bare }}` | | `a692ec` |
| `{{ accent.violet.rgba(0.3) }}` | | `rgba(166, 146, 236, 0.3)` |
| `{{ accent.violet.over(background.pane, 0.3) }}` | | flattened hex, for formats without alpha |
| `{{ metrics.bar_height }}` | | `32` |
| `{{ typography.family }}` | | `IBM Plex Mono` |

An unknown placeholder is a hard error at render time, not an empty string. A
silently blank colour is exactly the bug this whole design exists to prevent.

---

## 3. Bar render contract — `realm-bar` (ADR 0008, 0009)

The bar is a pure function of `RealmState` plus the palette. It owns no state
beyond its Wayland surface.

```rust
/// Draw one frame. Called only when the state or the palette changed.
fn render(state: &RealmState, palette: &Palette, probe: &Probe, canvas: &mut Pixmap) -> Damage;

/// The region that actually changed, so the compositor is handed a damage
/// rectangle rather than a whole-surface repaint.
pub struct Damage(Option<Rect>);
```

Rules, enforced by review and by the budgets in ARCHITECTURE.md §4:

1. **The bar owns no connected render, animation or module timer.** Every value
   it draws arrives in `RealmState`. Four of the mockup's modules — cpu, mem, gpu temperature and the
   `↑ 18k ↓ 1.2M` throughput half of net — are *rates over counters*, and the
   kernel exposes no event for those; no bar on any platform gets them without
   sampling. So the sampling lives in **one shared sampler in `realm-session`**,
   off the window-management event loop, and is the single documented exception
   to ADR 0009's no-timers rule. The bar stays a pure function of state, which
   is the property that actually mattered.
   SPEC 0004's disconnected-only control retry is a transport-liveness wakeup,
   is disarmed after subscription and never renders. The clock ticks to the
   next **minute** boundary, not every second: the design
   shows `14:32`, so 59 of every 60 wakeups would redraw nothing.
2. **No redraw when nothing changed.** `RealmState::renders_same_as` gates the
   frame before any drawing happens.
3. **Every glyph goes through `Probe::resolve`.** Drawing a raw `char` from the
   inventory bypasses the fallback contract and is how tofu ships.
4. **Damage, not repaint.** A clock tick must damage the clock, not the bar.

---

## 4. Control endpoint and client — `realm-control` (ADR 0004)

```rust
/// Validated Linux path capabilities. Their fields and owned descriptors are
/// private; accessors return display paths or borrows only.
pub struct RuntimeDir(/* absolute display path, retained fd, daemon euid */);
pub struct RealmDir(/* retained descriptor; exposes a borrowed fd */);
pub struct SocketEndpoint(/* exact ctl.sock display path + RealmDir */);
pub struct BoundControlEndpoint(/* non-listening fd + lock + path identity */);
pub struct ActiveControlListener(/* listening fd + lock + path identity */);
pub struct ControlServer(/* consumed listener + bounded connection state */);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u64); // private opaque value, never a raw fd
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResponseReceipt {
    connection: ConnectionId,
    sequence: std::num::NonZeroU64,
}
pub enum ResponseSettlement {
    Pending,
    Drained,
    Closed,
}
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlToken(u64); // private opaque poll-registration identity
pub struct PollInterest<'a> {
    pub token: ControlToken,
    pub fd: BorrowedFd<'a>,
    pub readable: bool,
    pub writable: bool,
}
pub struct ReadyEvent {
    pub token: ControlToken,
    /// POLLIN only.
    pub readable: bool,
    /// POLLERR or POLLHUP, reported even when readable interest is masked.
    pub terminal: bool,
    pub writable: bool,
}
pub enum ControlAction {
    Request { connection: ConnectionId, request: Request },
}
pub enum ControlError {
    StaleConnection { connection: ConnectionId },
    ShuttingDown,
    OutboundFrameTooLarge { connections: Vec<ConnectionId> }, // at most 64, ascending
    ResponseSequenceExhausted { connection: ConnectionId },
    ResponseBarrierConflict {
        active: ResponseReceipt,
        requested: ResponseReceipt,
    },
    PeerIo { connection: ConnectionId, source: std::io::Error },
    ListenerIo(std::io::Error),
    ListenerTerminal,
    ResourceExhausted(std::io::Error),
}
pub struct ClientEndpoint(/* retained RuntimeDir for retry */);
pub struct Client(/* connected transport wrapper added by #41 */);
pub struct Subscription(/* consuming event iterator added by #41 */);

pub enum ClientPhase {
    Connect,
    HelloWrite,
    HelloRead,
    RequestWrite,
    ResponseRead,
    SubscribeWrite,
    InitialState,
    SubscriptionEvent,
}

pub enum ClientError {
    MissingRealm,
    Refused,
    Path(IpcPathError),
    VersionMismatch { client: u32, server: u32 },
    Timeout { phase: ClientPhase },
    FrameTooLarge { phase: ClientPhase },
    InvalidRequest,
    UnexpectedResponse { phase: ClientPhase },
    MalformedResponse { phase: ClientPhase },
    Eof { phase: ClientPhase },
    Io { phase: ClientPhase, source: std::io::Error },
}

pub enum IpcPathError {
    MissingRuntimeDir,
    UnsafeRuntimeDir,
    UnsafeRealmDirectory,
    UnsafeSocketEntry,
    EndpointInUse,
    Io(std::io::Error),
}

pub trait RuntimeDirResolver {
    fn resolve(&self) -> Result<RuntimeDir, IpcPathError>;
}

pub fn production_runtime_dir() -> Result<RuntimeDir, IpcPathError>;
pub fn test_runtime_dir(path: &Path) -> Result<RuntimeDir, IpcPathError>;

impl RuntimeDir {
    pub fn path(&self) -> &Path;
    pub fn prepare_server_endpoint(self) -> Result<SocketEndpoint, IpcPathError>;
    pub fn client_endpoint(self) -> ClientEndpoint;
}

impl RealmDir {
    /// Used later for descriptor-relative ledger.json work.
    pub fn as_fd(&self) -> BorrowedFd<'_>;
}

impl SocketEndpoint {
    pub fn path(&self) -> &Path;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn bind(self) -> Result<BoundControlEndpoint, IpcPathError>;
}

impl BoundControlEndpoint {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn activate(self) -> Result<ActiveControlListener, IpcPathError>;
}

impl ActiveControlListener {
    pub fn endpoint(&self) -> &SocketEndpoint;
    pub fn realm_dir(&self) -> &RealmDir;
    pub fn into_server(self, now: Instant) -> ControlServer;
}

impl ControlServer {
    pub fn poll_interests(&self) -> impl Iterator<Item = PollInterest<'_>>;
    pub fn next_deadline(&self) -> Option<Instant>;
    pub fn service_one(&mut self, now: Instant, ready: ReadyEvent)
        -> Result<Option<ControlAction>, ControlError>;
    pub fn expire(&mut self, now: Instant) -> Result<(), ControlError>;
    pub fn complete_request(&mut self, now: Instant, connection: ConnectionId,
        response: Response) -> Result<ResponseReceipt, ControlError>;
    /// Read-only and nonblocking. Closed includes removal after reset, EOF, or
    /// deadline; Drained names this exact response generation.
    pub fn response_settlement(&self, receipt: ResponseReceipt)
        -> ResponseSettlement;
    /// If Pending, atomically suppress listener/all peer reads and expose only
    /// this response's write/terminal progress until it settles. Repeating the
    /// same barrier is idempotent; a distinct pending barrier is a contract
    /// error. Drained/Closed installs nothing.
    pub fn begin_response_barrier(&mut self, receipt: ResponseReceipt)
        -> Result<ResponseSettlement, ControlError>;
    pub fn complete_subscribe(&mut self, now: Instant, connection: ConnectionId,
        state: RealmState) -> Result<(), ControlError>;
    pub fn publish_state(&mut self, now: Instant, state: RealmState)
        -> Result<(), ControlError>;
    pub fn begin_shutdown(&mut self, now: Instant);
    pub fn is_shutdown_complete(&self) -> bool;
}

/// A connection to realm-session, added with the #41 transport slice.
impl Client {
    pub fn request(&mut self, req: Request) -> Result<Response, ClientError>;
    /// Subscribe after a successful Hello; yields an immediate state snapshot
    /// and coalesced later changes until Shutdown/EOF/error. No further request
    /// is valid.
    pub fn subscribe(self) -> Result<Subscription, ClientError>;
}

impl ClientEndpoint {
    /// Make exactly one descriptor-relative attempt through a generated procfd
    /// bridge and complete Hello using the explicit client name.
    pub fn connect(&self, client: &str) -> Result<Client, ClientError>;
}

impl Iterator for Subscription {
    type Item = Result<Event, ClientError>;
}
```

`realm-control` is a new Linux-only shared workspace library; non-Linux
compilation fails explicitly. `realm-core` remains portable and owns only wire
values plus encode/decode/version. `BoundControlEndpoint` is not `Clone` and
intentionally has no `AsFd`; `activate(self)` is the only public path to
`listen(..., 64)`. It verifies `SO_ACCEPTCONN` before returning the not-`Clone`
active wrapper. The active wrapper is then consumed into `ControlServer` and
does not implement public `AsFd`. `poll_interests` exposes only temporary
borrowed fds paired with stable `ControlToken`s; callers copy token/readiness
and drop every borrow before `service_one`. Raw fds are never connection
identities.

Bound, active, and server ownership retain a separately opened singleton-lock
description and the no-follow pathname identity used for ownership-safe Drop
cleanup. That private
lock fd is opened independently with
`O_RDONLY | O_DIRECTORY | O_CLOEXEC` (or an exact equivalent), is checked for
`FD_CLOEXEC`, and is never returned by an accessor or `AsFd`. An exec-launched
client therefore cannot retain singleton ownership after the daemon exits.

Linux has no `bindat` or `connectat`. Bind, stale-probe connect, and
shared-client connect use only an internally generated
`/proc/self/fd/<realm-dir-fd>/ctl.sock` address; missing procfs fails closed.
The canonical address and a worst-case procfd address, including terminating
NULs, are checked against `sockaddr_un` and procfs accessibility is proved
before server-side filesystem mutation. The canonical public path is a
display/external-client path, not a shared-crate resolution route.
`ClientEndpoint` retains the validated runtime capability and reopens `realm`
relative to it on every single connection attempt; it never rereads
`XDG_RUNTIME_DIR` or creates the directory. An absent `realm` is the retryable
`MissingRealm` classification, not `UnsafeRealmDirectory`. `realmctl`, not
`ClientEndpoint`, owns the six absolute not-before retry targets from one fixed
driver start. A late retryable attempt skips elapsed sleep; attempts never
overlap or move backward. Server methods use caller-supplied
`std::time::Instant`; no production transport clock trait is part of this
interface. SPEC 0007 is the complete construction, singleton ownership,
transport, deadline, shutdown, and cleanup contract.

The wire types (`Request`, `Response`, `Event`, `RealmState`) already exist in
`realm-core::ipc` and `realm-core::state` and remain normative. `ControlAction`
emits every decoded non-Hello request without implementing session semantics;
the #38 adapter owns completions and authoritative state. The transport's
65,536-byte bound includes LF on input and output. One-peer oversized completion
closes that peer; oversized `publish_state` encodes once, closes all and only
current subscribers, and reports their at-most-64 stable ids in ascending
order. `service_one` applies due deadlines before socket I/O, disables reads
while application completion or ordinary output is pending, and gives
simultaneous subscriber readability priority over writability.
After an ordinary application response drains, including `Response::Error`,
only a read-open peer returns to `Ready`; a read-half-closed peer closes.
Successful `complete_request` allocates one nonzero per-connection response
generation and returns its opaque receipt. `response_settlement` performs no
I/O: it reports Pending while that exact frame is queued/partial, Drained after
its last byte was sent, and Closed after EOF, reset, deadline, or other removal.
Later requests cannot make an older receipt pending again. Generation exhaustion
closes that connection and returns `ResponseSequenceExhausted`; it never wraps. An
oversized requester completion closes the connection and returns
`OutboundFrameTooLarge` without a receipt, which is already a Closed outcome for
the adapter's Quit barrier.

`ResponseReceipt`, response settlement, both fixed shutdown deadlines, and
abandonment decisions are #38 driver/transport concerns. Session stores no
receipt, reads no clock, and proves no deadline behavior. Its only MVP handoff
is the closed effect/result that tells #38 which exact response must settle
before #38 externally calls `begin_shutdown`; #38 later authorizes
`begin_exit_session` after its own control and worker conditions settle.

The outer poll mapping keeps input and terminal readiness distinct: `POLLIN`
sets `ReadyEvent::readable`, while `POLLERR | POLLHUP` sets `terminal` even when
the corresponding `PollInterest::readable` is false. `POLLNVAL` is an invalid
registration and never masquerades as input readiness.

`begin_response_barrier` makes the pre-shutdown QuitPending boundary a transport
state, not a caller convention. For a Pending receipt it removes listener and
all peer readable interests and exposes only that exact connection's
writable/terminal progress. `service_one` cannot decode or emit an action in
this state. Terminal readiness closes the barrier peer immediately and makes
its receipt Closed, including HUP-only; otherwise simultaneous input+writable
readiness ignores input and advances the queued response. `next_deadline` names only that response's existing
deadline. The same receipt is idempotent, a distinct concurrent Pending receipt
is fatal process/adapter contract failure `ResponseBarrierConflict` and leaves
the first barrier unchanged; it is never peer-local recovery. Drained/Closed installs no barrier. Once the
receipt settles, no ordinary interests reappear; #38 immediately calls
`begin_shutdown`, which supersedes the barrier.

`begin_shutdown` is total and idempotent: its first call fixes the hard 100 ms
deadline, stops admission/actions, closes every non-subscriber, and leaves only
bounded subscriber drain. An unstarted initial State is preserved before
Shutdown; a partial current frame finishes alone. #38 starts the worker seal
and its fixed 2,000 ms deadline when the adapter first observes the terminal
`QuitPending` boundary, before waiting for a response barrier or beginning the
control drain. It then drives interests, expiry, and the bounded
worker-fence/result path until control completion plus fence acknowledgement or
explicit degradation at that unchanged deadline, then calls
`Session::begin_exit_session()` exactly once and drives backend interest to
the one `BackendTurn::ExitComplete` produced by the expected post-flush
`BackendEvent::Disconnected`. No new connection or action is
admitted after shutdown starts. Later completion or publication returns
`ShuttingDown` without mutation, although an id already stale before shutdown
may remain `StaleConnection`. The `Client`,
`Subscription`, and `ControlServer` declarations describe the #41 transport
slice, not a claim that #218's endpoint slice implements it.

---

## 5. Activation lifecycle authority

Accepted [SPEC 0012](specs/0012-activation-launch-lifecycle.md) keeps lifecycle
selection, ownership evidence, lease transfer/release, durable state
transitions, and execution-gate authority private to `realm-theme`'s lifecycle
owner.  No `GenerationSelection` lifecycle-transfer method, lease reference, or
caller-constructed `LifecycleOwner` is a public interface.  The planned
fresh-Exec desktop-launch boundary is the consuming high-level facade constrained by
[SPEC 0013](specs/0013-truthful-fresh-desktop-exec.md): it accepts an immutable
admitted fresh-Exec plan and exposes neither an internal lifecycle capability
nor a public wire protocol.  Public request/response/history design remains
SPEC 0006/#117.  The private implementation still has the accepted
consume-on-transfer, no-drop-release, and proof-before-release obligations;
their concrete Rust types are not external compatibility interfaces.

---

## 6. What is deliberately *not* an interface

- **The ledger.** There is one implementation and there will only ever be one.
  Making it a trait would invite a second source of truth, which is the exact
  failure ADR 0001 exists to prevent.
- **The layout projection.** Same reason: layouts are an enum with a pure
  function, not a plugin surface. A layout that cannot be expressed as
  `fn(&Ledger, Workarea) -> Vec<Placement>` is a layout realm does not want.
