use std::os::fd::BorrowedFd;

use realm_session::backend::{BackendPollInterest, WmBackend};
use realm_session::session::Session;

fn registration_seam<B: WmBackend>(session: &Session<B>) {
    let _: BorrowedFd<'_> = session.backend_event_fd();
    let _: BackendPollInterest = session.backend_poll_interest();
}

fn main() {}
