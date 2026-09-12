//! Compositor-independent contracts for Realm's future session daemon.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backend;
pub mod persistence;
pub mod session;
pub mod snapshot;
pub mod timers;
pub mod turn;
pub mod worker;
