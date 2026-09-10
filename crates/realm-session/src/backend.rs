//! Compositor-independent window-management contract.

use std::os::fd::RawFd;
use std::time::Instant;

use realm_core::ipc::Capabilities;
use realm_core::layout::{Placement, Rect, Workarea};
use realm_core::WinId;

/// Result returned by compositor backend operations.
pub type BackendResult<T> = std::result::Result<T, BackendError>;

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
}

/// Something the compositor did that the authoritative ledger must reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    /// A new window became manageable.
    WindowOpened {
        /// Realm's never-reused window identifier.
        win: WinId,
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

    /// Apply the complete visible projection.
    ///
    /// Implementations are idempotent: submitting identical placements twice
    /// produces no visible change and no second frame.
    fn apply(&mut self, placements: &[Placement]) -> BackendResult<()>;

    /// Give a window keyboard focus.
    fn focus(&mut self, win: WinId) -> BackendResult<()>;

    /// Ask a window to close politely.
    fn close(&mut self, win: WinId) -> BackendResult<()>;

    /// Return the current projection workarea.
    fn workarea(&self) -> Workarea;

    /// Return the readable descriptor used by the session poll set.
    fn event_fd(&self) -> RawFd;

    /// Wait for the next compositor event, bounded by an optional deadline.
    fn next_event(&mut self, deadline: Option<Instant>) -> BackendResult<Option<BackendEvent>>;
}

#[cfg(test)]
mod tests {
    use std::os::fd::RawFd;
    use std::time::Instant;

    use realm_core::ipc::Capabilities;
    use realm_core::layout::{Placement, Workarea};
    use realm_core::WinId;

    use super::{BackendError, BackendEvent, BackendResult, WmBackend};

    struct ContractBackend;

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

        fn event_fd(&self) -> RawFd {
            7
        }

        fn next_event(
            &mut self,
            _deadline: Option<Instant>,
        ) -> BackendResult<Option<BackendEvent>> {
            Ok(Some(BackendEvent::Disconnected))
        }
    }

    #[test]
    fn trait_exposes_the_accepted_backend_contract() {
        let mut backend: Box<dyn WmBackend> = Box::new(ContractBackend);

        assert_eq!(backend.name(), "contract");
        assert_eq!(backend.event_fd(), 7);
        assert_eq!(backend.workarea().tiles.h, 1022);
        assert_eq!(
            backend.next_event(None).unwrap(),
            Some(BackendEvent::Disconnected)
        );
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
    }
}
