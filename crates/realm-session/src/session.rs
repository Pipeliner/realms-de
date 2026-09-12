//! Transactional session state and projection coordination.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use realm_core::ipc::PROTOCOL_VERSION;
use realm_core::ipc::{Capabilities, LedgerEntry, OrbitLedger};
use realm_core::keys::{Action, Binding, Keymap, Mode};
use realm_core::layout::{project, Layout, Placement, TriptychParams, Workarea};
use realm_core::ledger::{Dir, Orbit, ORBIT_COUNT};
use realm_core::state::{Module, OrbitCell, OrbitDisplay, RealmState};
use realm_core::{Ledger, OrbitId, WinId};
use serde::de::{Error as _, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize};

use crate::backend::{
    BackendBindingId, BackendBindingSpec, BackendBindingState, BackendCapacityResource,
    BackendContractError, BackendError, BackendEvent, BackendModifier, BackendNextKeyEdge,
    BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
    BackendResult, BackendSubmission, BackendTicket, BackendWindowId, WmBackend,
    KEY_REPEAT_DELAY_MS, KEY_REPEAT_RATE_HZ, MAX_CONFIGURED_BINDINGS, MAX_MANAGED_WINDOWS,
    MAX_POLICY_EVENTS, MAX_POLICY_TEXT_BYTES, MAX_REPLAY_POLICY_EVENTS, MAX_STAGED_EFFECTS,
    MAX_VISIBLE_APP_ID_JSON_BYTES, MAX_VISIBLE_TITLE_JSON_BYTES,
};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const ORBIT_NAMES: [&str; ORBIT_COUNT] = [
    "triptych",
    "scriptorium",
    "observatory",
    "forge",
    "athenaeum",
    "crypt",
];

/// Stable binding between one persisted Realm id and a compositor identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotBinding {
    /// Realm-owned numeric window id.
    pub win_id: WinId,
    /// Stable compositor-owned identity.
    pub backend_id: BackendWindowId,
}

impl<'de> Deserialize<'de> for SnapshotBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BindingVisitor;

        impl<'de> Visitor<'de> for BindingVisitor {
            type Value = SnapshotBinding;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a snapshot binding with fields win_id, backend_id in order")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(key) if key == "win_id" => {}
                    Some(_) => return Err(A::Error::custom("win_id must be the first field")),
                    None => return Err(A::Error::missing_field("win_id")),
                }
                let win_id = map.next_value()?;
                match map.next_key::<String>()? {
                    Some(key) if key == "backend_id" => {}
                    Some(_) => return Err(A::Error::custom("backend_id must be the second field")),
                    None => return Err(A::Error::missing_field("backend_id")),
                }
                let backend_id = map.next_value()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(A::Error::custom("snapshot binding has an extra field"));
                }
                Ok(SnapshotBinding { win_id, backend_id })
            }
        }

        deserializer.deserialize_map(BindingVisitor)
    }
}

#[derive(Serialize)]
struct SnapshotOrbitWire {
    id: OrbitId,
    windows: Vec<WinId>,
    focus: Option<usize>,
    stowed: Vec<WinId>,
    layout: Layout,
    fullscreen: Option<WinId>,
    name: String,
}

impl<'de> Deserialize<'de> for SnapshotOrbitWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OrbitVisitor;

        impl<'de> Visitor<'de> for OrbitVisitor {
            type Value = SnapshotOrbitWire;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an orbit with its seven fields in canonical order")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                require_field(&mut map, "id", 1)?;
                let id = map.next_value()?;
                require_field(&mut map, "windows", 2)?;
                let windows = map.next_value()?;
                require_field(&mut map, "focus", 3)?;
                let focus = map.next_value()?;
                require_field(&mut map, "stowed", 4)?;
                let stowed = map.next_value()?;
                require_field(&mut map, "layout", 5)?;
                let layout = map.next_value()?;
                require_field(&mut map, "fullscreen", 6)?;
                let fullscreen = map.next_value()?;
                require_field(&mut map, "name", 7)?;
                let name = map.next_value()?;
                reject_extra_field(&mut map, "orbit")?;
                Ok(SnapshotOrbitWire {
                    id,
                    windows,
                    focus,
                    stowed,
                    layout,
                    fullscreen,
                    name,
                })
            }
        }

        deserializer.deserialize_map(OrbitVisitor)
    }
}

impl From<&Orbit> for SnapshotOrbitWire {
    fn from(orbit: &Orbit) -> Self {
        Self {
            id: orbit.id,
            windows: orbit.windows.clone(),
            focus: orbit.focus,
            stowed: orbit.stowed.clone(),
            layout: orbit.layout,
            fullscreen: orbit.fullscreen,
            name: orbit.name.clone(),
        }
    }
}

#[derive(Serialize)]
struct SnapshotLedgerWire {
    orbits: Vec<SnapshotOrbitWire>,
    active: OrbitId,
}

impl SnapshotLedgerWire {
    fn into_ledger(self) -> Result<Ledger, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(self)?)
    }
}

impl From<&Ledger> for SnapshotLedgerWire {
    fn from(ledger: &Ledger) -> Self {
        Self {
            orbits: ledger.orbits().iter().map(Into::into).collect(),
            active: ledger.active(),
        }
    }
}

impl<'de> Deserialize<'de> for SnapshotLedgerWire {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct LedgerVisitor;

        impl<'de> Visitor<'de> for LedgerVisitor {
            type Value = SnapshotLedgerWire;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a ledger with fields orbits, active in canonical order")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                require_field(&mut map, "orbits", 1)?;
                let orbits = map.next_value()?;
                require_field(&mut map, "active", 2)?;
                let active = map.next_value()?;
                reject_extra_field(&mut map, "ledger")?;
                Ok(SnapshotLedgerWire { orbits, active })
            }
        }

        deserializer.deserialize_map(LedgerVisitor)
    }
}

fn require_field<'de, A>(
    map: &mut A,
    expected: &'static str,
    ordinal: usize,
) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
{
    match map.next_key::<String>()? {
        Some(key) if key == expected => Ok(()),
        Some(_) => Err(A::Error::custom(format_args!(
            "{expected} must be field {ordinal}"
        ))),
        None => Err(A::Error::missing_field(expected)),
    }
}

fn reject_extra_field<'de, A>(map: &mut A, record: &'static str) -> Result<(), A::Error>
where
    A: MapAccess<'de>,
{
    if map.next_key::<String>()?.is_some() {
        return Err(A::Error::custom(format_args!(
            "{record} has an extra field"
        )));
    }
    Ok(())
}

/// Closed, versioned persistence record for one authoritative live session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSnapshotV1 {
    schema_version: u32,
    protocol_version: u32,
    ledger: Ledger,
    bindings: Vec<SnapshotBinding>,
    next_win_id: u64,
    active_orbit: OrbitId,
}

impl Serialize for SessionSnapshotV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut record = serializer.serialize_struct("SessionSnapshotV1", 6)?;
        record.serialize_field("schema_version", &self.schema_version)?;
        record.serialize_field("protocol_version", &self.protocol_version)?;
        record.serialize_field("ledger", &SnapshotLedgerWire::from(&self.ledger))?;
        record.serialize_field("bindings", &self.bindings)?;
        record.serialize_field("next_win_id", &self.next_win_id)?;
        record.serialize_field("active_orbit", &self.active_orbit)?;
        record.end()
    }
}

impl<'de> Deserialize<'de> for SessionSnapshotV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SnapshotVisitor;

        impl<'de> Visitor<'de> for SnapshotVisitor {
            type Value = SessionSnapshotV1;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(
                    "a version-one session snapshot with its six fields in canonical order",
                )
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                match map.next_key::<String>()? {
                    Some(key) if key == "schema_version" => {}
                    Some(_) => {
                        return Err(A::Error::custom("schema_version must be the first field"));
                    }
                    None => return Err(A::Error::missing_field("schema_version")),
                }
                let schema_version = map.next_value()?;
                match map.next_key::<String>()? {
                    Some(key) if key == "protocol_version" => {}
                    Some(_) => {
                        return Err(A::Error::custom(
                            "protocol_version must be the second field",
                        ));
                    }
                    None => return Err(A::Error::missing_field("protocol_version")),
                }
                let protocol_version = map.next_value()?;
                match map.next_key::<String>()? {
                    Some(key) if key == "ledger" => {}
                    Some(_) => return Err(A::Error::custom("ledger must be the third field")),
                    None => return Err(A::Error::missing_field("ledger")),
                }
                let ledger = map
                    .next_value::<SnapshotLedgerWire>()?
                    .into_ledger()
                    .map_err(A::Error::custom)?;
                match map.next_key::<String>()? {
                    Some(key) if key == "bindings" => {}
                    Some(_) => return Err(A::Error::custom("bindings must be the fourth field")),
                    None => return Err(A::Error::missing_field("bindings")),
                }
                let bindings = map.next_value()?;
                match map.next_key::<String>()? {
                    Some(key) if key == "next_win_id" => {}
                    Some(_) => {
                        return Err(A::Error::custom("next_win_id must be the fifth field"));
                    }
                    None => return Err(A::Error::missing_field("next_win_id")),
                }
                let next_win_id = map.next_value()?;
                match map.next_key::<String>()? {
                    Some(key) if key == "active_orbit" => {}
                    Some(_) => {
                        return Err(A::Error::custom("active_orbit must be the sixth field"));
                    }
                    None => return Err(A::Error::missing_field("active_orbit")),
                }
                let active_orbit = map.next_value()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(A::Error::custom("session snapshot has an extra field"));
                }
                Ok(SessionSnapshotV1 {
                    schema_version,
                    protocol_version,
                    ledger,
                    bindings,
                    next_win_id,
                    active_orbit,
                })
            }
        }

        deserializer.deserialize_map(SnapshotVisitor)
    }
}

/// Failure to decode or validate a persisted session snapshot.
#[derive(Debug, thiserror::Error)]
pub enum SessionSnapshotError {
    /// The record is not valid JSON or does not match the closed schema.
    #[error("invalid session snapshot encoding: {0}")]
    Encoding(#[from] serde_json::Error),
    /// The record violates a snapshot invariant.
    #[error("invalid session snapshot: {0}")]
    Invalid(&'static str),
}

impl SessionSnapshotV1 {
    /// Construct and validate a version-one snapshot.
    pub fn new(
        mut ledger: Ledger,
        bindings: Vec<SnapshotBinding>,
        next_win_id: u64,
    ) -> Result<Self, SessionSnapshotError> {
        ledger.discard_undo_history();
        let active_orbit = ledger.active();
        let snapshot = Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            protocol_version: PROTOCOL_VERSION,
            ledger,
            bindings,
            next_win_id,
            active_orbit,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Decode and validate the closed version-one JSON representation.
    pub fn from_json(bytes: &[u8]) -> Result<Self, SessionSnapshotError> {
        let snapshot: Self = serde_json::from_slice(bytes)?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Encode the validated closed version-one JSON representation.
    pub fn to_json(&self) -> Result<Vec<u8>, SessionSnapshotError> {
        self.validate()?;
        Ok(serde_json::to_vec(self)?)
    }

    /// Sorted persisted bindings.
    pub fn bindings(&self) -> &[SnapshotBinding] {
        &self.bindings
    }

    /// Next numeric id, or the exhausted sentinel.
    pub fn next_win_id(&self) -> u64 {
        self.next_win_id
    }

    fn validate(&self) -> Result<(), SessionSnapshotError> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(SessionSnapshotError::Invalid("unsupported schema version"));
        }
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(SessionSnapshotError::Invalid("protocol version mismatch"));
        }
        if self.active_orbit != self.ledger.active() {
            return Err(SessionSnapshotError::Invalid(
                "active orbit disagrees with ledger",
            ));
        }
        if self.active_orbit.index() >= ORBIT_COUNT {
            return Err(SessionSnapshotError::Invalid(
                "active orbit is out of range",
            ));
        }
        if self.ledger.orbits().len() != ORBIT_COUNT {
            return Err(SessionSnapshotError::Invalid(
                "ledger must contain six orbits",
            ));
        }

        let mut ledger_windows = BTreeSet::new();
        for (index, orbit) in self.ledger.orbits().iter().enumerate() {
            if orbit.id != OrbitId::new(index).expect("canonical orbit index") {
                return Err(SessionSnapshotError::Invalid("orbit ids are not canonical"));
            }
            if orbit.name != ORBIT_NAMES[index] {
                return Err(SessionSnapshotError::Invalid(
                    "orbit names are not canonical",
                ));
            }
            match orbit.focus {
                None if !orbit.windows.is_empty() => {
                    return Err(SessionSnapshotError::Invalid("occupied orbit has no focus"));
                }
                Some(_) if orbit.windows.is_empty() => {
                    return Err(SessionSnapshotError::Invalid("empty orbit has a focus"));
                }
                Some(focus) if focus >= orbit.windows.len() => {
                    return Err(SessionSnapshotError::Invalid("orbit focus is out of range"));
                }
                _ => {}
            }
            for win in &orbit.windows {
                if win.0 >= self.next_win_id || !ledger_windows.insert(*win) {
                    return Err(SessionSnapshotError::Invalid(
                        "window ids must be unique and below the watermark",
                    ));
                }
            }
            let mut stowed = BTreeSet::new();
            for win in &orbit.stowed {
                if !orbit.windows.contains(win) || !stowed.insert(*win) {
                    return Err(SessionSnapshotError::Invalid(
                        "stowed windows must be a unique subset of the orbit",
                    ));
                }
            }
            if orbit
                .fullscreen
                .is_some_and(|win| !orbit.windows.contains(&win))
            {
                return Err(SessionSnapshotError::Invalid(
                    "fullscreen window must belong to the orbit",
                ));
            }
        }

        let mut bound_windows = BTreeSet::new();
        let mut backend_ids = BTreeSet::new();
        let mut previous = None;
        for binding in &self.bindings {
            if !bound_windows.insert(binding.win_id) || !backend_ids.insert(&binding.backend_id) {
                return Err(SessionSnapshotError::Invalid(
                    "bindings must be a one-to-one mapping",
                ));
            }
            if previous.is_some_and(|win| win >= binding.win_id) {
                return Err(SessionSnapshotError::Invalid(
                    "bindings must be strictly sorted by window id",
                ));
            }
            previous = Some(binding.win_id);
        }
        if bound_windows != ledger_windows {
            return Err(SessionSnapshotError::Invalid(
                "bindings must cover the ledger exactly",
            ));
        }
        Ok(())
    }
}

/// Recovery position relative to the backend's explicit replay barrier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPhase {
    /// Initial compositor windows are being accumulated without side effects.
    InitialReplay,
    /// Reconciliation is installed and backend work must finish before events continue.
    FinalizingReplay,
    /// The session is authoritative and accepts live events and desired actions.
    Live,
    /// A finalized Quit has stopped ordinary admission.
    QuitPending,
    /// Shutdown has discarded ordinary transaction semantics.
    ShuttingDown,
}

#[derive(Debug, Clone)]
struct ReplayWindow {
    backend_id: BackendWindowId,
    metadata: WindowMetadata,
}

/// Metadata retained for a window independently of the ledger's ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowMetadata {
    /// Application identifier reported by the compositor.
    pub app_id: String,
    /// Latest non-null title reported by the compositor.
    pub title: String,
}

/// Immediate programming edge for the sole key-repeat timer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RepeatTimerDirective {
    /// Preserve the current timer programming.
    #[default]
    Preserve,
    /// Arm or restart the timer with the fixed policy.
    Arm {
        /// Delay before the first expiry.
        delay: Duration,
        /// Interval between later expiries.
        interval: Duration,
    },
    /// Disarm the timer immediately.
    Disarm,
}

/// The completion barrier required after a derived Quit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuitAfter {
    /// A key-only Quit has no requester.
    NoRequester,
    /// Quit shares the original external action's completion.
    OriginalAction(BackendTicket),
    /// Direct control Quit owns the current request.
    CurrentControlRequest,
}

/// Ordered work released only at a clean transaction boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEffect {
    /// Launch one argv vector through the worker owner.
    Spawn(Vec<String>),
    /// Open the launcher.
    Launcher,
    /// Open the full binding sheet.
    Grimoire,
    /// Retained legacy theme action.
    ReloadTheme,
    /// Stop admission and apply the matching Quit barrier.
    QuitPending {
        /// Barrier owner that must settle before shutdown begins.
        after: QuitAfter,
    },
}

/// Final result for one externally admitted action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionCompletion {
    /// Original external ticket, never a follow-up response ticket.
    pub ticket: BackendTicket,
    /// Final clean result.
    pub result: Result<(), SessionActionError>,
}

/// A safely classified nonfatal backend diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDiagnostic {
    /// Backend error that did not invalidate the incarnation.
    pub error: BackendError,
}

/// Observable result of one in-process session transition.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUpdate {
    /// Immediate repeat-timer edge, consumed before all delayed fields.
    pub repeat_timer: RepeatTimerDirective,
    /// New authoritative persistence value, when one changed.
    pub persistence: Option<SessionSnapshotV1>,
    /// Whether one projection was successfully submitted to the backend.
    pub projection_applied: bool,
    /// The new visible snapshot, present only when it differs from the last one.
    pub state: Option<RealmState>,
    /// Original ticket exposed only by successful external admission.
    pub pending_action: Option<BackendTicket>,
    /// Original external result exposed only at its clean final boundary.
    pub action_completion: Option<ActionCompletion>,
    /// Ordered delayed effects.
    pub effects: Vec<SessionEffect>,
    /// One nonfatal diagnostic.
    pub diagnostic: Option<SessionDiagnostic>,
    /// A valid event intentionally left for a later policy slice.
    pub deferred: Option<BackendEvent>,
}

impl SessionUpdate {
    /// Construct the closed no-change result.
    pub fn unchanged() -> Self {
        Self {
            repeat_timer: RepeatTimerDirective::Preserve,
            persistence: None,
            projection_applied: false,
            state: None,
            pending_action: None,
            action_completion: None,
            effects: Vec::new(),
            diagnostic: None,
            deferred: None,
        }
    }

    fn deferred(event: BackendEvent) -> Self {
        Self {
            repeat_timer: RepeatTimerDirective::Preserve,
            persistence: None,
            projection_applied: false,
            state: None,
            pending_action: None,
            action_completion: None,
            effects: Vec::new(),
            diagnostic: None,
            deferred: Some(event),
        }
    }
}

/// Failure while translating a backend event into session state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionEventError {
    /// The compositor backend failed.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// The backend violated its local identity or policy contract.
    #[error(transparent)]
    BackendContract(#[from] BackendContractError),
    /// Every possible numeric window id has already been allocated.
    #[error("Realm window id space is exhausted")]
    WindowIdExhausted,
    /// Every nonzero response ticket has been consumed.
    #[error("backend response ticket space is exhausted")]
    BackendTicketExhausted,
    /// The one scheduled retry of pending backend work also failed.
    #[error("backend retry exhausted: {0}")]
    BackendRetryExhausted(BackendError),
    /// A backend event arrived while recovery or repair work was pending.
    #[error("backend event received while backend work is pending")]
    BackendWorkPending,
    /// The backend emitted its one-shot replay barrier more than once.
    #[error("initial replay completion barrier was repeated")]
    RepeatedInitialReplayComplete,
    /// The backend emitted an event that cannot exist before replay completes.
    #[error("unexpected event during initial replay: {0:?}")]
    UnexpectedInitialReplayEvent(BackendEvent),
    /// A policy fact cannot occur in the initial replay batch.
    #[error("unexpected policy event during initial replay: {0:?}")]
    UnexpectedInitialReplayPolicyEvent(BackendPolicyEvent),
}

/// Failure of a caller-requested session action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionActionError {
    /// The session is recovering or must repair pending backend work first.
    #[error("session is not ready for actions")]
    NotReady,
    /// Spawn requires a nonempty argv and program.
    #[error("spawn command requires a nonempty program")]
    InvalidSpawnCommand,
    /// Every nonzero response ticket has been consumed.
    #[error("backend response ticket space is exhausted")]
    BackendTicketExhausted,
    /// The action reached the compositor backend and it failed.
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// Result of a caller-requested session action.
pub type SessionActionResult<T> = Result<T, SessionActionError>;

#[derive(Debug, Clone)]
struct AuthorityState {
    ledger: Ledger,
    windows: BTreeMap<WinId, WindowMetadata>,
    backend_ids: BTreeMap<BackendWindowId, WinId>,
    bound_backend_ids: BTreeSet<BackendWindowId>,
    next_win_id: u64,
    workarea: Option<Workarea>,
    exclusive_focus: bool,
    effective_focus: Option<BackendWindowId>,
    mode: Mode,
    chord_echo: String,
    whichkey: bool,
    modules: Vec<Module>,
    active_modifiers: Vec<BackendModifier>,
    held_bindings: BTreeSet<BackendBindingId>,
}

#[derive(Debug, Clone)]
struct ActiveTransaction {
    committed: AuthorityState,
    visible: RealmState,
    last_committed_clean_projection: Vec<Placement>,
    working: AuthorityState,
    most_recent_private_projection: Vec<Placement>,
    original_action_result: Option<(BackendTicket, Result<(), SessionActionError>)>,
    repair_available: bool,
    response: BackendPolicyResponse,
    staged_effects: Vec<SessionEffect>,
    close_target: Option<WinId>,
    repeat_candidate: Option<BackendBindingId>,
    repeat_timer: RepeatTimerDirective,
    projection_required: bool,
    binding_changed: bool,
    bootstrap: bool,
    quit_staged: bool,
    terminal_result: Option<BackendResult<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransactionSubstate {
    Idle,
    AwaitingExternalTurn {
        ticket: BackendTicket,
    },
    AwaitingInternalTurn {
        ticket: BackendTicket,
    },
    InFlight {
        ticket: BackendTicket,
    },
    AwaitingDrain {
        ticket: BackendTicket,
    },
    RetryReady,
    #[allow(dead_code)]
    AwaitingRepairTurn {
        ticket: BackendTicket,
    },
}

/// The compositor-independent owner of Realm's ledger and visible state.
pub struct Session<B: WmBackend> {
    backend: B,
    ledger: Ledger,
    capabilities: Capabilities,
    workarea: Workarea,
    windows: BTreeMap<WinId, WindowMetadata>,
    backend_ids: BTreeMap<BackendWindowId, WinId>,
    pending_assignments: BTreeMap<BackendWindowId, WinId>,
    next_win_id: u64,
    last_projection: Vec<Placement>,
    projection_dirty: bool,
    state: RealmState,
    phase: RecoveryPhase,
    replay: Vec<ReplayWindow>,
    pending_backend_work: bool,
    policy_transactions: bool,
    bindings: BTreeMap<BackendBindingId, Binding>,
    binding_order: Vec<BackendBindingId>,
    modifier_label: String,
    binding_state: BackendBindingState,
    bound_backend_ids: BTreeSet<BackendWindowId>,
    workarea_known: bool,
    exclusive_focus: bool,
    effective_focus: Option<BackendWindowId>,
    mode: Mode,
    chord_echo: String,
    whichkey: bool,
    modules: Vec<Module>,
    active_modifiers: Vec<BackendModifier>,
    held_bindings: BTreeSet<BackendBindingId>,
    repeat_target: Option<BackendBindingId>,
    transaction: TransactionSubstate,
    active: Option<ActiveTransaction>,
    last_backend_ticket: u64,
    last_policy_turn: Option<BackendPolicyTurnId>,
    published_ledger: Ledger,
    published_windows: BTreeMap<WinId, WindowMetadata>,
}

#[allow(deprecated)]
impl<B: WmBackend> Session<B> {
    /// Connect a backend and seed an empty six-orbit session.
    pub fn connect(backend: B) -> BackendResult<Self> {
        Self::connect_with_snapshot_and_keymap(backend, None, Keymap::default())
    }

    /// Connect a backend and enter initial replay using an optional validated snapshot.
    pub fn connect_with_snapshot(
        backend: B,
        snapshot: Option<SessionSnapshotV1>,
    ) -> BackendResult<Self> {
        Self::connect_with_snapshot_and_keymap(backend, snapshot, Keymap::default())
    }

    fn connect_with_keymap(backend: B, keymap: Keymap) -> BackendResult<Self> {
        Self::connect_with_snapshot_and_keymap(backend, None, keymap)
    }

    fn connect_with_snapshot_and_keymap(
        mut backend: B,
        snapshot: Option<SessionSnapshotV1>,
        keymap: Keymap,
    ) -> BackendResult<Self> {
        if keymap.bindings.len() > MAX_CONFIGURED_BINDINGS {
            return Err(BackendError::Capacity {
                resource: BackendCapacityResource::ConfiguredBindings,
                limit: MAX_CONFIGURED_BINDINGS as u64,
            });
        }
        let capabilities = backend.connect()?;
        let workarea = backend.workarea();
        let (ledger, backend_ids, next_win_id) = snapshot.map_or_else(
            || (Ledger::new(), BTreeMap::new(), 0),
            |snapshot| {
                let backend_ids = snapshot
                    .bindings
                    .into_iter()
                    .map(|binding| (binding.backend_id, binding.win_id))
                    .collect();
                (snapshot.ledger, backend_ids, snapshot.next_win_id)
            },
        );
        let Keymap {
            modifier: modifier_label,
            bindings: keymap_bindings,
        } = keymap;
        let mut bindings = BTreeMap::new();
        let mut binding_order = Vec::with_capacity(keymap_bindings.len());
        let mut mechanisms = Vec::with_capacity(keymap_bindings.len());
        for (index, binding) in keymap_bindings.into_iter().enumerate() {
            let id = BackendBindingId::new(
                u32::try_from(index + 1).expect("binding capacity fits a u32"),
            )
            .expect("configured binding ids start at one");
            let modifiers = if binding.mode == Mode::Nav {
                vec![BackendModifier::Super]
            } else {
                Vec::new()
            };
            mechanisms.push(BackendBindingSpec {
                id,
                keysym: binding.key.clone(),
                modifiers,
            });
            binding_order.push(id);
            bindings.insert(id, binding);
        }
        backend.configure_bindings(mechanisms)?;
        let published_ledger = Ledger::new();
        Ok(Self {
            backend,
            ledger,
            capabilities,
            workarea,
            windows: BTreeMap::new(),
            backend_ids,
            pending_assignments: BTreeMap::new(),
            next_win_id,
            last_projection: Vec::new(),
            projection_dirty: true,
            state: RealmState::default(),
            phase: RecoveryPhase::InitialReplay,
            replay: Vec::new(),
            pending_backend_work: false,
            policy_transactions: false,
            bindings,
            binding_order,
            modifier_label,
            binding_state: BackendBindingState {
                enabled: Vec::new(),
                watched_modifiers: Vec::new(),
                next_key_edge: BackendNextKeyEdge::Preserve,
            },
            bound_backend_ids: BTreeSet::new(),
            workarea_known: false,
            exclusive_focus: false,
            effective_focus: None,
            mode: Mode::Nav,
            chord_echo: String::new(),
            whichkey: RealmState::default().whichkey,
            modules: Vec::new(),
            active_modifiers: Vec::new(),
            held_bindings: BTreeSet::new(),
            repeat_target: None,
            transaction: TransactionSubstate::Idle,
            active: None,
            last_backend_ticket: 0,
            last_policy_turn: None,
            published_ledger,
            published_windows: BTreeMap::new(),
        })
    }

    /// Current recovery phase.
    pub fn phase(&self) -> RecoveryPhase {
        self.phase
    }

    /// True when backend repair must run before another event is read.
    pub fn has_pending_backend_work(&self) -> bool {
        self.pending_backend_work || self.transaction == TransactionSubstate::RetryReady
    }

    /// True until the current requested/response/drain transaction is final.
    pub fn has_active_backend_transaction(&self) -> bool {
        self.transaction != TransactionSubstate::Idle
    }

    pub(crate) fn next_backend_event(
        &mut self,
        now: Instant,
    ) -> BackendResult<Option<BackendEvent>> {
        self.backend.next_event(Some(now))
    }

    /// A validated persistence snapshot, exposed only in authoritative live state.
    pub fn snapshot(&self) -> Option<SessionSnapshotV1> {
        (self.phase == RecoveryPhase::Live).then(|| {
            let bindings = self
                .backend_ids
                .iter()
                .map(|(backend_id, win_id)| SnapshotBinding {
                    win_id: *win_id,
                    backend_id: backend_id.clone(),
                })
                .collect::<Vec<_>>();
            let mut bindings = bindings;
            bindings.sort_by_key(|binding| binding.win_id);
            SessionSnapshotV1::new(self.ledger.clone(), bindings, self.next_win_id)
                .expect("live session invariants produce a valid snapshot")
        })
    }

    /// The authoritative ledger.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Capabilities returned by the backend during connection.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// The last visible state accepted for publication.
    pub fn state(&self) -> &RealmState {
        &self.state
    }

    /// Metadata for a managed window.
    pub fn window_metadata(&self, win: WinId) -> Option<&WindowMetadata> {
        self.windows.get(&win)
    }

    /// Realm id currently assigned to a compositor-stable identity.
    pub fn window_id(&self, backend_id: &BackendWindowId) -> Option<WinId> {
        self.backend_ids.get(backend_id).copied()
    }

    /// The projection most recently accepted by the backend.
    pub fn last_projection(&self) -> &[Placement] {
        &self.last_projection
    }

    fn binding_id_for_key(&self, key: &str) -> Option<BackendBindingId> {
        self.bindings
            .iter()
            .find_map(|(id, binding)| (binding.key == key).then_some(*id))
    }

    /// Switch the visible orbit transactionally.
    pub fn switch_orbit(&mut self, orbit: OrbitId) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.switch_orbit(orbit))
    }

    /// Change the active orbit's layout transactionally.
    pub fn set_layout(&mut self, layout: Layout) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.set_layout(layout))
    }

    /// Move focus by one ledger position transactionally.
    pub fn focus_step(&mut self, direction: Dir) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.focus_step(direction))
    }

    /// Swap the focused window with its neighbour transactionally.
    pub fn swap(&mut self, direction: Dir) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.swap(direction);
        })
    }

    /// Move the focused window to another orbit transactionally.
    pub fn move_focused_to_orbit(&mut self, orbit: OrbitId) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.move_to_orbit(orbit);
        })
    }

    /// Toggle the focused window's stowed state transactionally.
    pub fn toggle_stow(&mut self) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.toggle_stow();
        })
    }

    /// Toggle fullscreen for the focused window transactionally.
    pub fn toggle_fullscreen(&mut self) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.toggle_fullscreen();
        })
    }

    /// Restore the previous ledger state transactionally.
    pub fn undo(&mut self) -> SessionActionResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.undo();
        })
    }

    /// Ask the focused window to close without changing authoritative state.
    pub fn request_close_focused(&mut self) -> SessionActionResult<SessionUpdate> {
        if self.phase != RecoveryPhase::Live
            || self.pending_backend_work
            || self.has_active_backend_transaction()
        {
            return Err(SessionActionError::NotReady);
        }
        let Some(win) = self.ledger.focused() else {
            return Ok(SessionUpdate::unchanged());
        };
        if !self.policy_transactions {
            self.backend.close(win)?;
            return Ok(SessionUpdate::unchanged());
        }
        let ticket = self.allocate_action_ticket()?;
        let mut active = self.new_active_transaction();
        active.close_target = Some(win);
        active.original_action_result = Some((ticket, Ok(())));
        self.active = Some(active);
        self.transaction = TransactionSubstate::AwaitingExternalTurn { ticket };
        match self.backend.request_policy_turn() {
            Ok(()) => Ok(SessionUpdate {
                pending_action: Some(ticket),
                ..SessionUpdate::unchanged()
            }),
            Err(error) => {
                self.active = None;
                self.transaction = TransactionSubstate::Idle;
                Err(SessionActionError::Backend(error))
            }
        }
    }

    /// Admit an effect-only Spawn without allocating a compositor ticket.
    pub fn request_spawn(&mut self, argv: Vec<String>) -> SessionActionResult<SessionUpdate> {
        if self.phase != RecoveryPhase::Live
            || self.pending_backend_work
            || self.has_active_backend_transaction()
        {
            return Err(SessionActionError::NotReady);
        }
        if argv.is_empty() || argv.first().is_none_or(String::is_empty) {
            return Err(SessionActionError::InvalidSpawnCommand);
        }
        Ok(SessionUpdate {
            effects: vec![SessionEffect::Spawn(argv)],
            ..SessionUpdate::unchanged()
        })
    }

    /// Toggle the visible which-key strip without applying a projection.
    pub fn toggle_whichkey(&mut self) -> SessionUpdate {
        if self.phase != RecoveryPhase::Live
            || self.pending_backend_work
            || self.has_active_backend_transaction()
        {
            return SessionUpdate::unchanged();
        }
        let mut changed = self.state.clone();
        changed.revision = changed.revision.saturating_add(1);
        changed.whichkey = !changed.whichkey;
        self.whichkey = changed.whichkey;
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            ..SessionUpdate::unchanged()
        }
    }

    fn stage_ledger_update(
        &mut self,
        update: impl FnOnce(&mut Ledger),
    ) -> SessionActionResult<SessionUpdate> {
        if self.phase != RecoveryPhase::Live
            || self.pending_backend_work
            || self.has_active_backend_transaction()
        {
            return Err(SessionActionError::NotReady);
        }
        if self.policy_transactions {
            return self.stage_policy_ledger_update(update);
        }
        let mut candidate = self.ledger.clone();
        update(&mut candidate);
        match self.commit_desired(candidate, self.windows.clone(), self.workarea) {
            Ok(update) => Ok(update),
            Err(error) => {
                self.pending_backend_work = should_retry_desired_repair(&error);
                Err(error.into())
            }
        }
    }

    /// Run the one permitted retry of currently pending backend work.
    pub fn retry_pending_backend_work(&mut self) -> Result<SessionUpdate, SessionEventError> {
        if !self.pending_backend_work {
            return Ok(SessionUpdate::unchanged());
        }
        self.pending_backend_work = false;
        let result = if self.phase == RecoveryPhase::FinalizingReplay {
            self.finish_replay()
        } else {
            self.repair_authoritative_projection()
        };
        result.map_err(|error| match error {
            SessionEventError::Backend(error) => SessionEventError::BackendRetryExhausted(error),
            error => error,
        })
    }

    /// Apply one compositor event that has compositor-independent semantics.
    pub fn handle_backend_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<SessionUpdate, SessionEventError> {
        match event {
            BackendEvent::PolicyTurn(turn) => {
                self.policy_transactions = true;
                return self.handle_policy_turn(turn);
            }
            BackendEvent::OperationCompleted { ticket, result } => {
                return self.handle_operation_completed(ticket, result);
            }
            BackendEvent::RetainedObservationsDrained { ticket } => {
                return self.handle_observations_drained(ticket);
            }
            event => return self.handle_legacy_backend_event(event),
        }
    }

    fn handle_legacy_backend_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<SessionUpdate, SessionEventError> {
        if self.pending_backend_work {
            return Err(SessionEventError::BackendWorkPending);
        }
        match self.phase {
            RecoveryPhase::InitialReplay => self.handle_initial_replay_event(event),
            RecoveryPhase::FinalizingReplay => match event {
                BackendEvent::Disconnected => Err(BackendError::Disconnected.into()),
                BackendEvent::InitialReplayComplete => {
                    Err(SessionEventError::RepeatedInitialReplayComplete)
                }
                _ => Err(SessionEventError::BackendWorkPending),
            },
            RecoveryPhase::Live => self.handle_live_event(event),
            RecoveryPhase::QuitPending | RecoveryPhase::ShuttingDown => {
                Err(SessionEventError::BackendWorkPending)
            }
        }
    }

    fn handle_initial_replay_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<SessionUpdate, SessionEventError> {
        match event {
            BackendEvent::WindowOpened {
                backend_id,
                app_id,
                title,
            } => {
                let metadata = WindowMetadata { app_id, title };
                if let Some(replayed) = self
                    .replay
                    .iter_mut()
                    .find(|window| window.backend_id == backend_id)
                {
                    replayed.metadata = metadata;
                } else {
                    self.replay.push(ReplayWindow {
                        backend_id,
                        metadata,
                    });
                }
                Ok(SessionUpdate::unchanged())
            }
            BackendEvent::InitialReplayComplete => self.finalize_initial_replay(),
            BackendEvent::WorkareaChanged(workarea) => {
                self.workarea = workarea;
                Ok(SessionUpdate::unchanged())
            }
            BackendEvent::Disconnected => Err(BackendError::Disconnected.into()),
            event @ (BackendEvent::FocusChanged(_)
            | BackendEvent::ExclusiveFocusChanged(_)
            | BackendEvent::GeometryDrifted { .. }
            | BackendEvent::TitleChanged { .. }
            | BackendEvent::WindowClosed(_)
            | BackendEvent::PolicyTurn(_)
            | BackendEvent::OperationCompleted { .. }
            | BackendEvent::RetainedObservationsDrained { .. }) => {
                Err(SessionEventError::UnexpectedInitialReplayEvent(event))
            }
        }
    }

    fn handle_live_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<SessionUpdate, SessionEventError> {
        let result = match event {
            BackendEvent::InitialReplayComplete => {
                return Err(SessionEventError::RepeatedInitialReplayComplete);
            }
            BackendEvent::WindowOpened {
                backend_id,
                app_id,
                title,
            } => {
                let win = self.record_window_identity(backend_id)?;
                self.ledger.summon(win, self.ledger.active());
                self.windows.insert(win, WindowMetadata { app_id, title });
                self.repair_authoritative_projection()
            }
            BackendEvent::WindowClosed(win) => {
                let mut ledger = self.ledger.clone();
                ledger.banish(win);
                let mut windows = self.windows.clone();
                windows.remove(&win);
                self.backend_ids.retain(|_, assigned| *assigned != win);
                self.pending_assignments
                    .retain(|_, assigned| *assigned != win);
                self.commit_observed(ledger, windows, self.workarea)
            }
            BackendEvent::TitleChanged { win, title } => {
                let Some(metadata) = self.windows.get_mut(&win) else {
                    return Ok(SessionUpdate::unchanged());
                };
                metadata.title = title;
                self.repair_authoritative_projection()
            }
            BackendEvent::WorkareaChanged(workarea) => {
                self.commit_observed(self.ledger.clone(), self.windows.clone(), workarea)
            }
            BackendEvent::GeometryDrifted { .. } => Ok(SessionUpdate::unchanged()),
            BackendEvent::Disconnected => Err(BackendError::Disconnected.into()),
            BackendEvent::PolicyTurn(_)
            | BackendEvent::OperationCompleted { .. }
            | BackendEvent::RetainedObservationsDrained { .. } => {
                Err(SessionEventError::BackendWorkPending)
            }
            event @ (BackendEvent::FocusChanged(_) | BackendEvent::ExclusiveFocusChanged(_)) => {
                Ok(SessionUpdate::deferred(event))
            }
        };
        match result {
            Ok(update) => Ok(update),
            Err(error) => {
                self.pending_backend_work = matches!(
                    &error,
                    SessionEventError::Backend(error) if should_retry_authoritative(error)
                );
                Err(error)
            }
        }
    }

    /// Replace bar-module values without issuing a compositor request.
    pub fn update_modules(&mut self, modules: Vec<Module>) -> SessionUpdate {
        if self.phase != RecoveryPhase::Live
            || self.pending_backend_work
            || self.has_active_backend_transaction()
        {
            return SessionUpdate::unchanged();
        }
        if self.state.modules == modules {
            return SessionUpdate::unchanged();
        }
        let mut changed = self.state.clone();
        changed.revision = changed.revision.saturating_add(1);
        changed.modules = modules;
        self.modules = changed.modules.clone();
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            ..SessionUpdate::unchanged()
        }
    }

    fn finalize_initial_replay(&mut self) -> Result<SessionUpdate, SessionEventError> {
        let replayed_ids: BTreeSet<_> = self
            .replay
            .iter()
            .map(|window| window.backend_id.clone())
            .collect();
        let unknown_count = self
            .replay
            .iter()
            .filter(|window| !self.backend_ids.contains_key(&window.backend_id))
            .count() as u128;
        let available = u128::from(u64::MAX - self.next_win_id);
        if unknown_count > available {
            return Err(SessionEventError::WindowIdExhausted);
        }

        let mut ledger = self.ledger.clone();
        let mut backend_ids = self.backend_ids.clone();
        let mut windows = BTreeMap::new();
        let mut pending_assignments = BTreeMap::new();
        let mut next_win_id = self.next_win_id;

        let persisted_order: Vec<_> = ledger
            .orbits()
            .iter()
            .flat_map(|orbit| orbit.windows.iter().copied())
            .collect();
        for win in persisted_order {
            let backend_id = backend_ids
                .iter()
                .find_map(|(backend_id, assigned)| (*assigned == win).then(|| backend_id.clone()))
                .expect("validated snapshot binds every ledger window");
            if !replayed_ids.contains(&backend_id) {
                ledger.banish(win);
                backend_ids.remove(&backend_id);
            }
        }

        for replayed in &self.replay {
            let win = if let Some(win) = backend_ids.get(&replayed.backend_id).copied() {
                win
            } else {
                let win = WinId(next_win_id);
                next_win_id = next_win_id
                    .checked_add(1)
                    .expect("id availability was preflighted");
                backend_ids.insert(replayed.backend_id.clone(), win);
                ledger.summon(win, ledger.active());
                win
            };
            windows.insert(win, replayed.metadata.clone());
            pending_assignments.insert(replayed.backend_id.clone(), win);
        }
        ledger.discard_undo_history();

        self.ledger = ledger;
        self.backend_ids = backend_ids;
        self.windows = windows;
        self.pending_assignments = pending_assignments;
        self.next_win_id = next_win_id;
        self.phase = RecoveryPhase::FinalizingReplay;
        self.projection_dirty = true;

        match self.finish_replay() {
            Ok(update) => Ok(update),
            Err(error) => {
                self.pending_backend_work = matches!(
                    &error,
                    SessionEventError::Backend(error) if should_retry_authoritative(error)
                );
                Err(error)
            }
        }
    }

    fn finish_replay(&mut self) -> Result<SessionUpdate, SessionEventError> {
        self.bind_pending_windows()?;
        let projection = self.project(&self.ledger, self.workarea);
        self.apply_projection_if_needed(projection)?;
        self.phase = RecoveryPhase::Live;
        let state = self.publish_first_visible_state();
        Ok(SessionUpdate {
            projection_applied: true,
            state: Some(state),
            ..SessionUpdate::unchanged()
        })
    }

    fn publish_first_visible_state(&mut self) -> RealmState {
        self.state = self.visible_state(1);
        self.published_ledger = self.ledger.clone();
        self.published_windows = self.windows.clone();
        self.state.clone()
    }

    fn repair_authoritative_projection(&mut self) -> Result<SessionUpdate, SessionEventError> {
        self.bind_pending_windows()?;
        let projection = self.project(&self.ledger, self.workarea);
        let projection_applied = self.apply_projection_if_needed(projection)?;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied,
            state,
            ..SessionUpdate::unchanged()
        })
    }

    fn record_window_identity(
        &mut self,
        backend_id: BackendWindowId,
    ) -> Result<WinId, SessionEventError> {
        if let Some(win) = self.backend_ids.get(&backend_id).copied() {
            return Ok(win);
        }

        let win = WinId(self.next_win_id);
        self.next_win_id = self
            .next_win_id
            .checked_add(1)
            .ok_or(SessionEventError::WindowIdExhausted)?;
        self.backend_ids.insert(backend_id.clone(), win);
        self.pending_assignments.insert(backend_id, win);
        Ok(win)
    }

    fn bind_pending_windows(&mut self) -> Result<(), BackendContractError> {
        let pending: Vec<_> = self
            .pending_assignments
            .iter()
            .map(|(backend_id, win)| (backend_id.clone(), *win))
            .collect();
        for (backend_id, win) in pending {
            self.backend.assign_window(&backend_id, win)?;
            self.pending_assignments.remove(&backend_id);
        }
        Ok(())
    }

    fn apply_projection_if_needed(&mut self, projection: Vec<Placement>) -> BackendResult<bool> {
        if !self.projection_dirty && projection == self.last_projection {
            return Ok(false);
        }

        if let Err(error) = self.backend.apply(&projection) {
            self.projection_dirty = true;
            return Err(error);
        }
        self.last_projection = projection;
        self.projection_dirty = false;
        Ok(true)
    }

    fn commit_desired(
        &mut self,
        ledger: Ledger,
        windows: BTreeMap<WinId, WindowMetadata>,
        workarea: Workarea,
    ) -> BackendResult<SessionUpdate> {
        debug_assert!(self.pending_assignments.is_empty());
        let projection = self.project(&ledger, workarea);
        let projection_applied = self.apply_projection_if_needed(projection)?;

        self.ledger = ledger;
        self.windows = windows;
        self.workarea = workarea;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied,
            state,
            ..SessionUpdate::unchanged()
        })
    }

    fn commit_observed(
        &mut self,
        ledger: Ledger,
        windows: BTreeMap<WinId, WindowMetadata>,
        workarea: Workarea,
    ) -> Result<SessionUpdate, SessionEventError> {
        let projection = self.project(&ledger, workarea);

        self.ledger = ledger;
        self.windows = windows;
        self.workarea = workarea;

        self.bind_pending_windows()?;
        let projection_applied = self.apply_projection_if_needed(projection)?;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied,
            state,
            ..SessionUpdate::unchanged()
        })
    }

    fn project(&self, ledger: &Ledger, workarea: Workarea) -> Vec<Placement> {
        project(ledger.active_orbit(), workarea, TriptychParams::default())
    }

    fn commit_visible_state(&mut self) -> SessionUpdate {
        let candidate = self.visible_state(self.state.revision);
        self.published_ledger = self.ledger.clone();
        self.published_windows = self.windows.clone();

        if self.state.renders_same_as(&candidate) {
            return SessionUpdate::unchanged();
        }

        let mut changed = candidate;
        changed.revision = self.state.revision.saturating_add(1);
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            ..SessionUpdate::unchanged()
        }
    }

    fn visible_state(&self, revision: u64) -> RealmState {
        RealmState {
            revision,
            orbits: self
                .ledger
                .orbits()
                .iter()
                .map(|orbit| OrbitCell {
                    number: orbit.id.human(),
                    rune: orbit.id.rune().to_string(),
                    display: if orbit.id == self.ledger.active() {
                        OrbitDisplay::Active
                    } else if orbit.occupied() {
                        OrbitDisplay::Occupied
                    } else {
                        OrbitDisplay::Empty
                    },
                    windows: orbit.windows.len(),
                })
                .collect(),
            layout: self.ledger.active_orbit().layout,
            mode: self.state.mode,
            focused_title: self
                .ledger
                .focused()
                .and_then(|win| self.windows.get(&win))
                .map(|metadata| metadata.title.clone())
                .unwrap_or_default(),
            chord_echo: self.state.chord_echo.clone(),
            whichkey: self.whichkey,
            modules: self.modules.clone(),
        }
    }

    fn current_authority(&self) -> AuthorityState {
        AuthorityState {
            ledger: self.ledger.clone(),
            windows: self.windows.clone(),
            backend_ids: self.backend_ids.clone(),
            bound_backend_ids: self.bound_backend_ids.clone(),
            next_win_id: self.next_win_id,
            workarea: self.workarea_known.then_some(self.workarea),
            exclusive_focus: self.exclusive_focus,
            effective_focus: self.effective_focus.clone(),
            mode: self.mode,
            chord_echo: self.chord_echo.clone(),
            whichkey: self.state.whichkey,
            modules: self.state.modules.clone(),
            active_modifiers: self.active_modifiers.clone(),
            held_bindings: self.held_bindings.clone(),
        }
    }

    fn install_authority(&mut self, authority: AuthorityState) {
        self.ledger = authority.ledger;
        self.windows = authority.windows;
        self.backend_ids = authority.backend_ids;
        self.bound_backend_ids = authority.bound_backend_ids;
        self.next_win_id = authority.next_win_id;
        if let Some(workarea) = authority.workarea {
            self.workarea = workarea;
            self.workarea_known = true;
        } else {
            self.workarea_known = false;
        }
        self.exclusive_focus = authority.exclusive_focus;
        self.effective_focus = authority.effective_focus;
        self.mode = authority.mode;
        self.chord_echo = authority.chord_echo;
        self.held_bindings = authority.held_bindings;
        self.whichkey = authority.whichkey;
        self.modules = authority.modules;
        self.active_modifiers = authority.active_modifiers;
    }

    fn new_active_transaction(&self) -> ActiveTransaction {
        let committed = self.current_authority();
        ActiveTransaction {
            committed: committed.clone(),
            visible: self.state.clone(),
            last_committed_clean_projection: self.last_projection.clone(),
            working: committed,
            most_recent_private_projection: self.last_projection.clone(),
            original_action_result: None,
            repair_available: true,
            response: BackendPolicyResponse {
                projection: None,
                closes: Vec::new(),
                bindings: self.binding_state.clone(),
            },
            staged_effects: Vec::new(),
            close_target: None,
            repeat_candidate: None,
            repeat_timer: RepeatTimerDirective::Preserve,
            projection_required: false,
            binding_changed: false,
            bootstrap: false,
            quit_staged: false,
            terminal_result: None,
        }
    }

    fn allocate_ticket(&mut self) -> Result<BackendTicket, ()> {
        let raw = self.last_backend_ticket.checked_add(1).ok_or(())?;
        let ticket = BackendTicket::new(raw).ok_or(())?;
        self.last_backend_ticket = raw;
        Ok(ticket)
    }

    fn allocate_action_ticket(&mut self) -> SessionActionResult<BackendTicket> {
        self.allocate_ticket()
            .map_err(|()| SessionActionError::BackendTicketExhausted)
    }

    fn allocate_event_ticket(&mut self) -> Result<BackendTicket, SessionEventError> {
        self.allocate_ticket()
            .map_err(|()| SessionEventError::BackendTicketExhausted)
    }

    fn enabled_bindings(&self, mode: Mode) -> Vec<BackendBindingId> {
        self.binding_order
            .iter()
            .copied()
            .filter(|id| {
                self.bindings
                    .get(id)
                    .is_some_and(|binding| binding.mode == mode)
            })
            .collect()
    }

    fn desired_binding_state(
        &self,
        authority: &AuthorityState,
        next_key_edge: BackendNextKeyEdge,
    ) -> BackendBindingState {
        if authority.workarea.is_none() || self.phase == RecoveryPhase::InitialReplay {
            return BackendBindingState {
                enabled: Vec::new(),
                watched_modifiers: Vec::new(),
                next_key_edge,
            };
        }
        BackendBindingState {
            enabled: self.enabled_bindings(authority.mode),
            watched_modifiers: vec![BackendModifier::Super],
            next_key_edge,
        }
    }

    fn pending_chord_echo(&self) -> String {
        format!("{}+ ▸ awaiting chord…", self.modifier_label)
    }

    fn project_authority(&self, authority: &AuthorityState) -> Vec<Placement> {
        let Some(workarea) = authority.workarea else {
            return Vec::new();
        };
        let mut projection = project(
            authority.ledger.active_orbit(),
            workarea,
            TriptychParams::default(),
        );
        if authority.exclusive_focus {
            for placement in &mut projection {
                placement.focused = false;
            }
        }
        projection
    }

    fn stage_policy_ledger_update(
        &mut self,
        update: impl FnOnce(&mut Ledger),
    ) -> SessionActionResult<SessionUpdate> {
        let mut active = self.new_active_transaction();
        update(&mut active.working.ledger);
        if active.working.ledger == active.committed.ledger {
            return Ok(SessionUpdate::unchanged());
        }
        active.most_recent_private_projection = self.project_authority(&active.working);
        active.projection_required =
            active.most_recent_private_projection != active.last_committed_clean_projection;

        if !active.projection_required {
            let before = self.snapshot();
            self.install_authority(active.working);
            let state = self.publish_current_state(false);
            let persistence = changed_snapshot(before, self.snapshot());
            return Ok(SessionUpdate {
                persistence,
                state,
                ..SessionUpdate::unchanged()
            });
        }

        let ticket = self.allocate_action_ticket()?;
        active.original_action_result = Some((ticket, Ok(())));
        self.active = Some(active);
        self.transaction = TransactionSubstate::AwaitingExternalTurn { ticket };
        match self.backend.request_policy_turn() {
            Ok(()) => Ok(SessionUpdate {
                pending_action: Some(ticket),
                ..SessionUpdate::unchanged()
            }),
            Err(error) => {
                self.active = None;
                self.transaction = TransactionSubstate::Idle;
                Err(SessionActionError::Backend(error))
            }
        }
    }

    fn handle_policy_turn(
        &mut self,
        turn: BackendPolicyTurn,
    ) -> Result<SessionUpdate, SessionEventError> {
        self.validate_policy_turn(&turn)?;

        let carries_staged_projection = matches!(
            self.transaction,
            TransactionSubstate::AwaitingExternalTurn { .. }
                | TransactionSubstate::AwaitingInternalTurn { .. }
                | TransactionSubstate::AwaitingRepairTurn { .. }
        );

        let response_ticket = match self.transaction {
            TransactionSubstate::Idle => self.allocate_event_ticket()?,
            TransactionSubstate::AwaitingExternalTurn { ticket }
            | TransactionSubstate::AwaitingInternalTurn { ticket }
            | TransactionSubstate::AwaitingRepairTurn { ticket } => ticket,
            TransactionSubstate::AwaitingDrain { .. } => self.allocate_event_ticket()?,
            TransactionSubstate::InFlight { .. } | TransactionSubstate::RetryReady => {
                return Err(BackendContractError::InvalidPolicySequence.into());
            }
        };

        let mut active = self
            .active
            .take()
            .unwrap_or_else(|| self.new_active_transaction());
        active.response.projection = None;
        active.response.closes.clear();
        active.response.bindings.next_key_edge = BackendNextKeyEdge::Preserve;
        if !carries_staged_projection {
            active.projection_required = false;
        }
        active.binding_changed = false;
        active.terminal_result = None;
        if let Some(close) = active.close_target {
            if active.working.windows.contains_key(&close) {
                push_unique(&mut active.response.closes, close);
            }
        }

        if self.phase == RecoveryPhase::InitialReplay {
            self.reduce_initial_replay(&mut active, &turn.events)?;
        } else {
            self.reduce_policy_events(&mut active, &turn.events)?;
        }
        self.derive_response(&mut active);
        let response = active.response.clone();
        self.last_policy_turn = Some(turn.id);
        let submission = self
            .backend
            .respond_policy_turn(turn.id, response_ticket, response)
            .map_err(SessionEventError::Backend)?;

        active.close_target = None;
        match submission {
            BackendSubmission::Complete => self.finalize_active(active),
            BackendSubmission::Pending => {
                let update = SessionUpdate {
                    repeat_timer: active.repeat_timer.clone(),
                    ..SessionUpdate::unchanged()
                };
                active.repeat_timer = RepeatTimerDirective::Preserve;
                if active.bootstrap {
                    self.phase = RecoveryPhase::FinalizingReplay;
                }
                self.active = Some(active);
                self.transaction = TransactionSubstate::InFlight {
                    ticket: response_ticket,
                };
                Ok(update)
            }
        }
    }

    fn validate_policy_turn(&self, turn: &BackendPolicyTurn) -> Result<(), SessionEventError> {
        if self
            .last_policy_turn
            .is_some_and(|last| turn.id.get() <= last.get())
        {
            return Err(BackendContractError::InvalidPolicySequence.into());
        }
        match self.transaction {
            TransactionSubstate::Idle
            | TransactionSubstate::AwaitingExternalTurn { .. }
            | TransactionSubstate::AwaitingInternalTurn { .. }
            | TransactionSubstate::AwaitingRepairTurn { .. }
                if turn.drains.is_none() => {}
            TransactionSubstate::AwaitingDrain { ticket } if turn.drains == Some(ticket) => {}
            _ => return Err(BackendContractError::InvalidPolicySequence.into()),
        }

        let replay = self.phase == RecoveryPhase::InitialReplay;
        if replay {
            let barriers = turn
                .events
                .iter()
                .filter(|event| matches!(event, BackendPolicyEvent::InitialReplayComplete))
                .count();
            if barriers != 1
                || !matches!(
                    turn.events.last(),
                    Some(BackendPolicyEvent::InitialReplayComplete)
                )
            {
                return Err(BackendContractError::InvalidPolicySequence.into());
            }
            let mut focus_events = 0_usize;
            let mut exclusive_events = 0_usize;
            for event in &turn.events {
                match event {
                    BackendPolicyEvent::WindowOpened { .. }
                    | BackendPolicyEvent::InitialReplayComplete => {}
                    BackendPolicyEvent::FocusChanged(_) => {
                        focus_events += 1;
                        if focus_events > 1 {
                            return Err(BackendContractError::InvalidPolicySequence.into());
                        }
                    }
                    BackendPolicyEvent::ExclusiveFocusChanged(_) => {
                        exclusive_events += 1;
                        if exclusive_events > 1 {
                            return Err(BackendContractError::InvalidPolicySequence.into());
                        }
                    }
                    event => {
                        return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                            event.clone(),
                        ));
                    }
                }
            }
        } else if turn
            .events
            .iter()
            .any(|event| matches!(event, BackendPolicyEvent::InitialReplayComplete))
        {
            return Err(SessionEventError::RepeatedInitialReplayComplete);
        }

        let limit = if replay {
            MAX_REPLAY_POLICY_EVENTS
        } else {
            MAX_POLICY_EVENTS
        };
        if turn.events.len() > limit {
            return Err(capacity_error(BackendCapacityResource::PolicyFacts, limit));
        }

        let base = self
            .active
            .as_ref()
            .map_or_else(|| self.current_authority(), |active| active.working.clone());
        let mut known: BTreeSet<_> = if replay {
            BTreeSet::new()
        } else {
            base.backend_ids
                .iter()
                .filter_map(|(id, win)| {
                    (base.bound_backend_ids.contains(id) && base.windows.contains_key(win))
                        .then(|| id.clone())
                })
                .collect()
        };
        let mut live = known.clone();
        let mut mapped = base.backend_ids.keys().cloned().collect::<BTreeSet<_>>();
        let mut next_win_id = base.next_win_id;
        let mut text_bytes = 0_usize;
        let mut effects = self
            .active
            .as_ref()
            .map_or(0, |active| active.staged_effects.len());
        let mut quit_seen = self
            .active
            .as_ref()
            .is_some_and(|active| active.quit_staged);

        for event in &turn.events {
            match event {
                BackendPolicyEvent::WindowOpened {
                    backend_id,
                    app_id,
                    title,
                } => {
                    text_bytes = text_bytes
                        .saturating_add(backend_id.as_str().len())
                        .saturating_add(app_id.len())
                        .saturating_add(title.len());
                    known.insert(backend_id.clone());
                    live.insert(backend_id.clone());
                    if mapped.insert(backend_id.clone()) {
                        if next_win_id == u64::MAX {
                            return Err(SessionEventError::WindowIdExhausted);
                        }
                        next_win_id += 1;
                    }
                    if live.len() > MAX_MANAGED_WINDOWS {
                        return Err(capacity_error(
                            BackendCapacityResource::ManagedWindows,
                            MAX_MANAGED_WINDOWS,
                        ));
                    }
                }
                BackendPolicyEvent::WindowClosed(id) => {
                    text_bytes = text_bytes.saturating_add(id.as_str().len());
                    if !known.remove(id) {
                        return Err(BackendContractError::UnknownWindowReference.into());
                    }
                    live.remove(id);
                    mapped.remove(id);
                    if replay {
                        return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                            event.clone(),
                        ));
                    }
                }
                BackendPolicyEvent::TitleChanged { backend_id, title } => {
                    text_bytes = text_bytes
                        .saturating_add(backend_id.as_str().len())
                        .saturating_add(title.len());
                    if replay {
                        return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                            event.clone(),
                        ));
                    }
                    if !known.contains(backend_id) {
                        return Err(BackendContractError::UnknownWindowReference.into());
                    }
                }
                BackendPolicyEvent::FocusChanged(Some(id)) => {
                    text_bytes = text_bytes.saturating_add(id.as_str().len());
                    if !known.contains(id) {
                        return Err(BackendContractError::UnknownWindowReference.into());
                    }
                }
                BackendPolicyEvent::FocusChanged(None)
                | BackendPolicyEvent::ExclusiveFocusChanged(_) => {}
                BackendPolicyEvent::WorkareaChanged(_) => {
                    if replay {
                        return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                            event.clone(),
                        ));
                    }
                }
                BackendPolicyEvent::GeometryDrifted { backend_id, .. } => {
                    text_bytes = text_bytes.saturating_add(backend_id.as_str().len());
                    if replay {
                        return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                            event.clone(),
                        ));
                    }
                    if !known.contains(backend_id) {
                        return Err(BackendContractError::UnknownWindowReference.into());
                    }
                }
                BackendPolicyEvent::BindingPressed(id)
                | BackendPolicyEvent::BindingReleased(id)
                | BackendPolicyEvent::BindingRepeatStopped(id) => {
                    let Some(binding) = self.bindings.get(id) else {
                        return Err(BackendContractError::UnknownBinding { id: *id }.into());
                    };
                    if self.phase != RecoveryPhase::Live {
                        return Err(BackendContractError::InvalidPolicySequence.into());
                    }
                    if matches!(event, BackendPolicyEvent::BindingPressed(_)) && !quit_seen {
                        if action_has_effect(&binding.action) {
                            effects = effects.saturating_add(1);
                        }
                        if matches!(binding.action, Action::Quit) {
                            quit_seen = true;
                        }
                    }
                }
                BackendPolicyEvent::UnboundKeyEaten => {
                    if self.phase != RecoveryPhase::Live {
                        return Err(BackendContractError::InvalidPolicySequence.into());
                    }
                }
                BackendPolicyEvent::ModifiersChanged { old, new } => {
                    if !canonical_modifiers(old) || !canonical_modifiers(new) {
                        return Err(BackendContractError::NonCanonicalModifiers.into());
                    }
                }
                BackendPolicyEvent::InitialReplayComplete => {}
            }
        }
        if text_bytes > MAX_POLICY_TEXT_BYTES {
            return Err(capacity_error(
                BackendCapacityResource::PolicyTextBytes,
                MAX_POLICY_TEXT_BYTES,
            ));
        }
        if effects > MAX_STAGED_EFFECTS {
            return Err(capacity_error(
                BackendCapacityResource::PolicyEffects,
                MAX_STAGED_EFFECTS,
            ));
        }
        Ok(())
    }

    fn reduce_initial_replay(
        &mut self,
        active: &mut ActiveTransaction,
        events: &[BackendPolicyEvent],
    ) -> Result<(), SessionEventError> {
        let mut replayed = Vec::<ReplayWindow>::new();
        let mut effective_focus = None;
        let mut exclusive_focus = false;
        for event in events {
            match event {
                BackendPolicyEvent::WindowOpened {
                    backend_id,
                    app_id,
                    title,
                } => {
                    let metadata = WindowMetadata {
                        app_id: normalize_visible(app_id, MAX_VISIBLE_APP_ID_JSON_BYTES),
                        title: normalize_visible(title, MAX_VISIBLE_TITLE_JSON_BYTES),
                    };
                    if let Some(existing) = replayed
                        .iter_mut()
                        .find(|window| window.backend_id == *backend_id)
                    {
                        existing.metadata = metadata;
                    } else {
                        replayed.push(ReplayWindow {
                            backend_id: backend_id.clone(),
                            metadata,
                        });
                    }
                }
                BackendPolicyEvent::FocusChanged(focus) => effective_focus = focus.clone(),
                BackendPolicyEvent::ExclusiveFocusChanged(exclusive) => {
                    exclusive_focus = *exclusive
                }
                BackendPolicyEvent::InitialReplayComplete => {}
                event => {
                    return Err(SessionEventError::UnexpectedInitialReplayPolicyEvent(
                        event.clone(),
                    ));
                }
            }
        }

        let replayed_ids: BTreeSet<_> = replayed
            .iter()
            .map(|window| window.backend_id.clone())
            .collect();
        let unknown = replayed
            .iter()
            .filter(|window| !active.working.backend_ids.contains_key(&window.backend_id))
            .count();
        if unknown as u128 > u128::from(u64::MAX - active.working.next_win_id) {
            return Err(SessionEventError::WindowIdExhausted);
        }

        let persisted_order = active
            .working
            .ledger
            .orbits()
            .iter()
            .flat_map(|orbit| orbit.windows.iter().copied())
            .collect::<Vec<_>>();
        for win in persisted_order {
            let backend_id = active
                .working
                .backend_ids
                .iter()
                .find_map(|(id, assigned)| (*assigned == win).then(|| id.clone()))
                .expect("validated snapshot covers every ledger window");
            if !replayed_ids.contains(&backend_id) {
                active.working.ledger.banish(win);
                active.working.backend_ids.remove(&backend_id);
            }
        }
        active.working.windows.clear();
        active.working.bound_backend_ids.clear();
        for window in replayed {
            let win = if let Some(win) = active.working.backend_ids.get(&window.backend_id).copied()
            {
                win
            } else {
                let win = WinId(active.working.next_win_id);
                active.working.next_win_id = active
                    .working
                    .next_win_id
                    .checked_add(1)
                    .expect("replay allocation was preflighted");
                active
                    .working
                    .backend_ids
                    .insert(window.backend_id.clone(), win);
                active
                    .working
                    .ledger
                    .summon(win, active.working.ledger.active());
                win
            };
            active.working.windows.insert(win, window.metadata);
            self.backend.assign_window(&window.backend_id, win)?;
            active.working.bound_backend_ids.insert(window.backend_id);
        }
        active.working.ledger.discard_undo_history();
        active.working.workarea = None;
        active.working.effective_focus = effective_focus;
        active.working.exclusive_focus = exclusive_focus;
        active.bootstrap = true;
        active.response.projection = None;
        active.response.closes.clear();
        Ok(())
    }

    fn reduce_policy_events(
        &mut self,
        active: &mut ActiveTransaction,
        events: &[BackendPolicyEvent],
    ) -> Result<(), SessionEventError> {
        for event in events {
            match event {
                BackendPolicyEvent::WindowOpened {
                    backend_id,
                    app_id,
                    title,
                } => {
                    let win = if let Some(win) = active.working.backend_ids.get(backend_id).copied()
                    {
                        win
                    } else {
                        if active.working.next_win_id == u64::MAX {
                            return Err(SessionEventError::WindowIdExhausted);
                        }
                        let win = WinId(active.working.next_win_id);
                        active.working.next_win_id += 1;
                        active.working.backend_ids.insert(backend_id.clone(), win);
                        win
                    };
                    active
                        .working
                        .ledger
                        .summon(win, active.working.ledger.active());
                    active.working.windows.insert(
                        win,
                        WindowMetadata {
                            app_id: normalize_visible(app_id, MAX_VISIBLE_APP_ID_JSON_BYTES),
                            title: normalize_visible(title, MAX_VISIBLE_TITLE_JSON_BYTES),
                        },
                    );
                    if !active.working.bound_backend_ids.contains(backend_id) {
                        self.backend.assign_window(backend_id, win)?;
                        active.working.bound_backend_ids.insert(backend_id.clone());
                    }
                    if active.working.workarea.is_some() {
                        active.projection_required = true;
                    }
                }
                BackendPolicyEvent::WindowClosed(backend_id) => {
                    let win = active
                        .working
                        .backend_ids
                        .remove(backend_id)
                        .ok_or(BackendContractError::UnknownWindowReference)?;
                    active.working.bound_backend_ids.remove(backend_id);
                    active.working.windows.remove(&win);
                    active.working.ledger.banish(win);
                    active.response.closes.retain(|close| *close != win);
                    if active.working.workarea.is_some() {
                        active.projection_required = true;
                    }
                    if active.close_target == Some(win) {
                        active.close_target = None;
                    }
                }
                BackendPolicyEvent::TitleChanged { backend_id, title } => {
                    let win = *active
                        .working
                        .backend_ids
                        .get(backend_id)
                        .ok_or(BackendContractError::UnknownWindowReference)?;
                    active
                        .working
                        .windows
                        .get_mut(&win)
                        .ok_or(BackendContractError::UnknownWindowReference)?
                        .title = normalize_visible(title, MAX_VISIBLE_TITLE_JSON_BYTES);
                }
                BackendPolicyEvent::FocusChanged(focus) => {
                    active.working.effective_focus = focus.clone();
                    if !active.working.exclusive_focus {
                        let observed = focus
                            .as_ref()
                            .and_then(|id| active.working.backend_ids.get(id))
                            .copied();
                        if observed != active.working.ledger.focused()
                            && active.working.workarea.is_some()
                        {
                            active.projection_required = true;
                        }
                    }
                }
                BackendPolicyEvent::ExclusiveFocusChanged(exclusive) => {
                    if active.working.exclusive_focus != *exclusive
                        && active.working.workarea.is_some()
                    {
                        active.projection_required = true;
                    }
                    active.working.exclusive_focus = *exclusive;
                }
                BackendPolicyEvent::WorkareaChanged(workarea) => {
                    if active.working.workarea != Some(*workarea) {
                        active.working.workarea = Some(*workarea);
                        active.projection_required = true;
                    }
                }
                BackendPolicyEvent::GeometryDrifted { .. } => {}
                BackendPolicyEvent::BindingPressed(id) => {
                    active.working.held_bindings.insert(*id);
                    let binding = self
                        .bindings
                        .get(id)
                        .cloned()
                        .ok_or(BackendContractError::UnknownBinding { id: *id })?;
                    self.apply_binding_action(active, *id, &binding);
                    if active.working.mode != Mode::Nav
                        && active.response.bindings.next_key_edge == BackendNextKeyEdge::Preserve
                    {
                        active.response.bindings.next_key_edge = BackendNextKeyEdge::Ensure;
                    }
                }
                BackendPolicyEvent::BindingReleased(id)
                | BackendPolicyEvent::BindingRepeatStopped(id) => {
                    active.working.held_bindings.remove(id);
                    if active.repeat_candidate == Some(*id) {
                        active.repeat_timer = RepeatTimerDirective::Disarm;
                    }
                    if self.repeat_target == Some(*id) {
                        self.repeat_target = None;
                        active.repeat_timer = RepeatTimerDirective::Disarm;
                    }
                }
                BackendPolicyEvent::UnboundKeyEaten => {
                    if active.working.mode != Mode::Nav {
                        active.working.mode = Mode::Nav;
                        active.working.chord_echo.clear();
                        active.binding_changed = true;
                    }
                    active.response.bindings.next_key_edge = BackendNextKeyEdge::Preserve;
                }
                BackendPolicyEvent::ModifiersChanged { new, .. } => {
                    active.working.active_modifiers = new.clone();
                    if active.working.mode == Mode::Nav {
                        active.working.chord_echo = if new.contains(&BackendModifier::Super) {
                            self.pending_chord_echo()
                        } else {
                            String::new()
                        };
                    }
                }
                BackendPolicyEvent::InitialReplayComplete => {
                    return Err(SessionEventError::RepeatedInitialReplayComplete);
                }
            }
        }
        Ok(())
    }

    fn apply_binding_action(
        &self,
        active: &mut ActiveTransaction,
        id: BackendBindingId,
        binding: &Binding,
    ) {
        if active.quit_staged {
            return;
        }
        let before_projection = self.project_authority(&active.working);
        match &binding.action {
            Action::Spawn(argv) => active
                .staged_effects
                .push(SessionEffect::Spawn(argv.clone())),
            Action::Launcher => active.staged_effects.push(SessionEffect::Launcher),
            Action::Focus(direction) => active.working.ledger.focus_step(*direction),
            Action::Swap(direction) => {
                active.working.ledger.swap(*direction);
            }
            Action::Orbit(number) => {
                if let Some(orbit) = OrbitId::from_human(*number) {
                    active.working.ledger.switch_orbit(orbit);
                }
            }
            Action::MoveToOrbit(number) => {
                if let Some(orbit) = OrbitId::from_human(*number) {
                    active.working.ledger.move_to_orbit(orbit);
                }
            }
            Action::Stow => {
                active.working.ledger.toggle_stow();
            }
            Action::SetLayout(layout) => {
                active.working.ledger.set_layout(*layout);
            }
            Action::Fullscreen => {
                active.working.ledger.toggle_fullscreen();
            }
            Action::EnterMode(mode) => {
                active.working.mode = *mode;
                active.working.chord_echo = if *mode != Mode::Nav
                    || active
                        .working
                        .active_modifiers
                        .contains(&BackendModifier::Super)
                {
                    self.pending_chord_echo()
                } else {
                    String::new()
                };
                active.binding_changed = true;
                active.response.bindings.next_key_edge = if *mode == Mode::Nav {
                    BackendNextKeyEdge::Preserve
                } else {
                    BackendNextKeyEdge::Ensure
                };
            }
            Action::Banish => {
                if let Some(win) = active.working.ledger.focused() {
                    push_unique(&mut active.response.closes, win);
                }
            }
            Action::Undo => {
                active.working.ledger.undo();
            }
            Action::ToggleWhichKey => active.working.whichkey = !active.working.whichkey,
            Action::Grimoire => active.staged_effects.push(SessionEffect::Grimoire),
            Action::ReloadTheme => active.staged_effects.push(SessionEffect::ReloadTheme),
            Action::Quit => {
                let after = active
                    .original_action_result
                    .as_ref()
                    .map_or(QuitAfter::NoRequester, |(ticket, _)| {
                        QuitAfter::OriginalAction(*ticket)
                    });
                active
                    .staged_effects
                    .push(SessionEffect::QuitPending { after });
                active.quit_staged = true;
            }
        }
        let after_projection = self.project_authority(&active.working);
        if before_projection != after_projection {
            active.projection_required = true;
        }
        if binding.repeatable {
            active.repeat_candidate = Some(id);
        }
    }

    fn derive_response(&self, active: &mut ActiveTransaction) {
        active.most_recent_private_projection = self.project_authority(&active.working);
        if active.projection_required && active.working.workarea.is_some() {
            active.response.projection = Some(active.most_recent_private_projection.clone());
        }
        let edge = active.response.bindings.next_key_edge;
        active.response.bindings = self.desired_binding_state(&active.working, edge);
    }

    fn finalize_active(
        &mut self,
        mut active: ActiveTransaction,
    ) -> Result<SessionUpdate, SessionEventError> {
        debug_assert_eq!(active.visible, self.state);
        let _repair_available = active.repair_available;
        let before_snapshot = self.snapshot();
        let entering_live = self.phase == RecoveryPhase::FinalizingReplay
            && active.working.workarea.is_some()
            && active.response.projection.is_some();
        self.install_authority(active.working.clone());
        if let Some(projection) = active.response.projection.clone() {
            self.last_projection = projection;
            self.projection_dirty = false;
        }
        self.binding_state = BackendBindingState {
            enabled: active.response.bindings.enabled.clone(),
            watched_modifiers: active.response.bindings.watched_modifiers.clone(),
            next_key_edge: BackendNextKeyEdge::Preserve,
        };
        if entering_live {
            self.phase = RecoveryPhase::Live;
        } else if active.bootstrap {
            self.phase = RecoveryPhase::FinalizingReplay;
        }

        let mut repeat_timer = active.repeat_timer;
        if let Some(candidate) = active.repeat_candidate {
            let can_arm = self.held_bindings.contains(&candidate)
                && self.binding_state.enabled.contains(&candidate)
                && self
                    .bindings
                    .get(&candidate)
                    .is_some_and(|binding| binding.repeatable);
            if can_arm {
                self.repeat_target = Some(candidate);
                repeat_timer = RepeatTimerDirective::Arm {
                    delay: Duration::from_millis(KEY_REPEAT_DELAY_MS),
                    interval: Duration::from_millis(1_000 / u64::from(KEY_REPEAT_RATE_HZ)),
                };
            } else {
                self.repeat_target = None;
                repeat_timer = RepeatTimerDirective::Disarm;
            }
        }
        if self
            .repeat_target
            .is_some_and(|target| !self.binding_state.enabled.contains(&target))
        {
            self.repeat_target = None;
            repeat_timer = RepeatTimerDirective::Disarm;
        }
        if active.quit_staged {
            self.repeat_target = None;
            repeat_timer = RepeatTimerDirective::Disarm;
        }

        let state = if self.phase == RecoveryPhase::Live {
            self.publish_current_state(entering_live)
        } else {
            None
        };
        let persistence = changed_snapshot(before_snapshot, self.snapshot());
        let action_completion = active
            .original_action_result
            .take()
            .map(|(ticket, result)| ActionCompletion { ticket, result });
        let effects = active.staged_effects;
        if active.quit_staged {
            self.phase = RecoveryPhase::QuitPending;
        }
        self.active = None;
        self.transaction = TransactionSubstate::Idle;
        Ok(SessionUpdate {
            repeat_timer,
            persistence,
            projection_applied: active.response.projection.is_some(),
            state,
            pending_action: None,
            action_completion,
            effects,
            diagnostic: None,
            deferred: None,
        })
    }

    fn handle_operation_completed(
        &mut self,
        ticket: BackendTicket,
        result: BackendResult<()>,
    ) -> Result<SessionUpdate, SessionEventError> {
        let TransactionSubstate::InFlight { ticket: expected } = self.transaction else {
            return Err(BackendContractError::InvalidPolicySequence.into());
        };
        if ticket != expected {
            return Err(BackendContractError::InvalidPolicySequence.into());
        }
        let active = self
            .active
            .as_mut()
            .ok_or(BackendContractError::InvalidPolicySequence)?;
        let diagnostic = match &result {
            Err(error @ (BackendError::Io { .. } | BackendError::Unsupported { .. }))
                if active.original_action_result.is_none() =>
            {
                Some(SessionDiagnostic {
                    error: error.clone(),
                })
            }
            Err(error @ (BackendError::Io { .. } | BackendError::Unsupported { .. })) => {
                if let Some((_, original)) = &mut active.original_action_result {
                    *original = Err(SessionActionError::Backend(error.clone()));
                }
                None
            }
            Err(error) => return Err(SessionEventError::Backend(error.clone())),
            Ok(()) => None,
        };
        active.terminal_result = Some(result);
        self.transaction = TransactionSubstate::AwaitingDrain { ticket };
        Ok(SessionUpdate {
            diagnostic,
            ..SessionUpdate::unchanged()
        })
    }

    fn handle_observations_drained(
        &mut self,
        ticket: BackendTicket,
    ) -> Result<SessionUpdate, SessionEventError> {
        let TransactionSubstate::AwaitingDrain { ticket: expected } = self.transaction else {
            return Err(BackendContractError::InvalidPolicySequence.into());
        };
        if ticket != expected {
            return Err(BackendContractError::InvalidPolicySequence.into());
        }
        let active = self
            .active
            .take()
            .ok_or(BackendContractError::InvalidPolicySequence)?;
        self.finalize_active(active)
    }

    /// Fire one coalesced repeat tick after rechecking the captured target.
    pub fn fire_key_repeat(&mut self) -> Result<SessionUpdate, SessionEventError> {
        if self.phase != RecoveryPhase::Live || self.has_active_backend_transaction() {
            return Ok(SessionUpdate::unchanged());
        }
        let Some(target) = self.repeat_target else {
            return Ok(SessionUpdate::unchanged());
        };
        let Some(binding) = self.bindings.get(&target).cloned() else {
            return Ok(SessionUpdate::unchanged());
        };
        if !binding.repeatable
            || !self.held_bindings.contains(&target)
            || !self.binding_state.enabled.contains(&target)
        {
            return Ok(SessionUpdate::unchanged());
        }

        let mut active = self.new_active_transaction();
        self.apply_binding_action(&mut active, target, &binding);
        active.repeat_candidate = None;
        active.most_recent_private_projection = self.project_authority(&active.working);
        if !active.projection_required
            && !active.binding_changed
            && active.response.closes.is_empty()
        {
            let before = self.snapshot();
            self.install_authority(active.working);
            let state = self.publish_current_state(false);
            return Ok(SessionUpdate {
                persistence: changed_snapshot(before, self.snapshot()),
                state,
                effects: active.staged_effects,
                ..SessionUpdate::unchanged()
            });
        }
        let ticket = self.allocate_event_ticket()?;
        self.active = Some(active);
        self.transaction = TransactionSubstate::AwaitingInternalTurn { ticket };
        match self.backend.request_policy_turn() {
            Ok(()) => Ok(SessionUpdate::unchanged()),
            Err(error @ (BackendError::Io { .. } | BackendError::Unsupported { .. })) => {
                self.active = None;
                self.transaction = TransactionSubstate::Idle;
                Ok(SessionUpdate {
                    diagnostic: Some(SessionDiagnostic { error }),
                    ..SessionUpdate::unchanged()
                })
            }
            Err(error) => {
                self.active = None;
                self.transaction = TransactionSubstate::Idle;
                Err(SessionEventError::Backend(error))
            }
        }
    }

    fn publish_current_state(&mut self, force_first: bool) -> Option<RealmState> {
        let mut candidate = self.policy_visible_state(self.state.revision);
        let changed = force_first || !self.state.renders_same_as(&candidate);
        self.published_ledger = self.ledger.clone();
        self.published_windows = self.windows.clone();
        if !changed {
            return None;
        }
        candidate.revision = if force_first {
            1
        } else {
            self.state.revision.saturating_add(1)
        };
        self.state = candidate.clone();
        Some(candidate)
    }

    fn policy_visible_state(&self, revision: u64) -> RealmState {
        RealmState {
            revision,
            orbits: self
                .ledger
                .orbits()
                .iter()
                .map(|orbit| OrbitCell {
                    number: orbit.id.human(),
                    rune: orbit.id.rune().to_string(),
                    display: if orbit.id == self.ledger.active() {
                        OrbitDisplay::Active
                    } else if orbit.occupied() {
                        OrbitDisplay::Occupied
                    } else {
                        OrbitDisplay::Empty
                    },
                    windows: orbit.windows.len(),
                })
                .collect(),
            layout: self.ledger.active_orbit().layout,
            mode: self.mode,
            focused_title: if self.exclusive_focus {
                String::new()
            } else {
                self.ledger
                    .focused()
                    .and_then(|win| self.windows.get(&win))
                    .map(|metadata| metadata.title.clone())
                    .unwrap_or_default()
            },
            chord_echo: self.chord_echo.clone(),
            whichkey: self.whichkey,
            modules: self.modules.clone(),
        }
    }

    /// Return the ledger at the last visible clean boundary.
    pub fn visible_ledger(&self, selected: Option<OrbitId>) -> Vec<OrbitLedger> {
        self.published_ledger
            .orbits()
            .iter()
            .filter(|orbit| selected.is_none_or(|selected| orbit.id == selected))
            .map(|orbit| OrbitLedger {
                orbit: orbit.id.human(),
                rune: orbit.id.rune().to_string(),
                name: orbit.name.clone(),
                windows: orbit
                    .windows
                    .iter()
                    .filter_map(|win| {
                        self.published_windows.get(win).map(|metadata| LedgerEntry {
                            id: *win,
                            app_id: metadata.app_id.clone(),
                            title: metadata.title.clone(),
                            focused: orbit.focused() == Some(*win),
                            stowed: orbit.stowed.contains(win),
                        })
                    })
                    .collect(),
            })
            .collect()
    }
}

fn capacity_error(resource: BackendCapacityResource, limit: usize) -> SessionEventError {
    SessionEventError::Backend(BackendError::Capacity {
        resource,
        limit: limit as u64,
    })
}

fn canonical_modifiers(modifiers: &[BackendModifier]) -> bool {
    modifiers.len() <= 4 && modifiers.windows(2).all(|pair| pair[0] < pair[1])
}

fn action_has_effect(action: &Action) -> bool {
    matches!(
        action,
        Action::Spawn(_) | Action::Launcher | Action::Grimoire | Action::ReloadTheme | Action::Quit
    )
}

fn push_unique(values: &mut Vec<WinId>, value: WinId) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn changed_snapshot(
    before: Option<SessionSnapshotV1>,
    after: Option<SessionSnapshotV1>,
) -> Option<SessionSnapshotV1> {
    (before != after).then_some(after).flatten()
}

fn normalize_visible(source: &str, cap: usize) -> String {
    let encoded = source.chars().map(json_content_char_len).sum::<usize>();
    if encoded <= cap {
        return source.to_owned();
    }
    const ELLIPSIS: char = '…';
    let ellipsis_len = ELLIPSIS.len_utf8();
    let mut normalized = String::new();
    let mut used = 0_usize;
    for character in source.chars() {
        let width = json_content_char_len(character);
        if used.saturating_add(width).saturating_add(ellipsis_len) > cap {
            break;
        }
        normalized.push(character);
        used += width;
    }
    normalized.push(ELLIPSIS);
    normalized
}

fn json_content_char_len(character: char) -> usize {
    match character {
        '"' | '\\' | '\u{0008}' | '\u{0009}' | '\u{000a}' | '\u{000c}' | '\u{000d}' => 2,
        '\u{0000}'..='\u{001f}' => 6,
        character => character.len_utf8(),
    }
}

fn should_retry_authoritative(error: &BackendError) -> bool {
    matches!(error, BackendError::Io { .. })
}

fn should_retry_desired_repair(error: &BackendError) -> bool {
    !matches!(
        error,
        BackendError::Disconnected | BackendError::Unavailable { .. }
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::fs::File;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use realm_core::ipc::{self, Capabilities, Response, PROTOCOL_VERSION};
    use realm_core::keys::{Action, Binding, Keymap, Mode};
    use realm_core::layout::{project, Layout, Placement, Rect, TriptychParams, Workarea};
    use realm_core::ledger::{Dir, ORBIT_COUNT};
    use realm_core::state::{Module, RealmState};
    use realm_core::{Ledger, OrbitId, WinId};

    use crate::backend::{
        BackendBindingId, BackendBindingSpec, BackendCapacityResource, BackendContractError,
        BackendError, BackendEvent, BackendExitPolicy, BackendModifier, BackendNextKeyEdge,
        BackendPolicyEvent, BackendPolicyResponse, BackendPolicyTurn, BackendPolicyTurnId,
        BackendPollInterest, BackendReady, BackendResult, BackendSubmission, BackendTicket,
        BackendWindowId, WmBackend, KEY_REPEAT_DELAY_MS, KEY_REPEAT_RATE_HZ,
        MAX_CONFIGURED_BINDINGS, MAX_MANAGED_WINDOWS, MAX_POLICY_EVENTS, MAX_POLICY_TEXT_BYTES,
        MAX_REPLAY_POLICY_EVENTS, MAX_STAGED_EFFECTS, MAX_VISIBLE_APP_ID_JSON_BYTES,
        MAX_VISIBLE_TITLE_JSON_BYTES,
    };

    use super::{
        RecoveryPhase, RepeatTimerDirective, Session, SessionActionError, SessionEffect,
        SessionEventError, SessionSnapshotV1, SessionUpdate, SnapshotBinding, TransactionSubstate,
    };

    struct FakeBackend {
        connect_calls: usize,
        capabilities: Capabilities,
        workarea: Workarea,
        apply_attempts: Vec<Vec<Placement>>,
        successful_frames: Vec<Vec<Placement>>,
        fail_next_apply: Option<BackendError>,
        fail_next_assign: Option<BackendContractError>,
        assignment_attempts: Vec<(BackendWindowId, WinId)>,
        bound_windows: BTreeMap<BackendWindowId, WinId>,
        focus_calls: usize,
        close_attempts: Vec<WinId>,
        fail_next_close: Option<BackendError>,
        configured_bindings: Vec<Vec<BackendBindingSpec>>,
        configure_calls: Arc<Mutex<usize>>,
        request_attempts: usize,
        request_results: VecDeque<BackendResult<()>>,
        responses: Vec<(BackendPolicyTurnId, BackendTicket, BackendPolicyResponse)>,
        response_results: VecDeque<BackendResult<BackendSubmission>>,
        call_order: Vec<String>,
        next_event_calls: usize,
        event_file: File,
    }

    impl FakeBackend {
        fn new() -> Self {
            Self {
                connect_calls: 0,
                capabilities: Capabilities {
                    exact_geometry: true,
                    server_side_borders: true,
                    hide_show: true,
                    explicit_ordering: true,
                    fullscreen: true,
                    unsupported: Vec::new(),
                },
                workarea: Workarea::new(1920, 1080, 32, 26),
                apply_attempts: Vec::new(),
                successful_frames: Vec::new(),
                fail_next_apply: None,
                fail_next_assign: None,
                assignment_attempts: Vec::new(),
                bound_windows: BTreeMap::new(),
                focus_calls: 0,
                close_attempts: Vec::new(),
                fail_next_close: None,
                configured_bindings: Vec::new(),
                configure_calls: Arc::new(Mutex::new(0)),
                request_attempts: 0,
                request_results: VecDeque::new(),
                responses: Vec::new(),
                response_results: VecDeque::new(),
                call_order: Vec::new(),
                next_event_calls: 0,
                event_file: File::open("/dev/null").unwrap(),
            }
        }
    }

    impl WmBackend for FakeBackend {
        fn name(&self) -> &str {
            "fake"
        }

        fn connect(&mut self) -> BackendResult<Capabilities> {
            self.connect_calls += 1;
            Ok(self.capabilities.clone())
        }

        fn assign_window(
            &mut self,
            backend_id: &BackendWindowId,
            win: WinId,
        ) -> Result<(), BackendContractError> {
            self.assignment_attempts.push((backend_id.clone(), win));
            self.call_order
                .push(format!("assign:{}:{}", backend_id.as_str(), win.0));
            if let Some(error) = self.fail_next_assign.take() {
                return Err(error);
            }
            if let Some(bound) = self.bound_windows.get(backend_id) {
                if *bound != win {
                    return Err(BackendContractError::ConflictingBackendIdentity);
                }
                return Ok(());
            }
            self.bound_windows.insert(backend_id.clone(), win);
            Ok(())
        }

        fn configure_bindings(&mut self, bindings: Vec<BackendBindingSpec>) -> BackendResult<()> {
            *self.configure_calls.lock().unwrap() += 1;
            self.call_order.push("configure".to_owned());
            self.configured_bindings.push(bindings);
            Ok(())
        }

        fn request_policy_turn(&mut self) -> BackendResult<()> {
            self.request_attempts += 1;
            self.call_order.push("request".to_owned());
            self.request_results.pop_front().unwrap_or(Ok(()))
        }

        fn respond_policy_turn(
            &mut self,
            turn: BackendPolicyTurnId,
            ticket: BackendTicket,
            response: BackendPolicyResponse,
        ) -> BackendResult<BackendSubmission> {
            self.call_order.push(format!("respond:{}", turn.get()));
            self.responses.push((turn, ticket, response));
            self.response_results
                .pop_front()
                .unwrap_or(Ok(BackendSubmission::Complete))
        }

        fn begin_exit_session(&mut self, _policy: BackendExitPolicy) -> BackendResult<()> {
            Ok(())
        }

        fn apply(&mut self, placements: &[Placement]) -> BackendResult<()> {
            self.apply_attempts.push(placements.to_vec());
            if let Some(error) = self.fail_next_apply.take() {
                return Err(error);
            }
            self.successful_frames.push(placements.to_vec());
            Ok(())
        }

        fn focus(&mut self, _win: WinId) -> BackendResult<()> {
            self.focus_calls += 1;
            Ok(())
        }

        fn close(&mut self, win: WinId) -> BackendResult<()> {
            self.close_attempts.push(win);
            if let Some(error) = self.fail_next_close.take() {
                return Err(error);
            }
            Ok(())
        }

        fn workarea(&self) -> Workarea {
            self.workarea
        }

        fn event_fd(&self) -> BorrowedFd<'_> {
            self.event_file.as_fd()
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

        fn next_event(
            &mut self,
            _deadline: Option<Instant>,
        ) -> BackendResult<Option<BackendEvent>> {
            self.next_event_calls += 1;
            Ok(None)
        }
    }

    fn window_opened(id: &str, title: &str) -> BackendEvent {
        BackendEvent::WindowOpened {
            backend_id: BackendWindowId::new(id).unwrap(),
            app_id: "foot".to_owned(),
            title: title.to_owned(),
        }
    }

    fn recovery_snapshot() -> SessionSnapshotV1 {
        let first = OrbitId::from_human(1).unwrap();
        let second = OrbitId::from_human(2).unwrap();
        let mut ledger = Ledger::new();
        ledger.summon(WinId(7), first);
        ledger.summon(WinId(9), first);
        assert!(ledger.move_to_orbit(second));
        SessionSnapshotV1::new(
            ledger,
            vec![
                SnapshotBinding {
                    win_id: WinId(7),
                    backend_id: BackendWindowId::new("restored-7").unwrap(),
                },
                SnapshotBinding {
                    win_id: WinId(9),
                    backend_id: BackendWindowId::new("missing-9").unwrap(),
                },
            ],
            10,
        )
        .unwrap()
    }

    fn live_session(backend: FakeBackend) -> Session<FakeBackend> {
        let mut session = Session::connect(backend).unwrap();
        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        session.state = RealmState::default();
        session.backend.apply_attempts.clear();
        session.backend.successful_frames.clear();
        session
    }

    #[test]
    fn snapshot_validation_is_closed_and_total() {
        let snapshot = recovery_snapshot();
        let encoded = snapshot.to_json().unwrap();
        assert_eq!(SessionSnapshotV1::from_json(&encoded).unwrap(), snapshot);
        let encoded = String::from_utf8(encoded).unwrap();
        let unknown = encoded.replacen(
            "\"active_orbit\":0}",
            "\"active_orbit\":0,\"unexpected\":true}",
            1,
        );
        assert_ne!(unknown, encoded);
        assert!(SessionSnapshotV1::from_json(unknown.as_bytes()).is_err());
        assert_snapshot_invalid(
            &encoded,
            "\"schema_version\":1",
            "\"schema_version\":2",
            "unsupported schema version",
        );
        let protocol = format!("\"protocol_version\":{PROTOCOL_VERSION}");
        let wrong_protocol = format!("\"protocol_version\":{}", PROTOCOL_VERSION + 1);
        assert_snapshot_invalid(
            &encoded,
            &protocol,
            &wrong_protocol,
            "protocol version mismatch",
        );
        assert_snapshot_invalid(
            &encoded,
            "\"active_orbit\":0}",
            "\"active_orbit\":1}",
            "active orbit disagrees with ledger",
        );
        assert_snapshot_invalid(
            &encoded,
            ",{\"win_id\":9,\"backend_id\":\"missing-9\"}",
            "",
            "bindings must cover the ledger exactly",
        );
        let canonical_prefix =
            format!("\"schema_version\":1,\"protocol_version\":{PROTOCOL_VERSION}");
        let reversed_prefix =
            format!("\"protocol_version\":{PROTOCOL_VERSION},\"schema_version\":1");
        let out_of_order = encoded.replacen(&canonical_prefix, &reversed_prefix, 1);
        assert_ne!(out_of_order, encoded);
        assert!(SessionSnapshotV1::from_json(out_of_order.as_bytes()).is_err());
        let out_of_order_binding = encoded.replacen(
            "\"win_id\":7,\"backend_id\":\"restored-7\"",
            "\"backend_id\":\"restored-7\",\"win_id\":7",
            1,
        );
        assert!(SessionSnapshotV1::from_json(out_of_order_binding.as_bytes()).is_err());
    }

    fn assert_snapshot_invalid(encoded: &str, from: &str, to: &str, expected: &'static str) {
        let changed = encoded.replacen(from, to, 1);
        assert_ne!(changed, encoded);
        let result = SessionSnapshotV1::from_json(changed.as_bytes());
        assert!(
            matches!(
                &result,
                Err(super::SessionSnapshotError::Invalid(message)) if *message == expected
            ),
            "mutation {from:?} produced {result:?}, expected {expected:?}"
        );
    }

    #[test]
    fn nested_snapshot_records_are_closed_and_canonically_ordered() {
        let encoded = String::from_utf8(recovery_snapshot().to_json().unwrap()).unwrap();
        let malformed = [
            encoded.replacen(
                "\"ledger\":{\"orbits\":",
                "\"ledger\":{\"unexpected\":true,\"orbits\":",
                1,
            ),
            encoded.replacen(",\"active\":0},\"bindings\":", "},\"bindings\":", 1),
            encoded.replacen(
                "\"ledger\":{\"orbits\":",
                "\"ledger\":{\"orbits\":[],\"orbits\":",
                1,
            ),
            encoded.replacen(
                "\"ledger\":{\"orbits\":",
                "\"ledger\":{\"active\":0,\"orbits\":",
                1,
            ),
            encoded.replacen(
                "{\"id\":0,\"windows\":",
                "{\"unexpected\":true,\"id\":0,\"windows\":",
                1,
            ),
            encoded.replacen(",\"name\":\"triptych\"", "", 1),
            encoded.replacen(
                "{\"id\":0,\"windows\":",
                "{\"id\":0,\"id\":0,\"windows\":",
                1,
            ),
            encoded.replacen(
                "{\"id\":0,\"windows\":",
                "{\"windows\":[],\"id\":0,\"windows\":",
                1,
            ),
        ];
        for record in malformed {
            assert_ne!(record, encoded);
            assert!(SessionSnapshotV1::from_json(record.as_bytes()).is_err());
        }
    }

    #[test]
    fn snapshot_semantic_validation_exercises_every_invariant_family() {
        let encoded = String::from_utf8(recovery_snapshot().to_json().unwrap()).unwrap();
        for (from, to, expected) in [
            (
                "{\"id\":0,\"windows\":",
                "{\"id\":1,\"windows\":",
                "orbit ids are not canonical",
            ),
            (
                "\"name\":\"triptych\"",
                "\"name\":\"not-triptych\"",
                "orbit names are not canonical",
            ),
            (
                "\"windows\":[7],\"focus\":0",
                "\"windows\":[7],\"focus\":1",
                "orbit focus is out of range",
            ),
            (
                "\"windows\":[7],\"focus\":0",
                "\"windows\":[7],\"focus\":null",
                "occupied orbit has no focus",
            ),
            (
                "\"windows\":[],\"focus\":null",
                "\"windows\":[],\"focus\":0",
                "empty orbit has a focus",
            ),
            (
                "\"windows\":[9],\"focus\":0",
                "\"windows\":[7],\"focus\":0",
                "window ids must be unique and below the watermark",
            ),
            (
                "\"windows\":[7],\"focus\":0,\"stowed\":[]",
                "\"windows\":[7],\"focus\":0,\"stowed\":[9]",
                "stowed windows must be a unique subset of the orbit",
            ),
            (
                "\"windows\":[7],\"focus\":0,\"stowed\":[]",
                "\"windows\":[7],\"focus\":0,\"stowed\":[7,7]",
                "stowed windows must be a unique subset of the orbit",
            ),
            (
                "\"fullscreen\":null",
                "\"fullscreen\":9",
                "fullscreen window must belong to the orbit",
            ),
            (
                "\"backend_id\":\"missing-9\"",
                "\"backend_id\":\"restored-7\"",
                "bindings must be a one-to-one mapping",
            ),
            (
                "\"win_id\":9",
                "\"win_id\":7",
                "bindings must be a one-to-one mapping",
            ),
            (
                "[{\"win_id\":7,\"backend_id\":\"restored-7\"},{\"win_id\":9,\"backend_id\":\"missing-9\"}]",
                "[{\"win_id\":9,\"backend_id\":\"missing-9\"},{\"win_id\":7,\"backend_id\":\"restored-7\"}]",
                "bindings must be strictly sorted by window id",
            ),
            (
                "\"next_win_id\":10",
                "\"next_win_id\":7",
                "window ids must be unique and below the watermark",
            ),
            (
                ",{\"id\":5,\"windows\":[],\"focus\":null,\"stowed\":[],\"layout\":\"triptych\",\"fullscreen\":null,\"name\":\"crypt\"}",
                "",
                "ledger must contain six orbits",
            ),
        ] {
            assert_snapshot_invalid(&encoded, from, to, expected);
        }

        for invalid in [
            "\"backend_id\":\"\"",
            "\"backend_id\":\"123456789012345678901234567890123\"",
            "\"backend_id\":\"bad\\nidentity\"",
        ] {
            let changed = encoded.replacen("\"backend_id\":\"restored-7\"", invalid, 1);
            assert!(matches!(
                SessionSnapshotV1::from_json(changed.as_bytes()),
                Err(super::SessionSnapshotError::Encoding(error))
                    if error.to_string().contains(
                        "backend window identity must be 1..=32 printable ASCII bytes"
                    )
            ));
        }

        let active_out_of_range = encoded
            .replacen("\"active\":0", "\"active\":6", 1)
            .replacen("\"active_orbit\":0}", "\"active_orbit\":6}", 1);
        assert!(matches!(
            SessionSnapshotV1::from_json(active_out_of_range.as_bytes()),
            Err(super::SessionSnapshotError::Invalid(
                "active orbit is out of range"
            ))
        ));
    }

    #[test]
    fn legacy_replay_accumulator_rebinds_each_identity_once() {
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(recovery_snapshot())).unwrap();

        assert_eq!(session.phase(), RecoveryPhase::InitialReplay);
        assert!(session.snapshot().is_none());
        for event in [
            window_opened("restored-7", "old title"),
            window_opened("new-10", "new"),
            window_opened("restored-7", "latest title"),
        ] {
            let update = session.handle_backend_event(event).unwrap();
            assert!(!update.projection_applied);
            assert!(update.state.is_none());
        }

        assert!(session.backend.assignment_attempts.is_empty());
        assert!(session.backend.apply_attempts.is_empty());
        assert!(session.snapshot().is_none());

        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        assert_eq!(
            session.ledger().active_orbit().windows,
            [WinId(7), WinId(10)]
        );
        assert_eq!(
            session.window_metadata(WinId(7)).unwrap().title,
            "latest title"
        );
        assert_eq!(
            session.window_id(&BackendWindowId::new("new-10").unwrap()),
            Some(WinId(10))
        );
        assert_eq!(session.backend.assignment_attempts.len(), 2);
        assert_eq!(
            session
                .backend
                .assignment_attempts
                .iter()
                .filter(|(backend_id, _)| backend_id.as_str() == "restored-7")
                .count(),
            1
        );
    }

    #[test]
    fn legacy_replay_accumulator_rejects_non_replay_events() {
        for event in [
            BackendEvent::TitleChanged {
                win: WinId(7),
                title: "impossible".to_owned(),
            },
            BackendEvent::WindowClosed(WinId(7)),
            BackendEvent::FocusChanged(Some(WinId(7))),
            BackendEvent::ExclusiveFocusChanged(true),
            BackendEvent::GeometryDrifted {
                win: WinId(7),
                rect: Rect::new(0, 0, 10, 10),
            },
        ] {
            let mut session = Session::connect(FakeBackend::new()).unwrap();
            let error = session.handle_backend_event(event).unwrap_err();
            assert!(matches!(
                error,
                SessionEventError::UnexpectedInitialReplayEvent(_)
            ));
            assert_eq!(session.phase(), RecoveryPhase::InitialReplay);
            assert!(session.snapshot().is_none());
        }
    }

    #[test]
    fn restored_only_replay_preserves_persisted_focus() {
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(recovery_snapshot())).unwrap();
        session
            .handle_backend_event(window_opened("missing-9", "second"))
            .unwrap();
        session
            .handle_backend_event(window_opened("restored-7", "first"))
            .unwrap();

        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();

        assert_eq!(session.ledger().focused(), Some(WinId(7)));
        assert_eq!(
            session
                .ledger()
                .orbit(OrbitId::from_human(2).unwrap())
                .focused(),
            Some(WinId(9))
        );
    }

    #[test]
    fn replay_barrier_reconciles_then_publishes_once() {
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(recovery_snapshot())).unwrap();
        session
            .handle_backend_event(window_opened("new-10", "new"))
            .unwrap();
        session
            .handle_backend_event(window_opened("restored-7", "restored"))
            .unwrap();

        let update = session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();

        assert_eq!(session.phase(), RecoveryPhase::Live);
        assert_eq!(
            session.ledger().active_orbit().windows,
            [WinId(7), WinId(10)]
        );
        assert_eq!(session.ledger().focused(), Some(WinId(10)));
        assert!(session
            .ledger()
            .orbit(OrbitId::from_human(2).unwrap())
            .windows
            .is_empty());
        assert_eq!(session.ledger().undo_depth(), 0);
        assert_eq!(session.backend.assignment_attempts.len(), 2);
        assert_eq!(session.backend.apply_attempts.len(), 1);
        assert_eq!(update.state.as_ref().unwrap().revision, 1);
        assert_eq!(session.state().revision, 1);

        let persisted = session.snapshot().unwrap();
        assert_eq!(persisted.next_win_id(), 11);
        assert_eq!(persisted.bindings()[0].win_id, WinId(7));
        assert_eq!(persisted.bindings()[1].win_id, WinId(10));

        let mut empty = Session::connect(FakeBackend::new()).unwrap();
        let empty_update = empty
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        assert_eq!(empty.backend.apply_attempts, [Vec::<Placement>::new()]);
        assert_eq!(empty_update.state.unwrap().revision, 1);
    }

    #[test]
    fn backend_work_gets_one_retry_and_gates_event_reads() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(window_opened("new-0", "new"))
            .unwrap();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "first apply failure".to_owned(),
        });
        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap_err();
        assert!(session.has_pending_backend_work());
        assert_eq!(session.phase(), RecoveryPhase::FinalizingReplay);
        assert_eq!(session.backend.next_event_calls, 0);
        let ledger = session.ledger().clone();
        let error = session
            .handle_backend_event(window_opened("must-wait", "blocked"))
            .unwrap_err();
        assert_eq!(error, SessionEventError::BackendWorkPending);
        assert_eq!(session.phase(), RecoveryPhase::FinalizingReplay);
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(
            session.window_id(&BackendWindowId::new("must-wait").unwrap()),
            None
        );
        assert_eq!(session.backend.next_event_calls, 0);

        let recovered = session.retry_pending_backend_work().unwrap();
        assert!(!session.has_pending_backend_work());
        assert_eq!(session.phase(), RecoveryPhase::Live);
        assert_eq!(recovered.state.unwrap().revision, 1);

        let mut exhausted_retry = Session::connect(FakeBackend::new()).unwrap();
        exhausted_retry.backend.fail_next_apply = Some(BackendError::Io {
            message: "first apply failure".to_owned(),
        });
        exhausted_retry
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap_err();
        exhausted_retry.backend.fail_next_apply = Some(BackendError::Io {
            message: "second apply failure".to_owned(),
        });
        let error = exhausted_retry.retry_pending_backend_work().unwrap_err();
        assert!(matches!(error, SessionEventError::BackendRetryExhausted(_)));
        assert!(!exhausted_retry.has_pending_backend_work());
        assert_eq!(exhausted_retry.backend.apply_attempts.len(), 2);
    }

    #[test]
    fn pending_backend_work_gates_actions_and_publication() {
        let mut pre_live = Session::connect(FakeBackend::new()).unwrap();
        assert_eq!(
            pre_live.set_layout(Layout::Mono),
            Err(SessionActionError::NotReady)
        );
        assert_eq!(
            pre_live.request_close_focused(),
            Err(SessionActionError::NotReady)
        );

        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("one", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("two", "two"))
            .unwrap();
        let snapshot = session.snapshot().unwrap();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "candidate failed".to_owned(),
        });
        session.set_layout(Layout::Mono).unwrap_err();
        assert!(session.has_pending_backend_work());

        let state = session.state().clone();
        let apply_attempts = session.backend.apply_attempts.len();
        assert!(matches!(
            session.set_layout(Layout::Mono),
            Err(SessionActionError::NotReady)
        ));
        assert!(matches!(
            session.request_close_focused(),
            Err(SessionActionError::NotReady)
        ));
        assert_eq!(session.toggle_whichkey(), SessionUpdate::unchanged());
        assert_eq!(
            session.update_modules(vec![Module {
                id: "clock".to_owned(),
                text: "12:34".to_owned(),
                accent: None,
                urgent: false,
            }]),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.apply_attempts.len(), apply_attempts);
        assert!(session.backend.close_attempts.is_empty());
        assert_eq!(session.state(), &state);
        assert_eq!(session.snapshot().unwrap(), snapshot);

        session.retry_pending_backend_work().unwrap();
        assert!(session.toggle_whichkey().state.is_some());
    }

    #[test]
    fn persistence_exposes_only_authoritative_live_state() {
        let mut finalizing = Session::connect(FakeBackend::new()).unwrap();
        finalizing.backend.fail_next_apply = Some(BackendError::Io {
            message: "recovery projection pending".to_owned(),
        });
        finalizing
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap_err();
        assert_eq!(finalizing.phase(), RecoveryPhase::FinalizingReplay);
        assert!(finalizing.snapshot().is_none());

        let mut session = Session::connect(FakeBackend::new()).unwrap();
        assert!(session.snapshot().is_none());
        session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap();
        session
            .handle_backend_event(window_opened("one", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("two", "two"))
            .unwrap();
        let authoritative = session.snapshot().unwrap();

        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "candidate rejected".to_owned(),
        });
        session.set_layout(Layout::Mono).unwrap_err();
        assert_eq!(session.snapshot().unwrap(), authoritative);

        session.retry_pending_backend_work().unwrap();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "observed repair pending".to_owned(),
        });
        session
            .handle_backend_event(BackendEvent::WindowClosed(WinId(1)))
            .unwrap_err();
        let observed = session.snapshot().unwrap();
        assert_eq!(observed.bindings().len(), 1);
        assert_eq!(observed.bindings()[0].win_id, WinId(0));
    }

    #[test]
    fn legacy_replay_accumulator_rejects_exhausted_window_ids() {
        let mut ledger = Ledger::new();
        ledger.summon(WinId(u64::MAX - 1), OrbitId::default());
        let snapshot = SessionSnapshotV1::new(
            ledger.clone(),
            vec![SnapshotBinding {
                win_id: WinId(u64::MAX - 1),
                backend_id: BackendWindowId::new("old").unwrap(),
            }],
            u64::MAX,
        )
        .unwrap();
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(snapshot)).unwrap();
        session
            .handle_backend_event(window_opened("new", "new"))
            .unwrap();

        let error = session
            .handle_backend_event(BackendEvent::InitialReplayComplete)
            .unwrap_err();

        assert_eq!(error, SessionEventError::WindowIdExhausted);
        assert_eq!(session.phase(), RecoveryPhase::InitialReplay);
        assert_eq!(session.ledger(), &ledger);
        assert!(session.backend.assignment_attempts.is_empty());
        assert!(session.backend.apply_attempts.is_empty());
        assert!(session.snapshot().is_none());
    }

    #[test]
    fn session_connects_once_seeds_six_orbits_and_retains_capabilities() {
        let session = Session::connect(FakeBackend::new()).unwrap();

        assert_eq!(session.backend.connect_calls, 1);
        assert_eq!(session.ledger().orbits().len(), 6);
        assert_eq!(session.ledger().active(), OrbitId::from_human(1).unwrap());
        assert_eq!(session.state().revision, 0);
        assert!(session.capabilities().unsupported.is_empty());
        assert!(session.backend.apply_attempts.is_empty());
    }

    #[test]
    fn window_opened_summons_after_focus_and_applies_active_projection_once() {
        let area = Workarea::new(1920, 1080, 32, 26);
        let mut session = live_session(FakeBackend::new());

        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("r2", "two"))
            .unwrap();

        assert_eq!(
            session.ledger().active_orbit().windows,
            [WinId(0), WinId(1)]
        );
        assert_eq!(
            session.backend.bound_windows,
            BTreeMap::from([
                (BackendWindowId::new("r1").unwrap(), WinId(0)),
                (BackendWindowId::new("r2").unwrap(), WinId(1)),
            ])
        );
        assert_eq!(session.backend.successful_frames.len(), 2);
        assert_eq!(
            session.backend.successful_frames.last().unwrap(),
            &project(
                session.ledger().active_orbit(),
                area,
                TriptychParams::default()
            )
        );
    }

    #[test]
    fn unchanged_projection_is_not_applied_twice() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();

        let first = session.retry_pending_backend_work().unwrap();
        let second = session.retry_pending_backend_work().unwrap();

        assert!(!first.projection_applied);
        assert!(!second.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), 1);
    }

    #[test]
    fn no_op_ledger_mutation_does_not_apply_or_emit() {
        let mut session = live_session(FakeBackend::new());

        let update = session
            .switch_orbit(OrbitId::from_human(1).unwrap())
            .unwrap();

        assert!(!update.projection_applied);
        assert!(update.state.is_none());
        assert!(session.backend.apply_attempts.is_empty());
        assert_eq!(session.state().revision, 0);
    }

    #[test]
    fn workarea_change_reprojects_once_but_does_not_emit_unchanged_state() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let revision = session.state().revision;

        let update = session
            .handle_backend_event(BackendEvent::WorkareaChanged(Workarea {
                output: Rect::new(0, 0, 2560, 1440),
                tiles: Rect::new(0, 32, 2560, 1382),
            }))
            .unwrap();

        assert!(update.projection_applied);
        assert!(update.state.is_none());
        assert_eq!(session.backend.apply_attempts.len(), 2);
        assert_eq!(session.state().revision, revision);
    }

    #[test]
    fn geometry_drift_preserves_ledger_and_waits_for_next_projection() {
        let mut session = live_session(FakeBackend::new());
        for (id, title) in [("r1", "one"), ("r2", "two")] {
            session
                .handle_backend_event(window_opened(id, title))
                .unwrap();
        }
        let ledger = session.ledger().clone();
        let attempts = session.backend.apply_attempts.len();

        let drift = session
            .handle_backend_event(BackendEvent::GeometryDrifted {
                win: WinId(0),
                rect: Rect::new(50, 50, 20, 20),
            })
            .unwrap();

        assert_eq!(session.ledger(), &ledger);
        assert!(!drift.projection_applied);
        assert!(drift.state.is_none());
        assert_eq!(session.backend.apply_attempts.len(), attempts);

        session.set_layout(Layout::Mono).unwrap();
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
    }

    #[test]
    fn focused_title_change_emits_once_without_applying() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let attempts = session.backend.apply_attempts.len();
        let revision = session.state().revision;

        let changed = session
            .handle_backend_event(BackendEvent::TitleChanged {
                win: WinId(0),
                title: "renamed".to_owned(),
            })
            .unwrap();
        let unchanged = session
            .handle_backend_event(BackendEvent::TitleChanged {
                win: WinId(0),
                title: "renamed".to_owned(),
            })
            .unwrap();

        assert_eq!(changed.state.unwrap().focused_title, "renamed");
        assert!(unchanged.state.is_none());
        assert_eq!(session.state().revision, revision + 1);
        assert_eq!(session.backend.apply_attempts.len(), attempts);
    }

    #[test]
    fn module_change_emits_once_without_backend_apply() {
        let mut session = live_session(FakeBackend::new());
        let modules = vec![Module {
            id: "clock".to_owned(),
            text: "12:34".to_owned(),
            accent: None,
            urgent: false,
        }];

        let changed = session.update_modules(modules.clone());
        let unchanged = session.update_modules(modules);

        assert!(changed.state.is_some());
        assert!(unchanged.state.is_none());
        assert_eq!(session.state().revision, 1);
        assert!(session.backend.apply_attempts.is_empty());
        assert_eq!(session.backend.focus_calls, 0);
        assert!(session.backend.close_attempts.is_empty());
        assert_eq!(session.backend.next_event_calls, 0);
    }

    #[test]
    fn unsupported_apply_rolls_back_and_emits_no_state() {
        let mut backend = FakeBackend::new();
        backend.capabilities.exact_geometry = false;
        backend.capabilities.unsupported = vec!["exact-geometry".to_owned()];
        let mut session = live_session(backend);
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("r2", "two"))
            .unwrap();
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        let projection = session.last_projection().to_vec();
        let attempts = session.backend.apply_attempts.len();
        let successes = session.backend.successful_frames.len();
        session.backend.fail_next_apply = Some(BackendError::Unsupported {
            capability: "exact-geometry".to_owned(),
        });

        let error = session.set_layout(Layout::Mono).unwrap_err();

        assert_eq!(
            error,
            SessionActionError::Backend(BackendError::Unsupported {
                capability: "exact-geometry".to_owned()
            })
        );
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
        assert_eq!(session.backend.successful_frames.len(), successes);
        assert!(session.has_pending_backend_work());
    }

    #[test]
    fn failed_desired_apply_marks_current_projection_dirty_for_repair() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("r2", "two"))
            .unwrap();
        let authoritative = session.last_projection().to_vec();
        let attempts = session.backend.apply_attempts.len();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "partial apply".to_owned(),
        });

        session.set_layout(Layout::Mono).unwrap_err();
        let repaired = session.retry_pending_backend_work().unwrap();

        assert!(repaired.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), attempts + 2);
        assert_eq!(
            session.backend.apply_attempts.last().unwrap(),
            &authoritative
        );
        assert_eq!(
            session.backend.successful_frames.last().unwrap(),
            &authoritative
        );
    }

    #[test]
    fn failed_apply_after_window_close_retains_the_observed_close_for_retry() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let backend_id = BackendWindowId::new("r1").unwrap();
        assert_eq!(session.window_id(&backend_id), Some(WinId(0)));
        let previous_projection = session.last_projection().to_vec();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "render failed".to_owned(),
        });

        let error = session
            .handle_backend_event(BackendEvent::WindowClosed(WinId(0)))
            .unwrap_err();

        assert!(matches!(
            error,
            super::SessionEventError::Backend(BackendError::Io { .. })
        ));
        assert!(session.ledger().is_empty());
        assert!(session.window_metadata(WinId(0)).is_none());
        assert_eq!(session.window_id(&backend_id), None);
        assert_eq!(session.last_projection(), previous_projection);
        assert!(
            session
                .retry_pending_backend_work()
                .unwrap()
                .projection_applied
        );
        assert!(session.last_projection().is_empty());
    }

    #[test]
    fn failed_apply_after_workarea_change_retains_the_observed_area_for_retry() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let changed = Workarea {
            output: Rect::new(0, 0, 2560, 1440),
            tiles: Rect::new(0, 32, 2560, 1382),
        };
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "render failed".to_owned(),
        });

        session
            .handle_backend_event(BackendEvent::WorkareaChanged(changed))
            .unwrap_err();

        assert_eq!(session.workarea, changed);
        assert!(
            session
                .retry_pending_backend_work()
                .unwrap()
                .projection_applied
        );
        assert_eq!(session.last_projection()[0].rect, changed.tiles);
    }

    #[test]
    fn failed_apply_marks_projection_dirty_until_a_complete_repair() {
        let original = Workarea::new(1920, 1080, 32, 26);
        let changed = Workarea {
            output: Rect::new(0, 0, 2560, 1440),
            tiles: Rect::new(0, 32, 2560, 1382),
        };
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let p1 = session.last_projection().to_vec();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "partial apply".to_owned(),
        });

        session
            .handle_backend_event(BackendEvent::WorkareaChanged(changed))
            .unwrap_err();
        let repaired = session.retry_pending_backend_work().unwrap();
        let restored = session
            .handle_backend_event(BackendEvent::WorkareaChanged(original))
            .unwrap();

        assert!(repaired.projection_applied);
        assert!(restored.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), 4);
        assert_eq!(session.backend.apply_attempts.last().unwrap(), &p1);
        assert_eq!(session.backend.successful_frames.last().unwrap(), &p1);
    }

    #[test]
    fn assignment_contract_failure_uses_the_typed_fatal_event_path() {
        let backend_id = BackendWindowId::new("r1").unwrap();
        let backend = FakeBackend::new();
        let mut session = live_session(backend);
        session.backend.fail_next_assign = Some(BackendContractError::UnknownWindow {
            backend_id: backend_id.clone(),
        });

        let error = session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::UnknownWindow {
                backend_id: ref unknown,
            }) if unknown == &backend_id
        ));
        assert!(!session.has_pending_backend_work());
        assert_eq!(
            session.backend.assignment_attempts,
            [(backend_id.clone(), WinId(0))]
        );
        assert_eq!(session.window_id(&backend_id), Some(WinId(0)));
        assert_eq!(session.next_win_id, 1);
    }

    #[test]
    fn replayed_backend_identity_reuses_its_window_id_without_rebinding() {
        let backend_id = BackendWindowId::new("r1").unwrap();
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();

        let replay = session
            .handle_backend_event(window_opened("r1", "renamed"))
            .unwrap();

        assert_eq!(session.ledger().active_orbit().windows, [WinId(0)]);
        assert_eq!(session.window_id(&backend_id), Some(WinId(0)));
        assert_eq!(session.next_win_id, 1);
        assert_eq!(
            session.backend.assignment_attempts,
            [(backend_id, WinId(0))]
        );
        assert!(!replay.projection_applied);
        assert_eq!(replay.state.unwrap().focused_title, "renamed");
    }

    #[test]
    fn focus_event_is_deferred_without_becoming_a_backend_failure() {
        let mut session = live_session(FakeBackend::new());
        for event in [
            BackendEvent::FocusChanged(None),
            BackendEvent::ExclusiveFocusChanged(true),
        ] {
            let update = session.handle_backend_event(event.clone()).unwrap();

            assert_eq!(update.deferred, Some(event));
            assert!(!update.projection_applied);
            assert!(update.state.is_none());
        }
    }

    #[test]
    fn focus_step_commits_one_projection_and_one_visible_state() {
        let mut session = live_session(FakeBackend::new());
        for (id, title) in [("r1", "one"), ("r2", "two")] {
            session
                .handle_backend_event(window_opened(id, title))
                .unwrap();
        }
        let revision = session.state().revision;
        let attempts = session.backend.apply_attempts.len();
        let mut candidate = session.ledger().clone();
        candidate.focus_step(Dir::Prev);
        let expected = project(
            candidate.active_orbit(),
            Workarea::new(1920, 1080, 32, 26),
            TriptychParams::default(),
        );

        let update = session.focus_step(Dir::Prev).unwrap();

        assert_eq!(session.ledger().focused(), Some(WinId(0)));
        assert_eq!(session.state().focused_title, "one");
        assert_eq!(session.state().revision, revision + 1);
        assert!(update.projection_applied);
        assert_eq!(update.state.unwrap().focused_title, "one");
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
        assert_eq!(session.backend.apply_attempts.last().unwrap(), &expected);
    }

    #[test]
    fn failed_focus_step_rejects_the_candidate_and_repairs_authoritative_state() {
        let mut session = live_session(FakeBackend::new());
        for (id, title) in [("r1", "one"), ("r2", "two")] {
            session
                .handle_backend_event(window_opened(id, title))
                .unwrap();
        }
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        let authoritative = session.last_projection().to_vec();
        let attempts = session.backend.apply_attempts.len();
        let mut candidate = session.ledger().clone();
        candidate.focus_step(Dir::Prev);
        let expected_candidate = project(
            candidate.active_orbit(),
            Workarea::new(1920, 1080, 32, 26),
            TriptychParams::default(),
        );
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "partial focus apply".to_owned(),
        });

        session.focus_step(Dir::Prev).unwrap_err();

        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), authoritative);
        assert_eq!(session.backend.apply_attempts[attempts], expected_candidate);
        let repair = session.retry_pending_backend_work().unwrap();
        assert!(repair.projection_applied);
        assert!(repair.state.is_none());
        assert_eq!(session.backend.apply_attempts.len(), attempts + 2);
        assert_eq!(
            session.backend.apply_attempts.last().unwrap(),
            &authoritative
        );
    }

    #[test]
    fn swap_changes_order_once_and_is_a_no_op_with_one_window() {
        let mut session = live_session(FakeBackend::new());
        for (id, title) in [("r1", "one"), ("r2", "two")] {
            session
                .handle_backend_event(window_opened(id, title))
                .unwrap();
        }
        let revision = session.state().revision;
        let attempts = session.backend.apply_attempts.len();
        let mut candidate = session.ledger().clone();
        candidate.swap(Dir::Prev);
        let expected = project(
            candidate.active_orbit(),
            Workarea::new(1920, 1080, 32, 26),
            TriptychParams::default(),
        );

        let update = session.swap(Dir::Prev).unwrap();

        assert_eq!(
            session.ledger().active_orbit().windows,
            [WinId(1), WinId(0)]
        );
        assert_eq!(session.ledger().focused(), Some(WinId(1)));
        assert!(update.projection_applied);
        assert!(update.state.is_none());
        assert_eq!(session.state().revision, revision);
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
        assert_eq!(session.backend.apply_attempts.last().unwrap(), &expected);

        let mut singleton = live_session(FakeBackend::new());
        singleton
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let attempts = singleton.backend.apply_attempts.len();
        let revision = singleton.state().revision;

        let unchanged = singleton.swap(Dir::Next).unwrap();

        assert!(!unchanged.projection_applied);
        assert!(unchanged.state.is_none());
        assert_eq!(singleton.backend.apply_attempts.len(), attempts);
        assert_eq!(singleton.state().revision, revision);
    }

    #[test]
    fn typed_ledger_actions_delegate_to_the_ledger_contract() {
        let second = OrbitId::from_human(2).unwrap();

        let mut moved = live_session(FakeBackend::new());
        moved
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        moved.move_focused_to_orbit(second).unwrap();
        assert!(moved.ledger().active_orbit().windows.is_empty());
        assert_eq!(moved.ledger().orbit(second).windows, [WinId(0)]);

        let mut stowed = live_session(FakeBackend::new());
        stowed
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        stowed.toggle_stow().unwrap();
        assert_eq!(stowed.ledger().active_orbit().stowed, [WinId(0)]);
        assert!(stowed.last_projection().is_empty());

        let mut fullscreen = live_session(FakeBackend::new());
        fullscreen
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        fullscreen.toggle_fullscreen().unwrap();
        assert_eq!(
            fullscreen.ledger().active_orbit().fullscreen,
            Some(WinId(0))
        );
        assert_eq!(
            fullscreen.last_projection()[0].rect,
            Workarea::new(1920, 1080, 32, 26).output
        );

        let mut switched = live_session(FakeBackend::new());
        switched.switch_orbit(second).unwrap();
        assert_eq!(switched.ledger().active(), second);

        let mut laid_out = live_session(FakeBackend::new());
        laid_out.set_layout(Layout::Mono).unwrap();
        assert_eq!(laid_out.ledger().active_orbit().layout, Layout::Mono);
    }

    #[test]
    fn failed_undo_does_not_consume_history() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        session
            .handle_backend_event(window_opened("r2", "two"))
            .unwrap();
        session.set_layout(Layout::Mono).unwrap();
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        session.backend.fail_next_apply = Some(BackendError::Io {
            message: "partial undo apply".to_owned(),
        });

        session.undo().unwrap_err();

        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        session.retry_pending_backend_work().unwrap();
        let retry = session.undo().unwrap();
        assert!(retry.projection_applied);
        assert_eq!(session.ledger().active_orbit().layout, Layout::Triptych);
    }

    #[test]
    fn undo_never_removes_an_open_window_or_restores_a_closed_window() {
        let mut session = live_session(FakeBackend::new());
        for (id, title) in [("r1", "one"), ("r2", "two")] {
            session
                .handle_backend_event(window_opened(id, title))
                .unwrap();
        }
        session.swap(Dir::Prev).unwrap();
        session
            .handle_backend_event(window_opened("r3", "three"))
            .unwrap();
        let attempts_after_open = session.backend.apply_attempts.len();

        let after_open_undo = session.undo().unwrap();

        assert!(!after_open_undo.projection_applied);
        assert!(after_open_undo.state.is_none());
        assert_eq!(session.ledger().len(), 3);
        assert!(session.window_metadata(WinId(2)).is_some());
        assert_eq!(session.backend.apply_attempts.len(), attempts_after_open);

        session.swap(Dir::Next).unwrap();
        session
            .handle_backend_event(BackendEvent::WindowClosed(WinId(1)))
            .unwrap();
        let attempts_after_close = session.backend.apply_attempts.len();

        let after_close_undo = session.undo().unwrap();

        assert!(!after_close_undo.projection_applied);
        assert!(after_close_undo.state.is_none());
        assert_eq!(session.ledger().orbit_of(WinId(1)), None);
        assert!(session.window_metadata(WinId(1)).is_none());
        assert_eq!(
            session.window_id(&BackendWindowId::new("r2").unwrap()),
            None
        );
        assert_eq!(session.backend.apply_attempts.len(), attempts_after_close);
    }

    #[test]
    fn close_request_waits_for_the_observed_close_before_mutating_state() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        let projection = session.last_projection().to_vec();

        assert_eq!(
            session.request_close_focused().unwrap(),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.close_attempts, [WinId(0)]);
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);

        session.backend.fail_next_close = Some(BackendError::Io {
            message: "close request failed".to_owned(),
        });
        session.request_close_focused().unwrap_err();
        assert_eq!(session.backend.close_attempts, [WinId(0), WinId(0)]);
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);

        session
            .handle_backend_event(BackendEvent::WindowClosed(WinId(0)))
            .unwrap();
        assert!(session.ledger().is_empty());
    }

    #[test]
    fn legacy_empty_close_request_is_a_no_op() {
        let mut session = live_session(FakeBackend::new());

        assert_eq!(
            session.request_close_focused().unwrap(),
            SessionUpdate::unchanged()
        );
        assert!(session.backend.close_attempts.is_empty());
        assert_eq!(session.state().revision, 0);
        assert!(session.backend.apply_attempts.is_empty());
    }

    #[test]
    fn whichkey_toggle_only_emits_state_until_workarea_is_observed() {
        let mut session = live_session(FakeBackend::new());
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        let attempts = session.backend.apply_attempts.len();
        let revision = session.state().revision;

        let update = session.toggle_whichkey();

        assert!(!update.projection_applied);
        assert!(!update.state.as_ref().unwrap().whichkey);
        assert_eq!(session.state().revision, revision + 1);
        assert_eq!(session.backend.apply_attempts.len(), attempts);

        let changed = Workarea {
            output: Rect::new(0, 0, 2560, 1440),
            tiles: Rect::new(0, 64, 2560, 1324),
        };
        let workarea_update = session
            .handle_backend_event(BackendEvent::WorkareaChanged(changed))
            .unwrap();
        assert!(workarea_update.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
    }

    fn policy_turn(id: u64, events: Vec<BackendPolicyEvent>) -> BackendEvent {
        BackendEvent::PolicyTurn(BackendPolicyTurn {
            id: BackendPolicyTurnId::new(id).unwrap(),
            drains: None,
            events,
        })
    }

    fn draining_policy_turn(
        id: u64,
        ticket: BackendTicket,
        events: Vec<BackendPolicyEvent>,
    ) -> BackendEvent {
        BackendEvent::PolicyTurn(BackendPolicyTurn {
            id: BackendPolicyTurnId::new(id).unwrap(),
            drains: Some(ticket),
            events,
        })
    }

    fn policy_window_opened(id: &str, title: &str) -> BackendPolicyEvent {
        BackendPolicyEvent::WindowOpened {
            backend_id: BackendWindowId::new(id).unwrap(),
            app_id: "foot".to_owned(),
            title: title.to_owned(),
        }
    }

    fn selected_workarea() -> Workarea {
        Workarea::new(1920, 1080, 32, 26)
    }

    fn finish_policy_bootstrap(session: &mut Session<FakeBackend>) {
        let replay = session
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        assert!(replay.state.is_none());
        assert_eq!(session.phase(), RecoveryPhase::FinalizingReplay);
        let live = session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();
        assert_eq!(live.state.as_ref().map(|state| state.revision), Some(1));
        assert_eq!(session.phase(), RecoveryPhase::Live);
        session.backend.responses.clear();
        session.backend.request_attempts = 0;
        session.backend.call_order.clear();
    }

    fn policy_live_session(backend: FakeBackend) -> Session<FakeBackend> {
        let mut session = Session::connect(backend).unwrap();
        finish_policy_bootstrap(&mut session);
        session
    }

    fn policy_live_session_with_keymap(
        backend: FakeBackend,
        keymap: Keymap,
    ) -> Session<FakeBackend> {
        let mut session = Session::connect_with_keymap(backend, keymap).unwrap();
        finish_policy_bootstrap(&mut session);
        session
    }

    fn binding_id(session: &Session<FakeBackend>, key: &str) -> BackendBindingId {
        session
            .binding_id_for_key(key)
            .expect("test key is configured")
    }

    fn open_two_policy_windows(session: &mut Session<FakeBackend>) {
        session
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("r1", "one"),
                    policy_window_opened("r2", "two"),
                ],
            ))
            .unwrap();
        session.backend.responses.clear();
        session.backend.call_order.clear();
    }

    fn policy_in_flight_session() -> (Session<FakeBackend>, BackendTicket) {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .backend
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        session
            .handle_backend_event(policy_turn(3, Vec::new()))
            .unwrap();
        let ticket = session.backend.responses[0].1;
        (session, ticket)
    }

    fn policy_awaiting_drain_session() -> (Session<FakeBackend>, BackendTicket) {
        let (mut session, ticket) = policy_in_flight_session();
        session
            .handle_backend_event(BackendEvent::OperationCompleted {
                ticket,
                result: Ok(()),
            })
            .unwrap();
        (session, ticket)
    }

    fn test_binding(key: &str, action: Action, mode: Mode, repeatable: bool) -> Binding {
        Binding {
            key: key.to_owned(),
            hint_key: key.to_owned(),
            label: key.to_owned(),
            action,
            mode,
            repeatable,
            in_strip: false,
        }
    }

    fn spawn_keymap() -> Keymap {
        Keymap {
            modifier: "mod".to_owned(),
            bindings: vec![test_binding(
                "x",
                Action::Spawn(vec!["job-x".to_owned()]),
                Mode::Nav,
                false,
            )],
        }
    }

    fn json_content_len(value: &str) -> usize {
        serde_json::to_string(value).unwrap().len() - 2
    }

    #[test]
    fn binding_configuration_contains_mechanism_not_policy() {
        let session = Session::connect(FakeBackend::new()).unwrap();
        let configured = &session.backend.configured_bindings;
        let keymap = Keymap::default();

        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].len(), keymap.bindings.len());
        for (index, (mechanism, binding)) in configured[0].iter().zip(&keymap.bindings).enumerate()
        {
            assert_eq!(mechanism.id.get(), u32::try_from(index + 1).unwrap());
            assert_eq!(mechanism.keysym, binding.key);
            assert_eq!(mechanism.modifiers, [BackendModifier::Super]);
        }
        assert_eq!(session.state().mode, Mode::Nav);
    }

    #[test]
    fn unknown_binding_id_is_fatal_before_partial_reduction() {
        let mut session = policy_live_session(FakeBackend::new());
        let ledger = session.ledger().clone();
        let assignments = session.backend.assignment_attempts.len();

        let error = session
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("never-bound", "private"),
                    BackendPolicyEvent::BindingPressed(BackendBindingId::new(u32::MAX).unwrap()),
                ],
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::UnknownBinding { .. })
        ));
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.backend.assignment_attempts.len(), assignments);
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn policy_batch_is_atomic_and_linear() {
        let mut valid = policy_live_session(FakeBackend::new());
        let a = BackendWindowId::new("a").unwrap();
        let b = BackendWindowId::new("b").unwrap();
        valid
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("a", "a-0"),
                    policy_window_opened("b", "b-0"),
                    BackendPolicyEvent::TitleChanged {
                        backend_id: a.clone(),
                        title: "a-1".to_owned(),
                    },
                    BackendPolicyEvent::WindowClosed(b),
                    BackendPolicyEvent::FocusChanged(Some(a.clone())),
                ],
            ))
            .unwrap();
        assert_eq!(valid.ledger().active_orbit().windows, [WinId(0)]);
        assert_eq!(valid.window_metadata(WinId(0)).unwrap().title, "a-1");
        assert_eq!(valid.backend.responses.len(), 1);
        let whichkey = binding_id(&valid, "w");
        let toggled = valid
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(whichkey)],
            ))
            .unwrap();
        assert_eq!(
            toggled.state.as_ref().map(|state| state.whichkey),
            Some(false)
        );

        let mut invalid = policy_live_session(FakeBackend::new());
        let before = invalid.ledger().clone();
        let error = invalid
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("private", "private"),
                    BackendPolicyEvent::TitleChanged {
                        backend_id: BackendWindowId::new("unknown").unwrap(),
                        title: "invalid".to_owned(),
                    },
                ],
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::UnknownWindowReference)
        ));
        assert_eq!(invalid.ledger(), &before);
        assert!(invalid.backend.assignment_attempts.is_empty());
        assert!(invalid.backend.responses.is_empty());
    }

    #[test]
    fn policy_turn_must_be_answered_exactly_once() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(3, Vec::new()))
            .unwrap();

        let error = session
            .handle_backend_event(policy_turn(3, Vec::new()))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(session.backend.responses.len(), 1);
    }

    #[test]
    fn in_flight_rejects_every_second_untagged_turn_without_response() {
        for events in [Vec::new(), vec![policy_window_opened("private", "private")]] {
            let (mut session, ticket) = policy_in_flight_session();
            let responses = session.backend.responses.clone();
            let assignments = session.backend.assignment_attempts.clone();
            let watermark = session.last_backend_ticket;

            let error = session
                .handle_backend_event(policy_turn(4, events))
                .unwrap_err();

            assert!(matches!(
                error,
                SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
            ));
            assert_eq!(session.backend.responses, responses);
            assert_eq!(session.backend.assignment_attempts, assignments);
            assert_eq!(session.last_backend_ticket, watermark);
            assert_eq!(
                session.transaction,
                TransactionSubstate::InFlight { ticket }
            );
            assert!(session.active.is_some());
        }
    }

    #[test]
    fn awaiting_drain_rejects_untagged_and_wrong_tagged_turns() {
        let (mut untagged, ticket) = policy_awaiting_drain_session();
        let responses = untagged.backend.responses.clone();
        let watermark = untagged.last_backend_ticket;
        let error = untagged
            .handle_backend_event(policy_turn(4, Vec::new()))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(untagged.backend.responses, responses);
        assert_eq!(untagged.last_backend_ticket, watermark);
        assert_eq!(
            untagged.transaction,
            TransactionSubstate::AwaitingDrain { ticket }
        );

        let (mut wrong_tag, ticket) = policy_awaiting_drain_session();
        let wrong_ticket = BackendTicket::new(ticket.get() + 1).unwrap();
        let responses = wrong_tag.backend.responses.clone();
        let watermark = wrong_tag.last_backend_ticket;
        let error = wrong_tag
            .handle_backend_event(draining_policy_turn(4, wrong_ticket, Vec::new()))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(wrong_tag.backend.responses, responses);
        assert_eq!(wrong_tag.last_backend_ticket, watermark);
        assert_eq!(
            wrong_tag.transaction,
            TransactionSubstate::AwaitingDrain { ticket }
        );
    }

    #[test]
    fn awaiting_drain_accepts_only_one_matching_tagged_turn() {
        let (mut session, ticket) = policy_awaiting_drain_session();

        let completed = session
            .handle_backend_event(draining_policy_turn(4, ticket, Vec::new()))
            .unwrap();

        assert_eq!(completed, SessionUpdate::unchanged());
        assert_eq!(session.backend.responses.len(), 2);
        assert_eq!(session.backend.responses[1].0.get(), 4);
        assert_eq!(session.transaction, TransactionSubstate::Idle);
        assert!(session.active.is_none());
        let responses = session.backend.responses.clone();
        let error = session
            .handle_backend_event(draining_policy_turn(5, ticket, Vec::new()))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(session.backend.responses, responses);
    }

    #[test]
    fn awaiting_drain_accepts_only_one_matching_marker() {
        let (mut wrong_tag, ticket) = policy_awaiting_drain_session();
        let wrong_ticket = BackendTicket::new(ticket.get() + 1).unwrap();
        let responses = wrong_tag.backend.responses.clone();
        let error = wrong_tag
            .handle_backend_event(BackendEvent::RetainedObservationsDrained {
                ticket: wrong_ticket,
            })
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(wrong_tag.backend.responses, responses);
        assert_eq!(
            wrong_tag.transaction,
            TransactionSubstate::AwaitingDrain { ticket }
        );

        let (mut session, ticket) = policy_awaiting_drain_session();
        let responses = session.backend.responses.clone();
        let completed = session
            .handle_backend_event(BackendEvent::RetainedObservationsDrained { ticket })
            .unwrap();
        assert_eq!(completed, SessionUpdate::unchanged());
        assert_eq!(session.backend.responses, responses);
        assert_eq!(session.transaction, TransactionSubstate::Idle);
        assert!(session.active.is_none());

        let error = session
            .handle_backend_event(BackendEvent::RetainedObservationsDrained { ticket })
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(session.backend.responses, responses);
    }

    #[test]
    fn resize_binding_is_answered_in_the_same_policy_turn() {
        let mut session = policy_live_session(FakeBackend::new());
        let resize = binding_id(&session, "r");

        let update = session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();

        assert_eq!(session.backend.responses.len(), 1);
        let response = &session.backend.responses[0].2;
        assert!(response.bindings.enabled.is_empty());
        assert_eq!(response.bindings.next_key_edge, BackendNextKeyEdge::Ensure);
        assert_eq!(update.state.as_ref().unwrap().mode, Mode::Resize);

        let mut keymap = Keymap::default();
        keymap.bindings.push(test_binding(
            "Left",
            Action::Focus(Dir::Prev),
            Mode::Resize,
            true,
        ));
        let mut submap = policy_live_session_with_keymap(FakeBackend::new(), keymap);
        let resize = binding_id(&submap, "r");
        let left = binding_id(&submap, "Left");
        submap
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();
        submap.backend.responses.clear();
        submap
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(left)],
            ))
            .unwrap();
        assert_eq!(
            submap.backend.responses[0].2.bindings.next_key_edge,
            BackendNextKeyEdge::Ensure
        );
    }

    #[test]
    fn eaten_unbound_key_returns_to_nav_in_its_policy_response() {
        let mut session = policy_live_session(FakeBackend::new());
        let resize = binding_id(&session, "r");
        session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();
        session.backend.responses.clear();

        let update = session
            .handle_backend_event(policy_turn(4, vec![BackendPolicyEvent::UnboundKeyEaten]))
            .unwrap();

        let response = &session.backend.responses[0].2;
        let expected = Keymap::default().bindings.len();
        assert_eq!(response.bindings.enabled.len(), expected);
        assert_eq!(
            response.bindings.next_key_edge,
            BackendNextKeyEdge::Preserve
        );
        assert_eq!(update.state.as_ref().unwrap().mode, Mode::Nav);
        assert!(update.state.as_ref().unwrap().chord_echo.is_empty());
    }

    #[test]
    fn same_batch_submap_press_then_unbound_exit_returns_nav_without_ensure() {
        let mut keymap = Keymap::default();
        keymap.bindings.push(test_binding(
            "Left",
            Action::Focus(Dir::Prev),
            Mode::Resize,
            true,
        ));
        let mut unbound_exit = policy_live_session_with_keymap(FakeBackend::new(), keymap);
        let resize = binding_id(&unbound_exit, "r");
        let left = binding_id(&unbound_exit, "Left");
        unbound_exit
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();
        unbound_exit.backend.responses.clear();

        let update = unbound_exit
            .handle_backend_event(policy_turn(
                4,
                vec![
                    BackendPolicyEvent::BindingPressed(left),
                    BackendPolicyEvent::UnboundKeyEaten,
                ],
            ))
            .unwrap();

        let response = &unbound_exit.backend.responses[0].2;
        assert_eq!(
            response.bindings.next_key_edge,
            BackendNextKeyEdge::Preserve
        );
        assert!(response.bindings.enabled.contains(&resize));
        assert!(!response.bindings.enabled.contains(&left));
        assert_eq!(update.state.as_ref().unwrap().mode, Mode::Nav);
        assert!(update.state.as_ref().unwrap().chord_echo.is_empty());

        let mut bound_exit = policy_live_session(FakeBackend::new());
        let resize = binding_id(&bound_exit, "r");
        let escape = binding_id(&bound_exit, "Escape");
        let update = bound_exit
            .handle_backend_event(policy_turn(
                3,
                vec![
                    BackendPolicyEvent::BindingPressed(resize),
                    BackendPolicyEvent::BindingPressed(escape),
                ],
            ))
            .unwrap();
        let response = &bound_exit.backend.responses[0].2;
        assert_eq!(
            response.bindings.next_key_edge,
            BackendNextKeyEdge::Preserve
        );
        assert!(response.bindings.enabled.contains(&resize));
        assert!(update.state.is_none());
        assert_eq!(bound_exit.mode, Mode::Nav);
        assert!(bound_exit.chord_echo.is_empty());
    }

    #[test]
    fn non_action_policy_turns_are_answered_exactly_once() {
        let mut session = policy_live_session(FakeBackend::new());
        let focus = binding_id(&session, "j");
        session
            .handle_backend_event(policy_turn(3, Vec::new()))
            .unwrap();
        let modifier_update = session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::ModifiersChanged {
                    old: Vec::new(),
                    new: vec![BackendModifier::Super],
                }],
            ))
            .unwrap();
        assert_eq!(
            modifier_update
                .state
                .as_ref()
                .map(|state| state.chord_echo.as_str()),
            Some("mod+ ▸ awaiting chord…")
        );
        let modifier_update = session
            .handle_backend_event(policy_turn(
                5,
                vec![
                    BackendPolicyEvent::ModifiersChanged {
                        old: vec![BackendModifier::Super],
                        new: Vec::new(),
                    },
                    BackendPolicyEvent::ModifiersChanged {
                        old: Vec::new(),
                        new: vec![BackendModifier::Super],
                    },
                ],
            ))
            .unwrap();
        assert!(modifier_update.state.is_none());
        assert_eq!(session.state().chord_echo, "mod+ ▸ awaiting chord…");
        let modifier_update = session
            .handle_backend_event(policy_turn(
                6,
                vec![BackendPolicyEvent::ModifiersChanged {
                    old: vec![BackendModifier::Super],
                    new: Vec::new(),
                }],
            ))
            .unwrap();
        assert_eq!(
            modifier_update
                .state
                .as_ref()
                .map(|state| state.chord_echo.as_str()),
            Some("")
        );

        let cases = [
            vec![BackendPolicyEvent::BindingReleased(focus)],
            vec![BackendPolicyEvent::BindingRepeatStopped(focus)],
            vec![BackendPolicyEvent::ModifiersChanged {
                old: Vec::new(),
                new: Vec::new(),
            }],
        ];

        for (offset, events) in cases.into_iter().enumerate() {
            session
                .handle_backend_event(policy_turn(7 + offset as u64, events))
                .unwrap();
        }

        assert_eq!(session.backend.responses.len(), 7);
        assert_eq!(
            session
                .backend
                .responses
                .iter()
                .map(|(turn, _, _)| turn.get())
                .collect::<Vec<_>>(),
            vec![3, 4, 5, 6, 7, 8, 9]
        );
    }

    #[test]
    fn clean_projection_equality_commits_without_consuming_a_backend_ticket() {
        let mut session = policy_live_session(FakeBackend::new());

        let update = session.set_layout(Layout::Mono).unwrap();

        assert_eq!(session.ledger().active_orbit().layout, Layout::Mono);
        assert_eq!(update.state.as_ref().unwrap().layout, Layout::Mono);
        assert!(update.pending_action.is_none());
        assert!(update.action_completion.is_none());
        assert_eq!(session.backend.request_attempts, 0);
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn ticketless_local_update_is_final_without_action_completion() {
        let mut session = policy_live_session(FakeBackend::new());
        let second = OrbitId::from_human(2).unwrap();

        let update = session.switch_orbit(second).unwrap();

        assert_eq!(session.ledger().active(), second);
        assert!(update.pending_action.is_none());
        assert!(update.action_completion.is_none());
        assert!(update.effects.is_empty());
        assert_eq!(session.backend.request_attempts, 0);
    }

    #[test]
    fn direct_spawn_is_ticketless_effect_only_and_rejects_invalid_command() {
        let mut session = policy_live_session(FakeBackend::new());
        let argv = vec!["realm-term".to_owned(), "--new".to_owned()];

        let update = session.request_spawn(argv.clone()).unwrap();

        assert_eq!(update.effects, [SessionEffect::Spawn(argv)]);
        assert!(update.pending_action.is_none());
        assert!(update.action_completion.is_none());
        assert!(update.state.is_none());
        assert_eq!(session.backend.request_attempts, 0);
        assert_eq!(
            session.request_spawn(Vec::new()).unwrap_err(),
            SessionActionError::InvalidSpawnCommand
        );
        assert_eq!(
            session.request_spawn(vec![String::new()]).unwrap_err(),
            SessionActionError::InvalidSpawnCommand
        );
    }

    #[test]
    fn direct_spawn_is_not_ready_during_active_transaction() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let admitted = session.focus_step(Dir::Prev).unwrap();
        assert!(admitted.pending_action.is_some());

        assert_eq!(
            session
                .request_spawn(vec!["blocked".to_owned()])
                .unwrap_err(),
            SessionActionError::NotReady
        );
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn visible_metadata_is_json_bounded_at_ingress() {
        let mut session = policy_live_session(FakeBackend::new());
        let source = format!("{}{}", "\u{0001}\n\\\"".repeat(40), "界".repeat(80));
        session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::WindowOpened {
                    backend_id: BackendWindowId::new("bounded").unwrap(),
                    app_id: source.clone(),
                    title: source,
                }],
            ))
            .unwrap();

        let metadata = session.window_metadata(WinId(0)).unwrap();
        assert!(json_content_len(&metadata.app_id) <= MAX_VISIBLE_APP_ID_JSON_BYTES);
        assert!(json_content_len(&metadata.title) <= MAX_VISIBLE_TITLE_JSON_BYTES);
        assert!(metadata.app_id.ends_with('…'));
        assert!(metadata.title.ends_with('…'));
    }

    #[test]
    fn repeated_title_changes_replace_bounded_metadata() {
        let mut session = policy_live_session(FakeBackend::new());
        let id = BackendWindowId::new("bounded").unwrap();
        session
            .handle_backend_event(policy_turn(3, vec![policy_window_opened("bounded", "old")]))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::TitleChanged {
                    backend_id: id.clone(),
                    title: "x".repeat(20_000),
                }],
            ))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::TitleChanged {
                    backend_id: id,
                    title: format!("new-{}", "y".repeat(20_000)),
                }],
            ))
            .unwrap();

        let title = &session.window_metadata(WinId(0)).unwrap().title;
        assert!(title.starts_with("new-"));
        assert!(!title.contains('x'));
        assert!(json_content_len(title) <= MAX_VISIBLE_TITLE_JSON_BYTES);
    }

    #[test]
    fn maximum_show_ledger_fits_one_control_frame() {
        let mut ledger = Ledger::new();
        let mut bindings = Vec::with_capacity(MAX_MANAGED_WINDOWS);
        let mut events = Vec::with_capacity(MAX_MANAGED_WINDOWS + 1);
        for index in 0..MAX_MANAGED_WINDOWS {
            let win = WinId(u64::MAX - 1 - index as u64);
            let backend_id = BackendWindowId::new(format!("{index:032}")).unwrap();
            ledger.summon(win, OrbitId::new(index % ORBIT_COUNT).unwrap());
            bindings.push(SnapshotBinding {
                win_id: win,
                backend_id: backend_id.clone(),
            });
            events.push(BackendPolicyEvent::WindowOpened {
                backend_id,
                app_id: "\n".repeat(MAX_VISIBLE_APP_ID_JSON_BYTES),
                title: "\n".repeat(MAX_VISIBLE_TITLE_JSON_BYTES),
            });
        }
        bindings.sort_by_key(|binding| binding.win_id);
        let snapshot = SessionSnapshotV1::new(ledger, bindings, u64::MAX).unwrap();
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(snapshot)).unwrap();
        events.push(BackendPolicyEvent::InitialReplayComplete);
        session
            .handle_backend_event(policy_turn(1, events))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();

        let frame = ipc::encode(&Response::Ledger(session.visible_ledger(None))).unwrap();
        assert!(frame.len() < ipc::MAX_FRAME_BYTES);
    }

    #[test]
    fn desired_action_requests_turn_and_returns_pending_origin() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let before = session.ledger().clone();

        let admitted = session.focus_step(Dir::Prev).unwrap();
        let origin = admitted.pending_action.unwrap();

        assert_eq!(session.backend.request_attempts, 1);
        assert_eq!(session.ledger(), &before);
        assert!(admitted.action_completion.is_none());
        let completed = session
            .handle_backend_event(policy_turn(4, Vec::new()))
            .unwrap();
        assert_eq!(session.backend.responses[0].1, origin);
        assert_eq!(completed.action_completion.as_ref().unwrap().ticket, origin);
        assert_eq!(session.ledger().focused(), Some(WinId(0)));
    }

    #[test]
    fn desired_action_empty_turn_carries_staged_projection() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let previous_projection = session.last_projection().to_vec();
        let origin = session
            .focus_step(Dir::Prev)
            .unwrap()
            .pending_action
            .unwrap();

        let completed = session
            .handle_backend_event(policy_turn(4, Vec::new()))
            .unwrap();

        let response_projection = session.backend.responses[0]
            .2
            .projection
            .clone()
            .expect("the awaited response carries the staged projection");
        assert_ne!(response_projection, previous_projection);
        assert_eq!(
            response_projection
                .iter()
                .find(|placement| placement.focused)
                .map(|placement| placement.win),
            Some(WinId(0))
        );
        assert_eq!(session.last_projection(), response_projection);
        assert!(completed.projection_applied);
        assert_eq!(completed.action_completion.unwrap().ticket, origin);
    }

    #[test]
    fn failed_turn_request_is_error_atomic() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        let projection = session.last_projection().to_vec();
        session
            .backend
            .request_results
            .push_back(Err(BackendError::Io {
                message: "request failed".to_owned(),
            }));

        let error = session.focus_step(Dir::Prev).unwrap_err();

        assert!(matches!(
            error,
            SessionActionError::Backend(BackendError::Io { .. })
        ));
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);
        assert!(!session.has_active_backend_transaction());
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn failed_turn_request_consumes_origin_ticket() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        session
            .backend
            .request_results
            .push_back(Err(BackendError::Unsupported {
                capability: "policy-turn".to_owned(),
            }));
        session.focus_step(Dir::Prev).unwrap_err();

        let next = session
            .focus_step(Dir::Prev)
            .unwrap()
            .pending_action
            .unwrap();

        assert_eq!(next.get(), 5);
        assert_eq!(session.backend.request_attempts, 2);
    }

    #[test]
    fn close_target_closed_in_awaited_turn_is_not_replayed() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(
                3,
                vec![policy_window_opened("close-me", "one")],
            ))
            .unwrap();
        session.backend.responses.clear();
        let admitted = session.request_close_focused().unwrap();
        let origin = admitted.pending_action.unwrap();

        let completed = session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::WindowClosed(
                    BackendWindowId::new("close-me").unwrap(),
                )],
            ))
            .unwrap();

        assert!(session.backend.responses[0].2.closes.is_empty());
        assert_eq!(completed.action_completion.as_ref().unwrap().ticket, origin);
        assert!(completed.action_completion.unwrap().result.is_ok());
        assert!(session.ledger().is_empty());
    }

    #[test]
    fn failed_close_turn_request_is_an_immediate_clean_error() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(
                3,
                vec![policy_window_opened("close-me", "one")],
            ))
            .unwrap();
        session.backend.responses.clear();
        let before = session.ledger().clone();
        session
            .backend
            .request_results
            .push_back(Err(BackendError::Io {
                message: "request failed".to_owned(),
            }));

        let error = session.request_close_focused().unwrap_err();

        assert!(matches!(
            error,
            SessionActionError::Backend(BackendError::Io { .. })
        ));
        assert_eq!(session.ledger(), &before);
        assert!(!session.has_active_backend_transaction());
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn close_request_is_a_no_op_without_a_focused_window() {
        let mut session = policy_live_session(FakeBackend::new());

        let update = session.request_close_focused().unwrap();

        assert_eq!(update, SessionUpdate::unchanged());
        assert_eq!(session.backend.request_attempts, 0);
        assert!(session.backend.responses.is_empty());
    }

    #[test]
    fn duplicate_close_edges_are_unique_and_bounded() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(3, vec![policy_window_opened("victim", "one")]))
            .unwrap();
        session.backend.responses.clear();
        let banish = binding_id(&session, "q");

        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(banish); MAX_POLICY_EVENTS],
            ))
            .unwrap();

        assert_eq!(session.backend.responses[0].2.closes, [WinId(0)]);
        assert!(session.backend.responses[0].2.closes.len() <= MAX_MANAGED_WINDOWS);

        let mut ordered = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut ordered);
        ordered.backend.responses.clear();
        let previous = binding_id(&ordered, "k");
        let banish = binding_id(&ordered, "q");
        ordered.request_close_focused().unwrap();
        ordered
            .handle_backend_event(policy_turn(
                4,
                vec![
                    BackendPolicyEvent::BindingPressed(previous),
                    BackendPolicyEvent::BindingPressed(banish),
                ],
            ))
            .unwrap();
        assert_eq!(ordered.backend.responses[0].2.closes, [WinId(1), WinId(0)]);
    }

    #[test]
    fn identity_binding_is_local_idempotent_and_contract_closed() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("same", "first"),
                    policy_window_opened("same", "latest"),
                ],
            ))
            .unwrap();
        session
            .handle_backend_event(policy_turn(4, vec![policy_window_opened("same", "again")]))
            .unwrap();
        assert_eq!(session.backend.assignment_attempts.len(), 1);
        assert_eq!(session.ledger().active_orbit().windows, [WinId(0)]);

        let mut failing = policy_live_session(FakeBackend::new());
        failing.backend.fail_next_assign = Some(BackendContractError::UnknownWindow {
            backend_id: BackendWindowId::new("bad").unwrap(),
        });
        let error = failing
            .handle_backend_event(policy_turn(3, vec![policy_window_opened("bad", "one")]))
            .unwrap_err();
        assert!(matches!(error, SessionEventError::BackendContract(_)));
        assert!(failing.backend.responses.is_empty());
    }

    #[test]
    fn open_then_focus_and_title_in_one_turn_binds_once_in_order() {
        let mut session = policy_live_session(FakeBackend::new());
        let id = BackendWindowId::new("ordered").unwrap();

        session
            .handle_backend_event(policy_turn(
                3,
                vec![
                    policy_window_opened("ordered", "old"),
                    BackendPolicyEvent::FocusChanged(Some(id.clone())),
                    BackendPolicyEvent::TitleChanged {
                        backend_id: id,
                        title: "new".to_owned(),
                    },
                ],
            ))
            .unwrap();

        assert_eq!(session.window_metadata(WinId(0)).unwrap().title, "new");
        assert_eq!(session.backend.assignment_attempts.len(), 1);
        assert!(session.backend.call_order[0].starts_with("assign:ordered:"));
        assert_eq!(session.backend.call_order[1], "respond:3");
        assert!(session.backend.responses[0].2.projection.as_ref().unwrap()[0].focused);
    }

    #[test]
    fn exclusive_focus_true_then_false_clears_and_restores_ledger_focus() {
        let mut session = policy_live_session(FakeBackend::new());
        session
            .handle_backend_event(policy_turn(3, vec![policy_window_opened("focus", "title")]))
            .unwrap();
        session.backend.responses.clear();

        let lost = session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::ExclusiveFocusChanged(true)],
            ))
            .unwrap();
        assert!(session.backend.responses[0]
            .2
            .projection
            .as_ref()
            .unwrap()
            .iter()
            .all(|placement| !placement.focused));
        assert!(lost.state.unwrap().focused_title.is_empty());
        assert_eq!(session.ledger().focused(), Some(WinId(0)));
        session.backend.responses.clear();

        let restored = session
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::ExclusiveFocusChanged(false)],
            ))
            .unwrap();
        assert!(session.backend.responses[0].2.projection.as_ref().unwrap()[0].focused);
        assert_eq!(restored.state.unwrap().focused_title, "title");
        assert_eq!(session.ledger().focused(), Some(WinId(0)));
    }

    #[test]
    fn ordinary_focus_observation_cannot_change_ledger_policy() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focused = session.ledger().focused();

        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::FocusChanged(Some(
                    BackendWindowId::new("r1").unwrap(),
                ))],
            ))
            .unwrap();

        assert_eq!(session.ledger().focused(), focused);
        let projection = session.backend.responses[0].2.projection.as_ref().unwrap();
        assert_eq!(
            projection
                .iter()
                .find(|placement| placement.focused)
                .map(|p| p.win),
            focused
        );
    }

    #[test]
    fn policy_event_limit_accepts_exact_max_and_rejects_max_plus_one_atomically() {
        let mut session = policy_live_session(FakeBackend::new());
        let event = BackendPolicyEvent::ModifiersChanged {
            old: Vec::new(),
            new: Vec::new(),
        };
        session
            .handle_backend_event(policy_turn(3, vec![event.clone(); MAX_POLICY_EVENTS]))
            .unwrap();
        assert_eq!(session.backend.responses.len(), 1);

        let authority = session.current_authority();
        let visible_state = session.state().clone();
        let visible_ledger = session.visible_ledger(None);
        let published_ledger = session.published_ledger.clone();
        let published_windows = session.published_windows.clone();
        let snapshot = session.snapshot();
        let last_projection = session.last_projection().to_vec();
        let assignments = session.backend.assignment_attempts.clone();
        let bound_windows = session.backend.bound_windows.clone();
        let responses = session.backend.responses.clone();
        let call_order = session.backend.call_order.clone();
        let request_attempts = session.backend.request_attempts;
        let watermark = session.last_backend_ticket;
        let last_turn = session.last_policy_turn;
        let transaction = session.transaction;
        let repeat_target = session.repeat_target;
        let mut too_many = vec![policy_window_opened("private", "private")];
        too_many.extend(vec![event; MAX_POLICY_EVENTS]);
        let error = session
            .handle_backend_event(policy_turn(4, too_many))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::Backend(BackendError::Capacity {
                resource: BackendCapacityResource::PolicyFacts,
                ..
            })
        ));
        let after = session.current_authority();
        assert_eq!(&after.ledger, &authority.ledger);
        assert_eq!(&after.windows, &authority.windows);
        assert_eq!(&after.backend_ids, &authority.backend_ids);
        assert_eq!(&after.bound_backend_ids, &authority.bound_backend_ids);
        assert_eq!(after.next_win_id, authority.next_win_id);
        assert_eq!(after.workarea, authority.workarea);
        assert_eq!(after.exclusive_focus, authority.exclusive_focus);
        assert_eq!(&after.effective_focus, &authority.effective_focus);
        assert_eq!(after.mode, authority.mode);
        assert_eq!(&after.chord_echo, &authority.chord_echo);
        assert_eq!(after.whichkey, authority.whichkey);
        assert_eq!(&after.modules, &authority.modules);
        assert_eq!(&after.active_modifiers, &authority.active_modifiers);
        assert_eq!(&after.held_bindings, &authority.held_bindings);
        assert_eq!(session.backend.assignment_attempts, assignments);
        assert_eq!(session.backend.bound_windows, bound_windows);
        assert_eq!(session.last_backend_ticket, watermark);
        assert_eq!(session.backend.responses, responses);
        assert_eq!(session.backend.call_order, call_order);
        assert_eq!(session.backend.request_attempts, request_attempts);
        assert_eq!(session.state(), &visible_state);
        assert_eq!(session.visible_ledger(None), visible_ledger);
        assert_eq!(session.published_ledger, published_ledger);
        assert_eq!(session.published_windows, published_windows);
        assert_eq!(session.snapshot(), snapshot);
        assert_eq!(session.last_projection(), last_projection);
        assert_eq!(session.last_policy_turn, last_turn);
        assert_eq!(session.transaction, transaction);
        assert_eq!(session.repeat_target, repeat_target);
        assert!(session.active.is_none());
    }

    #[test]
    fn combined_effect_limit_rejects_external_plus_batch_overflow_atomically() {
        let mut session = policy_live_session_with_keymap(FakeBackend::new(), spawn_keymap());
        let spawn = binding_id(&session, "x");
        session
            .backend
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::BindingPressed(spawn)],
            ))
            .unwrap();
        let ticket = session.backend.responses[0].1;
        session
            .handle_backend_event(BackendEvent::OperationCompleted {
                ticket,
                result: Ok(()),
            })
            .unwrap();
        let responses = session.backend.responses.len();

        let error = session
            .handle_backend_event(draining_policy_turn(
                4,
                ticket,
                vec![BackendPolicyEvent::BindingPressed(spawn); MAX_STAGED_EFFECTS],
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::Backend(BackendError::Capacity {
                resource: BackendCapacityResource::PolicyEffects,
                ..
            })
        ));
        assert_eq!(session.backend.responses.len(), responses);
    }

    #[test]
    fn managed_window_limit_accepts_256_and_rejects_257_before_assignment() {
        let replay = |count: usize| {
            let mut events = (0..count)
                .map(|index| policy_window_opened(&format!("w-{index}"), "title"))
                .collect::<Vec<_>>();
            events.push(BackendPolicyEvent::InitialReplayComplete);
            events
        };

        let mut accepted = Session::connect(FakeBackend::new()).unwrap();
        accepted
            .handle_backend_event(policy_turn(1, replay(MAX_MANAGED_WINDOWS)))
            .unwrap();
        assert_eq!(
            accepted.backend.assignment_attempts.len(),
            MAX_MANAGED_WINDOWS
        );

        let mut rejected = Session::connect(FakeBackend::new()).unwrap();
        let error = rejected
            .handle_backend_event(policy_turn(1, replay(MAX_MANAGED_WINDOWS + 1)))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::Backend(BackendError::Capacity {
                resource: BackendCapacityResource::ManagedWindows,
                ..
            })
        ));
        assert!(rejected.backend.assignment_attempts.is_empty());
        assert!(rejected.backend.responses.is_empty());
    }

    #[test]
    fn binding_limit_accepts_64_and_rejects_65_before_configuration() {
        let keymap = |count: usize| Keymap {
            modifier: "mod".to_owned(),
            bindings: (0..count)
                .map(|index| {
                    test_binding(&format!("key-{index}"), Action::Launcher, Mode::Nav, false)
                })
                .collect(),
        };
        let accepted_backend = FakeBackend::new();
        let accepted_calls = accepted_backend.configure_calls.clone();
        Session::connect_with_keymap(accepted_backend, keymap(MAX_CONFIGURED_BINDINGS)).unwrap();
        assert_eq!(*accepted_calls.lock().unwrap(), 1);

        let rejected_backend = FakeBackend::new();
        let rejected_calls = rejected_backend.configure_calls.clone();
        let error = match Session::connect_with_keymap(
            rejected_backend,
            keymap(MAX_CONFIGURED_BINDINGS + 1),
        ) {
            Ok(_) => panic!("65 bindings must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            BackendError::Capacity {
                resource: BackendCapacityResource::ConfiguredBindings,
                ..
            }
        ));
        assert_eq!(*rejected_calls.lock().unwrap(), 0);
    }

    #[test]
    fn replay_barrier_is_single_final_and_forbidden_elsewhere() {
        let mut not_final = Session::connect(FakeBackend::new()).unwrap();
        assert!(not_final
            .handle_backend_event(policy_turn(
                1,
                vec![
                    BackendPolicyEvent::InitialReplayComplete,
                    policy_window_opened("late", "late"),
                ],
            ))
            .is_err());
        assert!(not_final.backend.responses.is_empty());

        let mut repeated = Session::connect(FakeBackend::new()).unwrap();
        assert!(repeated
            .handle_backend_event(policy_turn(
                1,
                vec![
                    BackendPolicyEvent::InitialReplayComplete,
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .is_err());
        assert!(repeated.backend.responses.is_empty());

        let mut later = Session::connect(FakeBackend::new()).unwrap();
        later
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        assert!(later
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .is_err());
        assert_eq!(later.backend.responses.len(), 1);

        let mut repeated_focus = Session::connect(FakeBackend::new()).unwrap();
        assert!(repeated_focus
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("focus", "focus"),
                    BackendPolicyEvent::FocusChanged(Some(BackendWindowId::new("focus").unwrap(),)),
                    BackendPolicyEvent::FocusChanged(None),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .is_err());
        assert!(repeated_focus.backend.assignment_attempts.is_empty());
        assert!(repeated_focus.backend.responses.is_empty());
    }

    #[test]
    fn initial_replay_is_silent_and_rebinds_each_identity_once() {
        let snapshot = recovery_snapshot();
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(snapshot)).unwrap();

        let update = session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("restored-7", "first"),
                    policy_window_opened("restored-7", "latest"),
                    policy_window_opened("new-10", "new"),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap();

        assert!(update.state.is_none());
        assert!(update.persistence.is_none());
        assert_eq!(session.backend.assignment_attempts.len(), 2);
        assert_eq!(
            session.backend.assignment_attempts[0],
            (BackendWindowId::new("restored-7").unwrap(), WinId(7))
        );
        assert_eq!(
            session.backend.assignment_attempts[1],
            (BackendWindowId::new("new-10").unwrap(), WinId(10))
        );
        assert_eq!(session.window_metadata(WinId(7)).unwrap().title, "latest");
    }

    #[test]
    fn initial_replay_is_projection_free_and_stays_unpublished() {
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(recovery_snapshot())).unwrap();

        let update = session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("restored-7", "restored"),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap();

        assert_eq!(session.phase(), RecoveryPhase::FinalizingReplay);
        assert!(session.backend.responses[0].2.projection.is_none());
        assert!(session.backend.responses[0].2.closes.is_empty());
        assert!(session.backend.responses[0].2.bindings.enabled.is_empty());
        assert_eq!(session.state().revision, 0);
        assert!(session.snapshot().is_none());
        assert!(session.last_projection().is_empty());
        assert!(update.state.is_none());

        let mut pending = Session::connect(FakeBackend::new()).unwrap();
        pending
            .backend
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        let update = pending
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        assert_eq!(pending.phase(), RecoveryPhase::FinalizingReplay);
        assert!(pending.has_active_backend_transaction());
        assert!(update.state.is_none());
        assert!(update.persistence.is_none());
    }

    #[test]
    fn replay_barrier_reconciles_without_projection() {
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(recovery_snapshot())).unwrap();

        session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("restored-7", "restored"),
                    policy_window_opened("new-10", "new"),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap();

        assert_eq!(
            session.ledger().active_orbit().windows,
            [WinId(7), WinId(10)]
        );
        assert_eq!(
            session.window_id(&BackendWindowId::new("missing-9").unwrap()),
            None
        );
        assert_eq!(
            session.window_id(&BackendWindowId::new("new-10").unwrap()),
            Some(WinId(10))
        );
        assert_eq!(session.backend.responses[0].2.projection, None);
    }

    #[test]
    fn initial_replay_rejects_workarea_before_assignment() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();

        let error = session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("private", "private"),
                    BackendPolicyEvent::WorkareaChanged(selected_workarea()),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::UnexpectedInitialReplayPolicyEvent(
                BackendPolicyEvent::WorkareaChanged(_)
            )
        ));
        assert!(session.backend.assignment_attempts.is_empty());
        assert!(session.backend.responses.is_empty());
        assert!(session.ledger().is_empty());
        assert_eq!(session.last_backend_ticket, 0);

        let mut precedence = Session::connect(FakeBackend::new()).unwrap();
        let mut events = (0..=MAX_MANAGED_WINDOWS)
            .map(|index| policy_window_opened(&format!("w-{index}"), "title"))
            .collect::<Vec<_>>();
        events.push(BackendPolicyEvent::WorkareaChanged(selected_workarea()));
        events.push(BackendPolicyEvent::InitialReplayComplete);
        let error = precedence
            .handle_backend_event(policy_turn(1, events))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::UnexpectedInitialReplayPolicyEvent(
                BackendPolicyEvent::WorkareaChanged(_)
            )
        ));
        assert!(precedence.backend.assignment_attempts.is_empty());
        assert!(precedence.backend.responses.is_empty());
        assert_eq!(precedence.last_backend_ticket, 0);
    }

    #[test]
    fn first_selected_workarea_projects_and_publishes_once() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("first", "title"),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap();

        let first = session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();
        let second = session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();

        assert_eq!(session.phase(), RecoveryPhase::Live);
        assert_eq!(first.state.as_ref().map(|state| state.revision), Some(1));
        assert!(first.persistence.is_some());
        assert!(session.backend.responses[1].2.projection.is_some());
        assert!(second.state.is_none());
        assert!(session.backend.responses[2].2.projection.is_none());
    }

    #[test]
    fn initial_workarea_projection_with_exclusive_focus_suppresses_window_focus() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        let id = BackendWindowId::new("focus").unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("focus", "title"),
                    BackendPolicyEvent::FocusChanged(Some(id)),
                    BackendPolicyEvent::ExclusiveFocusChanged(true),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap();

        let update = session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();

        assert!(session.backend.responses[1]
            .2
            .projection
            .as_ref()
            .unwrap()
            .iter()
            .all(|placement| !placement.focused));
        assert!(update.state.unwrap().focused_title.is_empty());
        assert_eq!(session.ledger().focused(), Some(WinId(0)));
    }

    #[test]
    fn initial_workarea_enables_nav_bindings_after_disabled_replay() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        let disabled = session.backend.responses[0].2.bindings.clone();

        session
            .handle_backend_event(policy_turn(
                2,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();
        let enabled = &session.backend.responses[1].2.bindings;

        assert!(disabled.enabled.is_empty());
        assert_eq!(disabled.next_key_edge, BackendNextKeyEdge::Preserve);
        assert_eq!(enabled.enabled.len(), Keymap::default().bindings.len());
        assert_eq!(enabled.watched_modifiers, [BackendModifier::Super]);
    }

    #[test]
    fn pre_workarea_authoritative_turns_accumulate_until_revision_one() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        let id = BackendWindowId::new("pre-live").unwrap();
        let accumulated = session
            .handle_backend_event(policy_turn(
                2,
                vec![
                    policy_window_opened("pre-live", "old"),
                    BackendPolicyEvent::TitleChanged {
                        backend_id: id.clone(),
                        title: "latest".to_owned(),
                    },
                    BackendPolicyEvent::FocusChanged(Some(id)),
                    BackendPolicyEvent::ExclusiveFocusChanged(false),
                ],
            ))
            .unwrap();
        assert!(accumulated.state.is_none());
        assert!(session.backend.responses[1].2.projection.is_none());

        let published = session
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::WorkareaChanged(selected_workarea())],
            ))
            .unwrap();
        assert_eq!(published.state.as_ref().unwrap().revision, 1);
        assert_eq!(published.state.as_ref().unwrap().focused_title, "latest");
        assert_eq!(session.ledger().active_orbit().windows, [WinId(0)]);
    }

    #[test]
    fn pre_workarea_binding_input_is_fatal_before_reduction() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(policy_turn(
                1,
                vec![BackendPolicyEvent::InitialReplayComplete],
            ))
            .unwrap();
        let binding = binding_id(&session, "j");
        let assignments = session.backend.assignment_attempts.len();
        let responses = session.backend.responses.len();

        let error = session
            .handle_backend_event(policy_turn(
                2,
                vec![
                    policy_window_opened("private", "private"),
                    BackendPolicyEvent::BindingPressed(binding),
                ],
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::InvalidPolicySequence)
        ));
        assert_eq!(session.backend.assignment_attempts.len(), assignments);
        assert_eq!(session.backend.responses.len(), responses);
        assert!(session.ledger().is_empty());
    }

    #[test]
    fn pre_barrier_non_replay_events_are_protocol_errors() {
        let forbidden = [
            BackendPolicyEvent::TitleChanged {
                backend_id: BackendWindowId::new("unknown").unwrap(),
                title: "title".to_owned(),
            },
            BackendPolicyEvent::GeometryDrifted {
                backend_id: BackendWindowId::new("unknown").unwrap(),
                rect: Rect::new(0, 0, 1, 1),
            },
            BackendPolicyEvent::UnboundKeyEaten,
        ];

        for event in forbidden {
            let mut session = Session::connect(FakeBackend::new()).unwrap();
            assert!(session
                .handle_backend_event(policy_turn(
                    1,
                    vec![event, BackendPolicyEvent::InitialReplayComplete],
                ))
                .is_err());
            assert!(session.backend.responses.is_empty());
        }
    }

    #[test]
    fn maximum_replay_with_focus_and_barrier_is_accepted() {
        assert_eq!(MAX_REPLAY_POLICY_EVENTS, MAX_MANAGED_WINDOWS + 3);
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        let mut events = (0..MAX_MANAGED_WINDOWS)
            .map(|index| policy_window_opened(&format!("w-{index}"), "title"))
            .collect::<Vec<_>>();
        events.push(BackendPolicyEvent::FocusChanged(Some(
            BackendWindowId::new(format!("w-{}", MAX_MANAGED_WINDOWS - 1)).unwrap(),
        )));
        events.push(BackendPolicyEvent::ExclusiveFocusChanged(false));
        events.push(BackendPolicyEvent::InitialReplayComplete);

        session
            .handle_backend_event(policy_turn(1, events))
            .unwrap();

        assert_eq!(
            session.backend.assignment_attempts.len(),
            MAX_MANAGED_WINDOWS
        );
        assert_eq!(session.backend.responses.len(), 1);
        assert!(session.backend.responses[0].2.projection.is_none());
    }

    #[test]
    fn maximum_policy_batch_preserves_input_order_and_is_bounded() {
        let keymap = Keymap {
            modifier: "mod".to_owned(),
            bindings: vec![
                test_binding("a", Action::Spawn(vec!["a".to_owned()]), Mode::Nav, false),
                test_binding("b", Action::Spawn(vec!["b".to_owned()]), Mode::Nav, false),
            ],
        };
        let mut session = policy_live_session_with_keymap(FakeBackend::new(), keymap);
        let a = binding_id(&session, "a");
        let b = binding_id(&session, "b");
        let events = (0..MAX_POLICY_EVENTS)
            .map(|index| BackendPolicyEvent::BindingPressed(if index % 2 == 0 { a } else { b }))
            .collect();

        let update = session
            .handle_backend_event(policy_turn(3, events))
            .unwrap();

        assert_eq!(update.effects.len(), MAX_STAGED_EFFECTS);
        for (index, effect) in update.effects.iter().enumerate() {
            let expected = if index % 2 == 0 { "a" } else { "b" };
            assert_eq!(effect, &SessionEffect::Spawn(vec![expected.to_owned()]));
        }
        assert_eq!(session.backend.responses.len(), 1);
    }

    #[test]
    fn policy_text_and_modifier_bounds_are_atomic() {
        let mut exact = policy_live_session(FakeBackend::new());
        let exact_id = BackendWindowId::new("exact-text").unwrap();
        exact
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::WindowOpened {
                    backend_id: exact_id.clone(),
                    app_id: String::new(),
                    title: "x".repeat(MAX_POLICY_TEXT_BYTES - exact_id.as_str().len()),
                }],
            ))
            .unwrap();
        exact
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::ModifiersChanged {
                    old: Vec::new(),
                    new: vec![
                        BackendModifier::Shift,
                        BackendModifier::Control,
                        BackendModifier::Alt,
                        BackendModifier::Super,
                    ],
                }],
            ))
            .unwrap();
        assert_eq!(exact.backend.responses.len(), 2);

        let mut too_long = policy_live_session(FakeBackend::new());
        let too_long_id = BackendWindowId::new("too-long").unwrap();
        let error = too_long
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::WindowOpened {
                    backend_id: too_long_id.clone(),
                    app_id: String::new(),
                    title: "x".repeat(MAX_POLICY_TEXT_BYTES + 1 - too_long_id.as_str().len()),
                }],
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::Backend(BackendError::Capacity {
                resource: BackendCapacityResource::PolicyTextBytes,
                ..
            })
        ));
        assert!(too_long.backend.assignment_attempts.is_empty());
        assert!(too_long.backend.responses.is_empty());

        let mut noncanonical = policy_live_session(FakeBackend::new());
        let error = noncanonical
            .handle_backend_event(policy_turn(
                3,
                vec![BackendPolicyEvent::ModifiersChanged {
                    old: Vec::new(),
                    new: vec![BackendModifier::Super, BackendModifier::Shift],
                }],
            ))
            .unwrap_err();
        assert!(matches!(
            error,
            SessionEventError::BackendContract(BackendContractError::NonCanonicalModifiers)
        ));
        assert!(noncanonical.backend.responses.is_empty());
    }

    #[test]
    fn exhausted_watermark_never_allocates_the_sentinel() {
        let snapshot = SessionSnapshotV1::new(Ledger::new(), Vec::new(), u64::MAX).unwrap();
        let mut session =
            Session::connect_with_snapshot(FakeBackend::new(), Some(snapshot)).unwrap();

        let error = session
            .handle_backend_event(policy_turn(
                1,
                vec![
                    policy_window_opened("overflow", "overflow"),
                    BackendPolicyEvent::InitialReplayComplete,
                ],
            ))
            .unwrap_err();

        assert_eq!(error, SessionEventError::WindowIdExhausted);
        assert!(session.backend.assignment_attempts.is_empty());
        assert!(session.backend.responses.is_empty());
        assert!(session.ledger().is_empty());
        assert_eq!(session.last_backend_ticket, 0);
    }

    #[test]
    fn armed_repeat_requests_internal_turn_without_action_completion() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focus = binding_id(&session, "j");
        let pressed = session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();
        assert!(matches!(
            pressed.repeat_timer,
            RepeatTimerDirective::Arm { .. }
        ));
        session.backend.responses.clear();
        let requests = session.backend.request_attempts;

        let fired = session.fire_key_repeat().unwrap();

        assert_eq!(session.backend.request_attempts, requests + 1);
        assert!(fired.pending_action.is_none());
        assert!(fired.action_completion.is_none());
        let completed = session
            .handle_backend_event(policy_turn(5, Vec::new()))
            .unwrap();
        assert!(completed.action_completion.is_none());
        assert_eq!(session.ledger().focused(), Some(WinId(1)));
    }

    #[test]
    fn internal_repeat_empty_turn_carries_repeated_projection() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focus = binding_id(&session, "j");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();
        let previous_projection = session.last_projection().to_vec();
        session.backend.responses.clear();

        assert_eq!(
            session.fire_key_repeat().unwrap(),
            SessionUpdate::unchanged()
        );
        let completed = session
            .handle_backend_event(policy_turn(5, Vec::new()))
            .unwrap();

        let response_projection = session.backend.responses[0]
            .2
            .projection
            .clone()
            .expect("the internal response carries the repeated projection");
        assert_ne!(response_projection, previous_projection);
        assert_eq!(
            response_projection
                .iter()
                .find(|placement| placement.focused)
                .map(|placement| placement.win),
            Some(WinId(1))
        );
        assert_eq!(session.last_projection(), response_projection);
        assert!(completed.projection_applied);
        assert!(completed.action_completion.is_none());
    }

    #[test]
    fn released_repeat_is_noop_before_request() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focus = binding_id(&session, "j");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();
        let released = session
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingReleased(focus)],
            ))
            .unwrap();
        assert_eq!(released.repeat_timer, RepeatTimerDirective::Disarm);
        let requests = session.backend.request_attempts;

        let fired = session.fire_key_repeat().unwrap();

        assert_eq!(fired, SessionUpdate::unchanged());
        assert_eq!(session.backend.request_attempts, requests);
    }

    #[test]
    fn new_repeatable_press_replaces_without_resuming_older_target() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let next = binding_id(&session, "j");
        let previous = binding_id(&session, "k");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(next)],
            ))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingPressed(previous)],
            ))
            .unwrap();
        session
            .handle_backend_event(policy_turn(
                6,
                vec![BackendPolicyEvent::BindingReleased(previous)],
            ))
            .unwrap();
        let requests = session.backend.request_attempts;

        assert_eq!(
            session.fire_key_repeat().unwrap(),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.request_attempts, requests);
    }

    #[test]
    fn same_batch_replacement_press_release_never_resumes_old_target() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let older = binding_id(&session, "j");
        let replacement = binding_id(&session, "k");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(older)],
            ))
            .unwrap();
        assert_eq!(session.repeat_target, Some(older));

        let replaced = session
            .handle_backend_event(policy_turn(
                5,
                vec![
                    BackendPolicyEvent::BindingPressed(replacement),
                    BackendPolicyEvent::BindingReleased(replacement),
                ],
            ))
            .unwrap();

        assert_eq!(replaced.repeat_timer, RepeatTimerDirective::Disarm);
        assert!(session.held_bindings.contains(&older));
        assert!(!session.held_bindings.contains(&replacement));
        assert_eq!(session.repeat_target, None);
        let requests = session.backend.request_attempts;
        assert_eq!(
            session.fire_key_repeat().unwrap(),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.request_attempts, requests);
    }

    #[test]
    fn same_batch_replacement_press_repeat_stop_never_resumes_old_target() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let older = binding_id(&session, "j");
        let replacement = binding_id(&session, "k");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(older)],
            ))
            .unwrap();
        assert_eq!(session.repeat_target, Some(older));

        let replaced = session
            .handle_backend_event(policy_turn(
                5,
                vec![
                    BackendPolicyEvent::BindingPressed(replacement),
                    BackendPolicyEvent::BindingRepeatStopped(replacement),
                ],
            ))
            .unwrap();

        assert_eq!(replaced.repeat_timer, RepeatTimerDirective::Disarm);
        assert!(session.held_bindings.contains(&older));
        assert!(!session.held_bindings.contains(&replacement));
        assert_eq!(session.repeat_target, None);
        let requests = session.backend.request_attempts;
        assert_eq!(
            session.fire_key_repeat().unwrap(),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.request_attempts, requests);
    }

    #[test]
    fn mode_change_disables_armed_repeat_target() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focus = binding_id(&session, "j");
        let resize = binding_id(&session, "r");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();

        let changed = session
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();
        let requests = session.backend.request_attempts;

        assert_eq!(changed.repeat_timer, RepeatTimerDirective::Disarm);
        assert_eq!(
            session.fire_key_repeat().unwrap(),
            SessionUpdate::unchanged()
        );
        assert_eq!(session.backend.request_attempts, requests);
    }

    #[test]
    fn repeat_tick_during_internal_in_flight_is_consumed_without_second_request() {
        let mut session = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut session);
        let focus = binding_id(&session, "j");
        session
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();
        session.fire_key_repeat().unwrap();
        session
            .backend
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        session
            .handle_backend_event(policy_turn(5, Vec::new()))
            .unwrap();
        let requests = session.backend.request_attempts;

        let tick = session.fire_key_repeat().unwrap();

        assert_eq!(tick, SessionUpdate::unchanged());
        assert_eq!(session.backend.request_attempts, requests);
        assert!(session.has_active_backend_transaction());
    }

    #[test]
    fn repeat_timer_directives_cover_final_press_pending_release_mode_and_quit() {
        let mut press_release = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut press_release);
        let focus = binding_id(&press_release, "j");
        let pressed = press_release
            .handle_backend_event(policy_turn(
                4,
                vec![BackendPolicyEvent::BindingPressed(focus)],
            ))
            .unwrap();
        assert_eq!(
            pressed.repeat_timer,
            RepeatTimerDirective::Arm {
                delay: Duration::from_millis(KEY_REPEAT_DELAY_MS),
                interval: Duration::from_millis(1_000 / u64::from(KEY_REPEAT_RATE_HZ)),
            }
        );
        press_release
            .backend
            .response_results
            .push_back(Ok(BackendSubmission::Pending));
        let pending_release = press_release
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingReleased(focus)],
            ))
            .unwrap();
        assert_eq!(pending_release.repeat_timer, RepeatTimerDirective::Disarm);

        let mut mode = policy_live_session(FakeBackend::new());
        open_two_policy_windows(&mut mode);
        let focus = binding_id(&mode, "j");
        let resize = binding_id(&mode, "r");
        mode.handle_backend_event(policy_turn(
            4,
            vec![BackendPolicyEvent::BindingPressed(focus)],
        ))
        .unwrap();
        let disabling = mode
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingPressed(resize)],
            ))
            .unwrap();
        assert_eq!(disabling.repeat_timer, RepeatTimerDirective::Disarm);

        let mut quit_keymap = Keymap::default();
        quit_keymap
            .bindings
            .push(test_binding("x", Action::Quit, Mode::Nav, false));
        let mut quit = policy_live_session_with_keymap(FakeBackend::new(), quit_keymap);
        open_two_policy_windows(&mut quit);
        let focus = binding_id(&quit, "j");
        let quit_id = binding_id(&quit, "x");
        quit.handle_backend_event(policy_turn(
            4,
            vec![BackendPolicyEvent::BindingPressed(focus)],
        ))
        .unwrap();
        let quitting = quit
            .handle_backend_event(policy_turn(
                5,
                vec![BackendPolicyEvent::BindingPressed(quit_id)],
            ))
            .unwrap();
        assert_eq!(quitting.repeat_timer, RepeatTimerDirective::Disarm);
        assert!(matches!(
            quitting.effects.as_slice(),
            [SessionEffect::QuitPending { .. }]
        ));
    }
}
