//! Transactional session state and projection coordination.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use realm_core::ipc::Capabilities;
use realm_core::ipc::PROTOCOL_VERSION;
use realm_core::layout::{project, Layout, Placement, TriptychParams, Workarea};
use realm_core::ledger::{Dir, Orbit, ORBIT_COUNT};
use realm_core::state::{Module, OrbitCell, OrbitDisplay, RealmState};
use realm_core::{Ledger, OrbitId, WinId};
use serde::de::{Error as _, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize};

use crate::backend::{BackendError, BackendEvent, BackendResult, BackendWindowId, WmBackend};

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
            let bytes = binding.backend_id.0.as_bytes();
            if bytes.is_empty()
                || bytes.len() > 32
                || !bytes.iter().all(|byte| (0x20..=0x7e).contains(byte))
            {
                return Err(SessionSnapshotError::Invalid(
                    "backend identity must be 1-32 printable ASCII bytes",
                ));
            }
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

/// Observable result of one in-process session transition.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUpdate {
    /// Whether one projection was successfully submitted to the backend.
    pub projection_applied: bool,
    /// The new visible snapshot, present only when it differs from the last one.
    pub state: Option<RealmState>,
    /// A valid event intentionally left for a later policy slice.
    pub deferred: Option<BackendEvent>,
}

impl SessionUpdate {
    fn unchanged() -> Self {
        Self {
            projection_applied: false,
            state: None,
            deferred: None,
        }
    }

    fn deferred(event: BackendEvent) -> Self {
        Self {
            projection_applied: false,
            state: None,
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
    /// Every possible numeric window id has already been allocated.
    #[error("Realm window id space is exhausted")]
    WindowIdExhausted,
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
}

/// Failure of a caller-requested session action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionActionError {
    /// The session is recovering or must repair pending backend work first.
    #[error("session is not ready for actions")]
    NotReady,
    /// The action reached the compositor backend and it failed.
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// Result of a caller-requested session action.
pub type SessionActionResult<T> = Result<T, SessionActionError>;

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
}

impl<B: WmBackend> Session<B> {
    /// Connect a backend and seed an empty six-orbit session.
    pub fn connect(backend: B) -> BackendResult<Self> {
        Self::connect_with_snapshot(backend, None)
    }

    /// Connect a backend and enter initial replay using an optional validated snapshot.
    pub fn connect_with_snapshot(
        mut backend: B,
        snapshot: Option<SessionSnapshotV1>,
    ) -> BackendResult<Self> {
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
        })
    }

    /// Current recovery phase.
    pub fn phase(&self) -> RecoveryPhase {
        self.phase
    }

    /// True when backend repair must run before another event is read.
    pub fn has_pending_backend_work(&self) -> bool {
        self.pending_backend_work
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
    pub fn request_close_focused(&mut self) -> SessionActionResult<Option<WinId>> {
        if self.phase != RecoveryPhase::Live || self.pending_backend_work {
            return Err(SessionActionError::NotReady);
        }
        let Some(win) = self.ledger.focused() else {
            return Ok(None);
        };
        self.backend.close(win)?;
        Ok(Some(win))
    }

    /// Toggle the visible which-key strip without applying a projection.
    pub fn toggle_whichkey(&mut self) -> SessionUpdate {
        if self.phase != RecoveryPhase::Live || self.pending_backend_work {
            return SessionUpdate::unchanged();
        }
        let mut changed = self.state.clone();
        changed.revision = changed.revision.saturating_add(1);
        changed.whichkey = !changed.whichkey;
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            deferred: None,
        }
    }

    fn stage_ledger_update(
        &mut self,
        update: impl FnOnce(&mut Ledger),
    ) -> SessionActionResult<SessionUpdate> {
        if self.phase != RecoveryPhase::Live || self.pending_backend_work {
            return Err(SessionActionError::NotReady);
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
        result.map_err(SessionEventError::BackendRetryExhausted)
    }

    /// Apply one compositor event that has compositor-independent semantics.
    pub fn handle_backend_event(
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
            | BackendEvent::WindowClosed(_)) => {
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
            BackendEvent::Disconnected => Err(BackendError::Disconnected),
            event @ (BackendEvent::FocusChanged(_) | BackendEvent::ExclusiveFocusChanged(_)) => {
                Ok(SessionUpdate::deferred(event))
            }
        };
        match result {
            Ok(update) => Ok(update),
            Err(error) => {
                self.pending_backend_work = should_retry_authoritative(&error);
                Err(error.into())
            }
        }
    }

    /// Replace bar-module values without issuing a compositor request.
    pub fn update_modules(&mut self, modules: Vec<Module>) -> SessionUpdate {
        if self.phase != RecoveryPhase::Live || self.pending_backend_work {
            return SessionUpdate::unchanged();
        }
        if self.state.modules == modules {
            return SessionUpdate::unchanged();
        }
        let mut changed = self.state.clone();
        changed.revision = changed.revision.saturating_add(1);
        changed.modules = modules;
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            deferred: None,
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
                self.pending_backend_work = should_retry_authoritative(&error);
                Err(error.into())
            }
        }
    }

    fn finish_replay(&mut self) -> BackendResult<SessionUpdate> {
        self.bind_pending_windows()?;
        let projection = self.project(&self.ledger, self.workarea);
        self.apply_projection_if_needed(projection)?;
        self.phase = RecoveryPhase::Live;
        let state = self.publish_first_visible_state();
        Ok(SessionUpdate {
            projection_applied: true,
            state: Some(state),
            deferred: None,
        })
    }

    fn publish_first_visible_state(&mut self) -> RealmState {
        self.state = self.visible_state(1);
        self.state.clone()
    }

    fn repair_authoritative_projection(&mut self) -> BackendResult<SessionUpdate> {
        self.bind_pending_windows()?;
        let projection = self.project(&self.ledger, self.workarea);
        let projection_applied = self.apply_projection_if_needed(projection)?;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied,
            state,
            deferred: None,
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

    fn bind_pending_windows(&mut self) -> BackendResult<()> {
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
        self.bind_pending_windows()?;
        let projection = self.project(&ledger, workarea);
        let projection_applied = self.apply_projection_if_needed(projection)?;

        self.ledger = ledger;
        self.windows = windows;
        self.workarea = workarea;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied,
            state,
            deferred: None,
        })
    }

    fn commit_observed(
        &mut self,
        ledger: Ledger,
        windows: BTreeMap<WinId, WindowMetadata>,
        workarea: Workarea,
    ) -> BackendResult<SessionUpdate> {
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
            deferred: None,
        })
    }

    fn project(&self, ledger: &Ledger, workarea: Workarea) -> Vec<Placement> {
        project(ledger.active_orbit(), workarea, TriptychParams::default())
    }

    fn commit_visible_state(&mut self) -> SessionUpdate {
        let candidate = self.visible_state(self.state.revision);

        if self.state.renders_same_as(&candidate) {
            return SessionUpdate::unchanged();
        }

        let mut changed = candidate;
        changed.revision = self.state.revision.saturating_add(1);
        self.state = changed.clone();
        SessionUpdate {
            projection_applied: false,
            state: Some(changed),
            deferred: None,
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
            whichkey: self.state.whichkey,
            modules: self.state.modules.clone(),
        }
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
    use std::collections::BTreeMap;
    use std::os::fd::RawFd;
    use std::time::Instant;

    use realm_core::ipc::{Capabilities, PROTOCOL_VERSION};
    use realm_core::layout::{project, Layout, Placement, Rect, TriptychParams, Workarea};
    use realm_core::ledger::Dir;
    use realm_core::state::{Module, RealmState};
    use realm_core::{Ledger, OrbitId, WinId};

    use crate::backend::{BackendError, BackendEvent, BackendResult, BackendWindowId, WmBackend};

    use super::{
        RecoveryPhase, Session, SessionActionError, SessionEventError, SessionSnapshotV1,
        SessionUpdate, SnapshotBinding,
    };

    struct FakeBackend {
        connect_calls: usize,
        capabilities: Capabilities,
        workarea: Workarea,
        apply_attempts: Vec<Vec<Placement>>,
        successful_frames: Vec<Vec<Placement>>,
        fail_next_apply: Option<BackendError>,
        fail_next_assign: Option<BackendError>,
        assignment_attempts: Vec<(BackendWindowId, WinId)>,
        bound_windows: BTreeMap<BackendWindowId, WinId>,
        focus_calls: usize,
        close_attempts: Vec<WinId>,
        fail_next_close: Option<BackendError>,
        next_event_calls: usize,
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
                next_event_calls: 0,
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

        fn assign_window(&mut self, backend_id: &BackendWindowId, win: WinId) -> BackendResult<()> {
            self.assignment_attempts.push((backend_id.clone(), win));
            if let Some(error) = self.fail_next_assign.take() {
                return Err(error);
            }
            if let Some(bound) = self.bound_windows.get(backend_id) {
                if *bound != win {
                    return Err(BackendError::Unavailable {
                        message: "conflicting identity binding".to_owned(),
                    });
                }
                return Ok(());
            }
            self.bound_windows.insert(backend_id.clone(), win);
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

        fn event_fd(&self) -> RawFd {
            -1
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
            backend_id: BackendWindowId(id.to_owned()),
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
                    backend_id: BackendWindowId("restored-7".to_owned()),
                },
                SnapshotBinding {
                    win_id: WinId(9),
                    backend_id: BackendWindowId("missing-9".to_owned()),
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
                "\"backend_id\":\"restored-7\"",
                "\"backend_id\":\"\"",
                "backend identity must be 1-32 printable ASCII bytes",
            ),
            (
                "\"backend_id\":\"restored-7\"",
                "\"backend_id\":\"123456789012345678901234567890123\"",
                "backend identity must be 1-32 printable ASCII bytes",
            ),
            (
                "\"backend_id\":\"restored-7\"",
                "\"backend_id\":\"bad\\nidentity\"",
                "backend identity must be 1-32 printable ASCII bytes",
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
    fn initial_replay_is_silent_and_rebinds_each_identity_once() {
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
            session.window_id(&BackendWindowId("new-10".to_owned())),
            Some(WinId(10))
        );
        assert_eq!(session.backend.assignment_attempts.len(), 2);
        assert_eq!(
            session
                .backend
                .assignment_attempts
                .iter()
                .filter(|(backend_id, _)| backend_id.0 == "restored-7")
                .count(),
            1
        );
    }

    #[test]
    fn pre_barrier_non_replay_events_are_protocol_errors() {
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
            session.window_id(&BackendWindowId("must-wait".to_owned())),
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
    fn exhausted_watermark_never_allocates_the_sentinel() {
        let mut ledger = Ledger::new();
        ledger.summon(WinId(u64::MAX - 1), OrbitId::default());
        let snapshot = SessionSnapshotV1::new(
            ledger.clone(),
            vec![SnapshotBinding {
                win_id: WinId(u64::MAX - 1),
                backend_id: BackendWindowId("old".to_owned()),
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
                (BackendWindowId("r1".to_owned()), WinId(0)),
                (BackendWindowId("r2".to_owned()), WinId(1)),
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
        let backend_id = BackendWindowId("r1".to_owned());
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
    fn failed_identity_binding_preserves_observed_window_for_retry() {
        let backend_id = BackendWindowId("r1".to_owned());
        let backend = FakeBackend::new();
        let mut session = live_session(backend);
        session.backend.fail_next_assign = Some(BackendError::Io {
            message: "binding failed".to_owned(),
        });

        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap_err();

        assert_eq!(session.ledger().active_orbit().windows, [WinId(0)]);
        assert_eq!(session.window_id(&backend_id), Some(WinId(0)));
        assert_eq!(session.window_metadata(WinId(0)).unwrap().title, "one");
        assert_eq!(session.next_win_id, 1);
        assert_eq!(session.state().revision, 0);
        assert!(session.backend.apply_attempts.is_empty());
        assert!(session.backend.bound_windows.is_empty());

        let repaired = session.retry_pending_backend_work().unwrap();

        assert!(repaired.projection_applied);
        assert_eq!(repaired.state.unwrap().focused_title, "one");
        assert_eq!(
            session.backend.assignment_attempts,
            [
                (backend_id.clone(), WinId(0)),
                (backend_id.clone(), WinId(0))
            ]
        );
        assert_eq!(session.window_id(&backend_id), Some(WinId(0)));
        assert_eq!(session.next_win_id, 1);
    }

    #[test]
    fn replayed_backend_identity_reuses_its_window_id_without_rebinding() {
        let backend_id = BackendWindowId("r1".to_owned());
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
        assert_eq!(session.window_id(&BackendWindowId("r2".to_owned())), None);
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

        assert_eq!(session.request_close_focused().unwrap(), Some(WinId(0)));
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
    fn close_request_is_a_no_op_without_a_focused_window() {
        let mut session = live_session(FakeBackend::new());

        assert_eq!(session.request_close_focused().unwrap(), None);
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
}
