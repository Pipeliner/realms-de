//! Transactional session state and projection coordination.

use std::collections::BTreeMap;

use realm_core::ipc::Capabilities;
use realm_core::layout::{project, Placement, TriptychParams, Workarea};
use realm_core::state::{Module, OrbitCell, OrbitDisplay, RealmState};
use realm_core::{Ledger, WinId};

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
    next_win_id: u64,
    last_projection: Vec<Placement>,
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
            next_win_id: 0,
            last_projection: Vec::new(),
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

    /// Stage a ledger mutation and commit it only after backend application.
    pub fn update_ledger(
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
        let projection = self.project(&self.ledger, self.workarea);
        if projection == self.last_projection {
            return Ok(SessionUpdate::unchanged());
        }
        self.backend.apply(&projection)?;
        self.last_projection = projection;
        let state = self.commit_visible_state().state;
        Ok(SessionUpdate {
            projection_applied: true,
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
                let win = self.assign_window(backend_id)?;
                let mut ledger = self.ledger.clone();
                ledger.summon(win, ledger.active());
                let mut windows = self.windows.clone();
                windows.insert(win, WindowMetadata { app_id, title });
                self.commit_observed(ledger, windows, self.workarea)
                    .map_err(Into::into)
            }
            BackendEvent::WindowClosed(win) => {
                let mut ledger = self.ledger.clone();
                ledger.banish(win);
                let mut windows = self.windows.clone();
                windows.remove(&win);
                self.backend_ids.retain(|_, assigned| *assigned != win);
                self.commit_observed(ledger, windows, self.workarea)
                    .map_err(Into::into)
            }
            BackendEvent::TitleChanged { win, title } => {
                let Some(metadata) = self.windows.get_mut(&win) else {
                    return Ok(SessionUpdate::unchanged());
                };
                metadata.title = title;
                Ok(self.commit_visible_state())
            }
            BackendEvent::WorkareaChanged(workarea) => self
                .commit_observed(self.ledger.clone(), self.windows.clone(), workarea)
                .map_err(Into::into),
            BackendEvent::GeometryDrifted { .. } => Ok(SessionUpdate::unchanged()),
            BackendEvent::Disconnected => Err(BackendError::Disconnected.into()),
            event @ BackendEvent::FocusChanged(_) => Ok(SessionUpdate::deferred(event)),
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

    fn assign_window(&mut self, backend_id: BackendWindowId) -> Result<WinId, SessionEventError> {
        let (win, next) = match self.backend_ids.get(&backend_id).copied() {
            Some(win) => (win, self.next_win_id),
            None => (
                WinId(self.next_win_id),
                self.next_win_id
                    .checked_add(1)
                    .ok_or(SessionEventError::WindowIdExhausted)?,
            ),
        };
        self.backend.assign_window(&backend_id, win)?;
        self.backend_ids.insert(backend_id, win);
        self.next_win_id = next;
        Ok(win)
    }

    fn commit_desired(
        &mut self,
        ledger: Ledger,
        windows: BTreeMap<WinId, WindowMetadata>,
        workarea: Workarea,
    ) -> BackendResult<SessionUpdate> {
        let projection = self.project(&ledger, workarea);
        let projection_applied = projection != self.last_projection;
        if projection_applied {
            self.backend.apply(&projection)?;
        }

        self.ledger = ledger;
        self.windows = windows;
        self.workarea = workarea;
        if projection_applied {
            self.last_projection = projection;
        }
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
        let projection_applied = projection != self.last_projection;

        self.ledger = ledger;
        self.windows = windows;
        self.workarea = workarea;

        if projection_applied {
            self.backend.apply(&projection)?;
            self.last_projection = projection;
        }
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
    use std::os::fd::RawFd;
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{project, Layout, Placement, Rect, TriptychParams, Workarea};
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
        assignments: Vec<(BackendWindowId, WinId)>,
        focus_calls: usize,
        close_calls: usize,
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
                assignments: Vec::new(),
                focus_calls: 0,
                close_calls: 0,
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
            self.assignments.push((backend_id.clone(), win));
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

        fn close(&mut self, _win: WinId) -> BackendResult<()> {
            self.close_calls += 1;
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
            session.backend.assignments,
            [
                (BackendWindowId("r1".to_owned()), WinId(0)),
                (BackendWindowId("r2".to_owned()), WinId(1))
            ]
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
            .update_ledger(|ledger| ledger.switch_orbit(OrbitId::from_human(1).unwrap()))
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

        session
            .update_ledger(|ledger| ledger.set_layout(Layout::Mono))
            .unwrap();
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
        assert_eq!(session.backend.close_calls, 0);
        assert_eq!(session.backend.next_event_calls, 0);
    }

    #[test]
    fn unsupported_apply_rolls_back_and_emits_no_state() {
        let mut backend = FakeBackend::new();
        backend.capabilities.exact_geometry = false;
        backend.capabilities.unsupported = vec!["exact-geometry".to_owned()];
        let mut session = Session::connect(backend).unwrap();
        let ledger = session.ledger().clone();
        let state = session.state().clone();
        let projection = session.last_projection().to_vec();
        session.backend.fail_next_apply = Some(BackendError::Unsupported {
            capability: "exact-geometry".to_owned(),
        });

        let error = session
            .update_ledger(|ledger| ledger.summon(WinId(99), ledger.active()))
            .unwrap_err();

        assert_eq!(
            error,
            BackendError::Unsupported {
                capability: "exact-geometry".to_owned()
            }
        );
        assert_eq!(session.ledger(), &ledger);
        assert_eq!(session.state(), &state);
        assert_eq!(session.last_projection(), projection);
        assert_eq!(session.backend.apply_attempts.len(), 1);
        assert!(session.backend.successful_frames.is_empty());
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
    fn focus_event_is_deferred_without_becoming_a_backend_failure() {
        let mut session = Session::connect(FakeBackend::new()).unwrap();
        let event = BackendEvent::FocusChanged(None);

        let update = session.handle_backend_event(event.clone()).unwrap();

        assert_eq!(update.deferred, Some(event));
        assert!(!update.projection_applied);
        assert!(update.state.is_none());
    }
}
