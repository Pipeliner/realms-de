use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::time::Instant;

use realm_core::ipc::Capabilities;
use realm_core::WinId;
use realm_session::backend::{
    BackendBindingSpec, BackendContractError, BackendEvent, BackendExitPolicy,
    BackendPolicyResponse, BackendPolicyTurnId, BackendPollInterest, BackendReady, BackendResult,
    BackendSubmission, BackendTicket, BackendWindowId, WmBackend,
};

struct AcceptedBackend(File);

impl WmBackend for AcceptedBackend {
    fn name(&self) -> &str {
        "accepted"
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

fn main() {
    let _backend: Box<dyn WmBackend> =
        Box::new(AcceptedBackend(File::open("/dev/null").unwrap()));
}
