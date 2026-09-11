//! Compositor-independent window-management contract.

use std::num::{NonZeroU32, NonZeroU64, TryFromIntError};
use std::os::fd::BorrowedFd;
use std::time::Instant;

use realm_core::ipc::Capabilities;
use realm_core::layout::{Placement, Rect, Workarea};
use realm_core::WinId;
use serde::{Deserialize, Deserializer, Serialize};

/// Result returned by compositor backend operations.
pub type BackendResult<T> = std::result::Result<T, BackendError>;

/// Stable compositor-owned identity used to recover Realm window ids.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct BackendWindowId(String);

impl BackendWindowId {
    /// Construct an identity from 1..=32 printable ASCII bytes.
    pub fn new(text: impl Into<String>) -> Option<Self> {
        let text = text.into();
        if (1..=32).contains(&text.len()) && text.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        {
            Some(Self(text))
        } else {
            None
        }
    }

    /// Return the validated identity text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BackendWindowId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let text = String::deserialize(deserializer)?;
        Self::new(text).ok_or_else(|| {
            serde::de::Error::custom("backend window identity must be 1..=32 printable ASCII bytes")
        })
    }
}

macro_rules! nonzero_backend_id {
    ($name:ident, $raw:ty, $nonzero:ty) => {
        impl $name {
            /// Construct the opaque identity, rejecting zero.
            pub fn new(raw: $raw) -> Option<Self> {
                <$nonzero>::new(raw).map(Self)
            }

            /// Return the nonzero integer identity.
            pub fn get(self) -> $raw {
                self.0.get()
            }
        }

        impl TryFrom<$raw> for $name {
            type Error = TryFromIntError;

            fn try_from(raw: $raw) -> Result<Self, Self::Error> {
                <$nonzero>::try_from(raw).map(Self)
            }
        }
    };
}

/// Opaque nonzero identity allocated once by Session for one backend operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendTicket(NonZeroU64);

nonzero_backend_id!(BackendTicket, u64, NonZeroU64);

/// Opaque nonzero identity of one compositor policy boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendPolicyTurnId(NonZeroU64);

nonzero_backend_id!(BackendPolicyTurnId, u64, NonZeroU64);

/// Stable Session-selected identity for one configured binding mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BackendBindingId(NonZeroU32);

nonzero_backend_id!(BackendBindingId, u32, NonZeroU32);

/// Modifier names used by compositor binding mechanisms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BackendModifier {
    /// Shift.
    Shift,
    /// Control.
    Control,
    /// Alt.
    Alt,
    /// Super/logo.
    Super,
}

/// Maximum number of managed windows.
pub const MAX_MANAGED_WINDOWS: usize = 256;
/// Maximum number of configured binding mechanisms.
pub const MAX_CONFIGURED_BINDINGS: usize = 64;
/// Maximum number of facts in one live or draining policy turn.
pub const MAX_POLICY_EVENTS: usize = 256;
/// Maximum number of staged effects across one transaction.
pub const MAX_STAGED_EFFECTS: usize = 256;
/// Maximum total UTF-8 payload bytes in one policy turn.
pub const MAX_POLICY_TEXT_BYTES: usize = 65_535;
/// Maximum live production-backend output objects.
pub const MAX_BACKEND_OUTPUTS: usize = 16;
/// Maximum live production-backend seat objects.
pub const MAX_BACKEND_SEATS: usize = 16;
/// Maximum live production-backend input device objects.
pub const MAX_BACKEND_INPUT_DEVICES: usize = 64;
/// Maximum live production-backend libinput device objects.
pub const MAX_BACKEND_LIBINPUT_DEVICES: usize = 64;
/// Maximum JSON-encoded content bytes for a visible application id.
pub const MAX_VISIBLE_APP_ID_JSON_BYTES: usize = 40;
/// Maximum JSON-encoded content bytes for a visible window title.
pub const MAX_VISIBLE_TITLE_JSON_BYTES: usize = 80;
/// Key-repeat rate in hertz.
pub const KEY_REPEAT_RATE_HZ: u32 = 25;
/// Initial key-repeat delay in milliseconds.
pub const KEY_REPEAT_DELAY_MS: u64 = 600;
/// Maximum admitted process jobs, excluding the replaceable snapshot slot.
pub const MAX_WORKER_JOBS: usize = 256;
/// Maximum queued worker results.
pub const MAX_WORKER_RESULTS: usize = 256;
/// Maximum retained snapshot bytes.
pub const MAX_SNAPSHOT_BYTES: usize = 65_535;
/// Degraded-shutdown worker fence timeout in milliseconds.
pub const WORKER_SHUTDOWN_TIMEOUT_MS: u64 = 2_000;
/// Maximum number of events in the one initial replay turn.
pub const MAX_REPLAY_POLICY_EVENTS: usize = MAX_MANAGED_WINDOWS + 3;

/// Worker queue whose accepted capacity was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerCapacityResource {
    /// Submitted jobs.
    Jobs,
    /// Completed results.
    Results,
}

/// A bounded worker queue reached its accepted capacity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("worker capacity exceeded for {resource:?}: limit {limit}")]
pub struct WorkerCapacityError {
    /// Queue whose capacity was exceeded.
    pub resource: WorkerCapacityResource,
    /// Accepted queue limit.
    pub limit: usize,
}

/// One configured binding mechanism, without Realm policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendBindingSpec {
    /// Stable mechanism identity.
    pub id: BackendBindingId,
    /// Canonical xkbcommon keysym name.
    pub keysym: String,
    /// Canonical sorted unique modifiers.
    pub modifiers: Vec<BackendModifier>,
}

/// One fact delivered before a compositor policy boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendPolicyEvent {
    /// The initial replay is complete.
    InitialReplayComplete,
    /// A new window became manageable.
    WindowOpened {
        /// Stable compositor identity.
        backend_id: BackendWindowId,
        /// Application identifier.
        app_id: String,
        /// Window title.
        title: String,
    },
    /// A managed window closed.
    WindowClosed(BackendWindowId),
    /// A managed window title changed.
    TitleChanged {
        /// Stable compositor identity.
        backend_id: BackendWindowId,
        /// New title.
        title: String,
    },
    /// Compositor keyboard focus changed.
    FocusChanged(Option<BackendWindowId>),
    /// Exclusive layer focus changed.
    ExclusiveFocusChanged(bool),
    /// The selected output workarea changed.
    WorkareaChanged(Workarea),
    /// The compositor moved a window independently.
    GeometryDrifted {
        /// Stable compositor identity.
        backend_id: BackendWindowId,
        /// Observed rectangle.
        rect: Rect,
    },
    /// A configured binding was pressed.
    BindingPressed(BackendBindingId),
    /// A configured binding was released.
    BindingReleased(BackendBindingId),
    /// The compositor stopped repeating a binding.
    BindingRepeatStopped(BackendBindingId),
    /// An ensured otherwise-unbound key was consumed.
    UnboundKeyEaten,
    /// Watched modifier state changed.
    ModifiersChanged {
        /// Previous canonical modifier state.
        old: Vec<BackendModifier>,
        /// New canonical modifier state.
        new: Vec<BackendModifier>,
    },
}

/// Complete report-order batch preceding one mandatory compositor response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendPolicyTurn {
    /// Backend-allocated turn identity.
    pub id: BackendPolicyTurnId,
    /// Operation whose separate empty drain marker this turn replaces.
    pub drains: Option<BackendTicket>,
    /// Ordered policy facts.
    pub events: Vec<BackendPolicyEvent>,
}

/// One-shot next-key policy edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendNextKeyEdge {
    /// Preserve current compositor state.
    Preserve,
    /// Ensure the next otherwise-unbound key is consumed.
    Ensure,
    /// Cancel a prior ensure operation.
    Cancel,
}

/// Complete desired binding state for one response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendBindingState {
    /// Complete canonical sorted unique enabled mechanisms.
    pub enabled: Vec<BackendBindingId>,
    /// Complete canonical sorted unique watched modifiers.
    pub watched_modifiers: Vec<BackendModifier>,
    /// Explicit one-shot next-key edge.
    pub next_key_edge: BackendNextKeyEdge,
}

/// Session's complete answer to one offered policy turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendPolicyResponse {
    /// `None` preserves a clean projection; `Some(empty)` hides every window.
    pub projection: Option<Vec<Placement>>,
    /// Unique close edges in first-occurrence report order.
    pub closes: Vec<WinId>,
    /// Complete binding policy.
    pub bindings: BackendBindingState,
}

/// Fixed Session-authorized policy for terminal exit sequences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendExitPolicy {
    /// Complete canonical sorted unique enabled mechanisms.
    pub enabled: Vec<BackendBindingId>,
    /// Complete canonical sorted unique watched modifiers.
    pub watched_modifiers: Vec<BackendModifier>,
}

/// Result of one nonblocking backend submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSubmission {
    /// The response and its admitted observation epoch are final.
    Complete,
    /// Exactly one matching terminal event will arrive later.
    Pending,
}

/// Dynamic poll interest for the backend's descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendPollInterest {
    /// Internal work is already queued.
    pub immediate: bool,
    /// Read readiness is useful.
    pub readable: bool,
    /// A protocol flush needs write readiness.
    pub writable: bool,
}

/// Readiness reported by the outer event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendReady {
    /// Read readiness was observed while enabled.
    pub readable: bool,
    /// Error or hangup readiness was observed.
    pub terminal: bool,
    /// Write readiness was observed.
    pub writable: bool,
}

/// Closed backend protocol failure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendProtocolErrorKind {
    /// A payload ended before its declared boundary.
    TruncatedPayload,
    /// Ancillary data was truncated.
    TruncatedAncillary,
    /// Transport framing was invalid.
    InvalidFraming,
    /// Protocol objects arrived in an invalid order.
    InvalidObjectOrder,
}

/// Closed backend capacity resource classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendCapacityResource {
    /// Managed windows.
    ManagedWindows,
    /// Configured bindings.
    ConfiguredBindings,
    /// Output objects.
    Outputs,
    /// Seat objects.
    Seats,
    /// Input device objects.
    InputDevices,
    /// Libinput device objects.
    LibinputDevices,
    /// Never-reused object ordinals.
    ObjectOrdinals,
    /// Policy facts.
    PolicyFacts,
    /// Transaction policy effects.
    PolicyEffects,
    /// Policy text bytes.
    PolicyTextBytes,
    /// Retained transport bytes.
    RetainedBytes,
    /// Retained transport descriptors.
    RetainedDescriptors,
    /// Policy-turn identities.
    PolicyTurnIds,
}

/// A compositor backend failure with enough structure to report it honestly.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// The backend cannot honour the named Realm capability.
    #[error("backend cannot honour capability {capability}")]
    Unsupported {
        /// Stable capability name, also present in `Capabilities::unsupported`.
        capability: String,
    },
    /// The compositor connection was lost after it had been established.
    #[error("backend disconnected")]
    Disconnected,
    /// The backend could not become the active window manager.
    #[error("backend unavailable: {message}")]
    Unavailable {
        /// Human-readable refusal reason from the backend.
        message: String,
    },
    /// The backend transport failed without a clean disconnect.
    #[error("backend I/O failed: {message}")]
    Io {
        /// Human-readable transport failure.
        message: String,
    },
    /// The backend transport violated its closed protocol contract.
    #[error("backend protocol violation: {kind:?}")]
    Protocol {
        /// Machine-readable protocol failure class.
        kind: BackendProtocolErrorKind,
    },
    /// The backend exceeded an accepted finite capacity.
    #[error("backend capacity exceeded for {resource:?}: limit {limit}")]
    Capacity {
        /// Machine-readable exhausted resource.
        resource: BackendCapacityResource,
        /// Accepted limit for the resource.
        limit: u64,
    },
}

/// Local identity-map or exposed policy contract violation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendContractError {
    /// Assignment named a backend window not live in this incarnation.
    #[error("backend window is not live in this incarnation: {backend_id:?}")]
    UnknownWindow {
        /// Unknown compositor identity.
        backend_id: BackendWindowId,
    },
    /// A backend identity already has a different Realm assignment.
    #[error("backend identity already has a different Realm assignment")]
    ConflictingBackendIdentity,
    /// A Realm window already has a different backend identity.
    #[error("Realm window id already has a different backend identity")]
    ConflictingRealmIdentity,
    /// A policy fact referenced a window before a current-incarnation open.
    #[error("backend identity was referenced before current-incarnation open")]
    UnknownWindowReference,
    /// A policy fact named an unconfigured binding.
    #[error("backend reported an unconfigured binding id: {id:?}")]
    UnknownBinding {
        /// Unconfigured mechanism identity.
        id: BackendBindingId,
    },
    /// A modifier vector was not a canonical sorted unique subset.
    #[error("backend modifier vectors are not canonical sorted unique subsets")]
    NonCanonicalModifiers,
    /// A policy transaction sequence was invalid.
    #[error("backend emitted an invalid policy transaction sequence")]
    InvalidPolicySequence,
}

/// Something the compositor did that the authoritative ledger must reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    /// One complete report-order policy boundary awaiting a response.
    PolicyTurn(BackendPolicyTurn),
    /// The one terminal result for a pending backend operation.
    OperationCompleted {
        /// Operation identity.
        ticket: BackendTicket,
        /// Terminal operation result.
        result: BackendResult<()>,
    },
    /// No policy facts were retained behind the completed operation.
    RetainedObservationsDrained {
        /// Completed operation identity.
        ticket: BackendTicket,
    },
    /// The backend has reported every window present at connection time.
    InitialReplayComplete,
    /// A new window became manageable.
    WindowOpened {
        /// Stable compositor identity used for restart reconciliation.
        backend_id: BackendWindowId,
        /// Application identifier reported by the compositor.
        app_id: String,
        /// Window title reported by the compositor.
        title: String,
    },
    /// A managed window closed.
    WindowClosed(WinId),
    /// A managed window changed its title.
    TitleChanged {
        /// Window whose title changed.
        win: WinId,
        /// New title.
        title: String,
    },
    /// The compositor's current keyboard focus changed.
    FocusChanged(Option<WinId>),
    /// A layer surface acquired or released exclusive keyboard focus.
    ExclusiveFocusChanged(bool),
    /// The output workarea available for projection changed.
    WorkareaChanged(Workarea),
    /// The compositor moved a window independently.
    ///
    /// This is advisory. The ledger remains authoritative and the next
    /// projection must overrule this rectangle.
    GeometryDrifted {
        /// Window moved by the compositor.
        win: WinId,
        /// Rectangle observed from the compositor.
        rect: Rect,
    },
    /// The compositor connection ended.
    Disconnected,
}

/// Compositor seam driven by the Realm session daemon.
pub trait WmBackend: Send {
    /// Human-readable backend name shown by `realmctl doctor`.
    fn name(&self) -> &str;

    /// Connect and report what the backend can honour.
    fn connect(&mut self) -> BackendResult<Capabilities>;

    /// Bind a stable backend identity to Realm's allocated window id.
    ///
    /// This is a local, bounded, nonblocking identity-map installation and
    /// emits no compositor request. Repeating the same pair is a no-op. An
    /// unknown object or conflicting pair is a fatal backend-contract error;
    /// it is never retryable while the compositor waits for a policy response.
    /// The binding must succeed before the response can include the window.
    fn assign_window(
        &mut self,
        backend_id: &BackendWindowId,
        win: WinId,
    ) -> Result<(), BackendContractError>;

    /// Register bounded binding mechanisms without assigning Realm policy.
    fn configure_bindings(&mut self, bindings: Vec<BackendBindingSpec>) -> BackendResult<()>;

    /// Idempotently request a future policy turn without blocking.
    fn request_policy_turn(&mut self) -> BackendResult<()>;

    /// Consume one offered turn and submit Session's complete answer.
    fn respond_policy_turn(
        &mut self,
        turn: BackendPolicyTurnId,
        ticket: BackendTicket,
        response: BackendPolicyResponse,
    ) -> BackendResult<BackendSubmission>;

    /// Begin the terminal compositor exit sequence.
    fn begin_exit_session(&mut self, policy: BackendExitPolicy) -> BackendResult<()>;

    /// Apply the complete visible projection when it changed or a prior apply failed.
    ///
    /// While no error intervenes, implementations are idempotent: submitting
    /// identical placements twice produces no visible change and no second
    /// frame. Before returning an error, an implementation invalidates every
    /// projection, diff, per-window and request cache. The next call must issue
    /// the complete requested projection even when it equals the last
    /// successful projection. Only success restores cache validity.
    #[deprecated(note = "temporary compatibility seam; use respond_policy_turn")]
    fn apply(&mut self, placements: &[Placement]) -> BackendResult<()>;

    /// Give a window keyboard focus.
    #[deprecated(note = "temporary compatibility seam; use respond_policy_turn")]
    fn focus(&mut self, win: WinId) -> BackendResult<()>;

    /// Ask a window to close politely.
    #[deprecated(note = "temporary compatibility seam; use respond_policy_turn")]
    fn close(&mut self, win: WinId) -> BackendResult<()>;

    /// Return the current projection workarea.
    #[deprecated(note = "temporary compatibility seam; consume policy turns")]
    fn workarea(&self) -> Workarea;

    /// Return the backend descriptor used by the session poll set.
    fn event_fd(&self) -> BorrowedFd<'_>;

    /// Return queued-work and dynamic descriptor interest without side effects.
    fn poll_interest(&self) -> BackendPollInterest;

    /// Perform one bounded nonblocking protocol quantum and expose at most one event.
    fn service(&mut self, ready: BackendReady, now: Instant)
        -> BackendResult<Option<BackendEvent>>;

    /// Wait for the next compositor event, bounded by an optional deadline.
    #[deprecated(note = "temporary compatibility seam; use service")]
    fn next_event(&mut self, deadline: Option<Instant>) -> BackendResult<Option<BackendEvent>>;
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs::File;
    use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{Placement, Workarea};
    use realm_core::WinId;

    use super::{
        BackendBindingId, BackendBindingSpec, BackendBindingState, BackendCapacityResource,
        BackendContractError, BackendError, BackendEvent, BackendExitPolicy, BackendModifier,
        BackendNextKeyEdge, BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn,
        BackendPolicyTurnId, BackendPollInterest, BackendProtocolErrorKind, BackendReady,
        BackendResult, BackendSubmission, BackendTicket, BackendWindowId, WmBackend,
        KEY_REPEAT_DELAY_MS, KEY_REPEAT_RATE_HZ, MAX_BACKEND_INPUT_DEVICES,
        MAX_BACKEND_LIBINPUT_DEVICES, MAX_BACKEND_OUTPUTS, MAX_BACKEND_SEATS,
        MAX_CONFIGURED_BINDINGS, MAX_MANAGED_WINDOWS, MAX_POLICY_EVENTS, MAX_POLICY_TEXT_BYTES,
        MAX_REPLAY_POLICY_EVENTS, MAX_SNAPSHOT_BYTES, MAX_STAGED_EFFECTS,
        MAX_VISIBLE_APP_ID_JSON_BYTES, MAX_VISIBLE_TITLE_JSON_BYTES, MAX_WORKER_JOBS,
        MAX_WORKER_RESULTS, WORKER_SHUTDOWN_TIMEOUT_MS,
    };

    struct ContractBackend {
        assigned: Option<(BackendWindowId, WinId)>,
        event_file: File,
        events: VecDeque<BackendEvent>,
    }

    impl Default for ContractBackend {
        fn default() -> Self {
            let turn = BackendPolicyTurn {
                id: BackendPolicyTurnId::new(7).unwrap(),
                drains: None,
                events: vec![BackendPolicyEvent::InitialReplayComplete],
            };
            let ticket = BackendTicket::new(11).unwrap();
            Self {
                assigned: None,
                event_file: File::open("/dev/null").unwrap(),
                events: VecDeque::from([
                    BackendEvent::PolicyTurn(turn),
                    BackendEvent::OperationCompleted {
                        ticket,
                        result: Ok(()),
                    },
                    BackendEvent::RetainedObservationsDrained { ticket },
                    BackendEvent::Disconnected,
                ]),
            }
        }
    }

    impl WmBackend for ContractBackend {
        fn name(&self) -> &str {
            "contract"
        }

        fn connect(&mut self) -> BackendResult<Capabilities> {
            Ok(Capabilities {
                exact_geometry: false,
                server_side_borders: false,
                hide_show: false,
                explicit_ordering: false,
                fullscreen: false,
                unsupported: vec!["exact-geometry".to_owned()],
            })
        }

        fn assign_window(
            &mut self,
            backend_id: &BackendWindowId,
            win: WinId,
        ) -> Result<(), BackendContractError> {
            self.assigned = Some((backend_id.clone(), win));
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
            Err(BackendError::Unsupported {
                capability: "exact-geometry".to_owned(),
            })
        }

        fn focus(&mut self, _win: WinId) -> BackendResult<()> {
            Ok(())
        }

        fn close(&mut self, _win: WinId) -> BackendResult<()> {
            Ok(())
        }

        fn workarea(&self) -> Workarea {
            Workarea::new(1920, 1080, 32, 26)
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
            Ok(self.events.pop_front())
        }

        fn next_event(
            &mut self,
            _deadline: Option<Instant>,
        ) -> BackendResult<Option<BackendEvent>> {
            Ok(self.events.pop_front())
        }
    }

    #[test]
    #[allow(deprecated)]
    fn trait_exposes_the_accepted_backend_contract() {
        let mut backend: Box<dyn WmBackend> = Box::new(ContractBackend::default());
        let binding_id = BackendBindingId::try_from(3).unwrap();
        let turn_id = BackendPolicyTurnId::try_from(5).unwrap();
        let ticket = BackendTicket::try_from(9).unwrap();

        assert_eq!(backend.name(), "contract");
        assert!(backend.event_fd().as_raw_fd() >= 0);
        assert_eq!(backend.workarea().tiles.h, 1022);
        assert_eq!(binding_id.get(), 3);
        assert_eq!(turn_id.get(), 5);
        assert_eq!(ticket.get(), 9);
        assert!(BackendBindingId::new(0).is_none());
        assert!(BackendPolicyTurnId::try_from(0).is_err());
        assert!(BackendTicket::new(0).is_none());
        assert_ne!(BackendNextKeyEdge::Preserve, BackendNextKeyEdge::Ensure);
        assert_ne!(BackendNextKeyEdge::Ensure, BackendNextKeyEdge::Cancel);
        backend
            .configure_bindings(vec![BackendBindingSpec {
                id: binding_id,
                keysym: "Return".to_owned(),
                modifiers: vec![BackendModifier::Control],
            }])
            .unwrap();
        backend.request_policy_turn().unwrap();
        let response = BackendPolicyResponse {
            projection: Some(Vec::new()),
            closes: vec![WinId(42)],
            bindings: BackendBindingState {
                enabled: vec![binding_id],
                watched_modifiers: vec![BackendModifier::Super],
                next_key_edge: BackendNextKeyEdge::Ensure,
            },
        };
        assert_eq!(
            backend
                .respond_policy_turn(turn_id, ticket, response)
                .unwrap(),
            BackendSubmission::Complete
        );

        let interest = backend.poll_interest();
        assert!(interest.immediate && interest.readable && !interest.writable);
        assert!(matches!(
            backend
                .service(
                    BackendReady {
                        readable: true,
                        terminal: false,
                        writable: false,
                    },
                    Instant::now(),
                )
                .unwrap(),
            Some(BackendEvent::PolicyTurn(BackendPolicyTurn {
                id,
                drains: None,
                events,
            })) if id.get() == 7
                && matches!(events.as_slice(), [BackendPolicyEvent::InitialReplayComplete])
        ));
        assert!(matches!(
            backend
                .service(
                    BackendReady {
                        readable: false,
                        terminal: false,
                        writable: true,
                    },
                    Instant::now(),
                )
                .unwrap(),
            Some(BackendEvent::OperationCompleted {
                ticket: completed,
                result: Ok(()),
            }) if completed == BackendTicket::new(11).unwrap()
        ));
        assert!(matches!(
            backend
                .service(
                    BackendReady {
                        readable: false,
                        terminal: false,
                        writable: false,
                    },
                    Instant::now(),
                )
                .unwrap(),
            Some(BackendEvent::RetainedObservationsDrained { ticket: drained })
                if drained == BackendTicket::new(11).unwrap()
        ));
        assert_eq!(
            backend
                .service(
                    BackendReady {
                        readable: false,
                        terminal: true,
                        writable: false,
                    },
                    Instant::now(),
                )
                .unwrap(),
            Some(BackendEvent::Disconnected)
        );
        backend
            .begin_exit_session(BackendExitPolicy {
                enabled: vec![binding_id],
                watched_modifiers: vec![BackendModifier::Alt],
            })
            .unwrap();

        assert_eq!(MAX_MANAGED_WINDOWS, 256);
        assert_eq!(MAX_CONFIGURED_BINDINGS, 64);
        assert_eq!(MAX_POLICY_EVENTS, 256);
        assert_eq!(MAX_STAGED_EFFECTS, 256);
        assert_eq!(MAX_POLICY_TEXT_BYTES, 65_535);
        assert_eq!(MAX_BACKEND_OUTPUTS, 16);
        assert_eq!(MAX_BACKEND_SEATS, 16);
        assert_eq!(MAX_BACKEND_INPUT_DEVICES, 64);
        assert_eq!(MAX_BACKEND_LIBINPUT_DEVICES, 64);
        assert_eq!(MAX_VISIBLE_APP_ID_JSON_BYTES, 40);
        assert_eq!(MAX_VISIBLE_TITLE_JSON_BYTES, 80);
        assert_eq!(KEY_REPEAT_RATE_HZ, 25);
        assert_eq!(KEY_REPEAT_DELAY_MS, 600);
        assert_eq!(MAX_WORKER_JOBS, 256);
        assert_eq!(MAX_WORKER_RESULTS, 256);
        assert_eq!(MAX_SNAPSHOT_BYTES, 65_535);
        assert_eq!(WORKER_SHUTDOWN_TIMEOUT_MS, 2_000);
        assert_eq!(MAX_REPLAY_POLICY_EVENTS, 259);
    }

    #[test]
    fn backend_window_id_accepts_exact_bounds_and_rejects_invalid_bytes() {
        assert_eq!(BackendWindowId::new(" ").unwrap().as_str(), " ");
        assert_eq!(BackendWindowId::new("~").unwrap().as_str(), "~");

        let maximum = "x".repeat(32);
        assert_eq!(
            BackendWindowId::new(maximum.clone()).unwrap().as_str(),
            maximum
        );

        assert!(BackendWindowId::new("").is_none());
        assert!(BackendWindowId::new("x".repeat(33)).is_none());
        assert!(BackendWindowId::new("\u{1f}").is_none());
        assert!(BackendWindowId::new("\u{7f}").is_none());
        assert!(BackendWindowId::new("é").is_none());
    }

    #[test]
    fn backend_errors_retain_machine_readable_context() {
        let error = BackendError::Unsupported {
            capability: "exact-geometry".to_owned(),
        };
        assert!(matches!(
            error,
            BackendError::Unsupported { ref capability } if capability == "exact-geometry"
        ));
        assert_eq!(
            error.to_string(),
            "backend cannot honour capability exact-geometry"
        );

        let protocol = BackendError::Protocol {
            kind: BackendProtocolErrorKind::InvalidFraming,
        };
        assert!(matches!(
            &protocol,
            BackendError::Protocol {
                kind: BackendProtocolErrorKind::InvalidFraming
            }
        ));
        assert_eq!(
            protocol.to_string(),
            "backend protocol violation: InvalidFraming"
        );

        let capacity = BackendError::Capacity {
            resource: BackendCapacityResource::PolicyTurnIds,
            limit: u64::MAX,
        };
        assert!(matches!(
            &capacity,
            BackendError::Capacity {
                resource: BackendCapacityResource::PolicyTurnIds,
                limit: u64::MAX,
            }
        ));
        assert_eq!(
            capacity.to_string(),
            format!(
                "backend capacity exceeded for PolicyTurnIds: limit {}",
                u64::MAX
            )
        );
    }

    #[test]
    fn stable_backend_identity_is_assigned_before_window_use() {
        let mut backend = ContractBackend::default();
        let backend_id = BackendWindowId::new("river-window-17").unwrap();

        backend.assign_window(&backend_id, WinId(42)).unwrap();

        assert_eq!(backend.assigned, Some((backend_id.clone(), WinId(42))));
        let event = BackendEvent::WindowOpened {
            backend_id: backend_id.clone(),
            app_id: "foot".to_owned(),
            title: "shell".to_owned(),
        };
        assert!(matches!(
            event,
            BackendEvent::WindowOpened {
                backend_id: observed,
                ..
            } if observed == backend_id
        ));
    }

    #[test]
    fn exclusive_layer_focus_is_distinct_from_window_focus() {
        assert_ne!(
            BackendEvent::ExclusiveFocusChanged(true),
            BackendEvent::FocusChanged(None)
        );
    }
}
