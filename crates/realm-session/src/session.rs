//! Transactional session state and projection coordination.

use std::collections::BTreeMap;

use realm_core::ipc::Capabilities;
use realm_core::layout::{project, Layout, Placement, TriptychParams, Workarea};
use realm_core::ledger::Dir;
use realm_core::state::{Module, OrbitCell, OrbitDisplay, RealmState};
use realm_core::{Ledger, OrbitId, WinId};

use crate::backend::{BackendError, BackendEvent, BackendResult, BackendWindowId, WmBackend};

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
}

impl<B: WmBackend> Session<B> {
    /// Connect a backend and seed an empty six-orbit session.
    pub fn connect(mut backend: B) -> BackendResult<Self> {
        let capabilities = backend.connect()?;
        let workarea = backend.workarea();
        Ok(Self {
            backend,
            ledger: Ledger::new(),
            capabilities,
            workarea,
            windows: BTreeMap::new(),
            backend_ids: BTreeMap::new(),
            pending_assignments: BTreeMap::new(),
            next_win_id: 0,
            last_projection: Vec::new(),
            projection_dirty: false,
            state: RealmState::default(),
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
    pub fn switch_orbit(&mut self, orbit: OrbitId) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.switch_orbit(orbit))
    }

    /// Change the active orbit's layout transactionally.
    pub fn set_layout(&mut self, layout: Layout) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.set_layout(layout))
    }

    /// Move focus by one ledger position transactionally.
    pub fn focus_step(&mut self, direction: Dir) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| ledger.focus_step(direction))
    }

    /// Swap the focused window with its neighbour transactionally.
    pub fn swap(&mut self, direction: Dir) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.swap(direction);
        })
    }

    /// Move the focused window to another orbit transactionally.
    pub fn move_focused_to_orbit(&mut self, orbit: OrbitId) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.move_to_orbit(orbit);
        })
    }

    /// Toggle the focused window's stowed state transactionally.
    pub fn toggle_stow(&mut self) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.toggle_stow();
        })
    }

    /// Toggle fullscreen for the focused window transactionally.
    pub fn toggle_fullscreen(&mut self) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.toggle_fullscreen();
        })
    }

    /// Restore the previous ledger state transactionally.
    pub fn undo(&mut self) -> BackendResult<SessionUpdate> {
        self.stage_ledger_update(|ledger| {
            ledger.undo();
        })
    }

    /// Ask the focused window to close without changing authoritative state.
    pub fn request_close_focused(&mut self) -> BackendResult<Option<WinId>> {
        let Some(win) = self.ledger.focused() else {
            return Ok(None);
        };
        self.backend.close(win)?;
        Ok(Some(win))
    }

    /// Toggle the visible which-key strip without applying a projection.
    pub fn toggle_whichkey(&mut self) -> SessionUpdate {
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
    ) -> BackendResult<SessionUpdate> {
        let mut candidate = self.ledger.clone();
        update(&mut candidate);
        self.commit_desired(candidate, self.windows.clone(), self.workarea)
    }

    /// Retry a projection left pending by a failed observed-state transition.
    ///
    /// This never mutates the ledger. A successful retry may release the
    /// visible state snapshot that was withheld with the failed attempt.
    pub fn retry_pending_projection(&mut self) -> BackendResult<SessionUpdate> {
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

    /// Apply one compositor event that has compositor-independent semantics.
    pub fn handle_backend_event(
        &mut self,
        event: BackendEvent,
    ) -> Result<SessionUpdate, SessionEventError> {
        match event {
            BackendEvent::WindowOpened {
                backend_id,
                app_id,
                title,
            } => {
                let win = self.record_window_identity(backend_id)?;
                self.ledger.summon(win, self.ledger.active());
                self.windows.insert(win, WindowMetadata { app_id, title });
                self.retry_pending_projection().map_err(Into::into)
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
                    .map_err(Into::into)
            }
            BackendEvent::TitleChanged { win, title } => {
                let Some(metadata) = self.windows.get_mut(&win) else {
                    return Ok(SessionUpdate::unchanged());
                };
                metadata.title = title;
                self.retry_pending_projection().map_err(Into::into)
            }
            BackendEvent::WorkareaChanged(workarea) => self
                .commit_observed(self.ledger.clone(), self.windows.clone(), workarea)
                .map_err(Into::into),
            BackendEvent::GeometryDrifted { .. } => Ok(SessionUpdate::unchanged()),
            BackendEvent::Disconnected => Err(BackendError::Disconnected.into()),
            event @ (BackendEvent::FocusChanged(_) | BackendEvent::ExclusiveFocusChanged(_)) => {
                Ok(SessionUpdate::deferred(event))
            }
        }
    }

    /// Replace bar-module values without issuing a compositor request.
    pub fn update_modules(&mut self, modules: Vec<Module>) -> SessionUpdate {
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
        let candidate = RealmState {
            revision: self.state.revision,
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
        };

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
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::fd::RawFd;
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{project, Layout, Placement, Rect, TriptychParams, Workarea};
    use realm_core::ledger::Dir;
    use realm_core::state::Module;
    use realm_core::{OrbitId, WinId};

    use crate::backend::{BackendError, BackendEvent, BackendResult, BackendWindowId, WmBackend};

    use super::Session;

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
        let mut session = Session::connect(FakeBackend::new()).unwrap();

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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        session
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();

        let first = session.retry_pending_projection().unwrap();
        let second = session.retry_pending_projection().unwrap();

        assert!(!first.projection_applied);
        assert!(!second.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), 1);
    }

    #[test]
    fn no_op_ledger_mutation_does_not_apply_or_emit() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();

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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(backend).unwrap();
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
            BackendError::Unsupported {
                capability: "exact-geometry".to_owned()
            }
        );
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);
        assert_eq!(session.backend.apply_attempts.len(), attempts + 1);
        assert_eq!(session.backend.successful_frames.len(), successes);
    }

    #[test]
    fn failed_desired_apply_marks_current_projection_dirty_for_repair() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let repaired = session.retry_pending_projection().unwrap();

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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
                .retry_pending_projection()
                .unwrap()
                .projection_applied
        );
        assert!(session.last_projection().is_empty());
    }

    #[test]
    fn failed_apply_after_workarea_change_retains_the_observed_area_for_retry() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
                .retry_pending_projection()
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let repaired = session
            .handle_backend_event(BackendEvent::WorkareaChanged(original))
            .unwrap();

        assert!(repaired.projection_applied);
        assert_eq!(session.backend.apply_attempts.len(), 3);
        assert_eq!(session.backend.apply_attempts.last().unwrap(), &p1);
        assert_eq!(session.backend.successful_frames.last().unwrap(), &p1);
    }

    #[test]
    fn failed_identity_binding_preserves_observed_window_for_retry() {
        let backend_id = BackendWindowId("r1".to_owned());
        let mut backend = FakeBackend::new();
        backend.fail_next_assign = Some(BackendError::Io {
            message: "binding failed".to_owned(),
        });
        let mut session = Session::connect(backend).unwrap();

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

        let repaired = session.retry_pending_projection().unwrap();

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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let repair = session.retry_pending_projection().unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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

        let mut singleton = Session::connect(FakeBackend::new()).unwrap();
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

        let mut moved = Session::connect(FakeBackend::new()).unwrap();
        moved
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        moved.move_focused_to_orbit(second).unwrap();
        assert!(moved.ledger().active_orbit().windows.is_empty());
        assert_eq!(moved.ledger().orbit(second).windows, [WinId(0)]);

        let mut stowed = Session::connect(FakeBackend::new()).unwrap();
        stowed
            .handle_backend_event(window_opened("r1", "one"))
            .unwrap();
        stowed.toggle_stow().unwrap();
        assert_eq!(stowed.ledger().active_orbit().stowed, [WinId(0)]);
        assert!(stowed.last_projection().is_empty());

        let mut fullscreen = Session::connect(FakeBackend::new()).unwrap();
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

        let mut switched = Session::connect(FakeBackend::new()).unwrap();
        switched.switch_orbit(second).unwrap();
        assert_eq!(switched.ledger().active(), second);

        let mut laid_out = Session::connect(FakeBackend::new()).unwrap();
        laid_out.set_layout(Layout::Mono).unwrap();
        assert_eq!(laid_out.ledger().active_orbit().layout, Layout::Mono);
    }

    #[test]
    fn failed_undo_does_not_consume_history() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let retry = session.undo().unwrap();
        assert!(retry.projection_applied);
        assert_eq!(session.ledger().active_orbit().layout, Layout::Triptych);
    }

    #[test]
    fn undo_never_removes_an_open_window_or_restores_a_closed_window() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
        let mut session = Session::connect(FakeBackend::new()).unwrap();

        assert_eq!(session.request_close_focused().unwrap(), None);
        assert!(session.backend.close_attempts.is_empty());
        assert_eq!(session.state().revision, 0);
        assert!(session.backend.apply_attempts.is_empty());
    }

    #[test]
    fn whichkey_toggle_only_emits_state_until_workarea_is_observed() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
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
