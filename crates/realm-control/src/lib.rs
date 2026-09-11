#![forbid(unsafe_code)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(not(target_os = "linux"))]
compile_error!("realm-control requires Linux");

#[cfg(target_os = "linux")]
mod endpoint;
#[cfg(target_os = "linux")]
mod error;
#[allow(dead_code)]
#[cfg(target_os = "linux")]
mod protocol;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod server;
#[cfg(target_os = "linux")]
mod sys;

#[cfg(target_os = "linux")]
pub use endpoint::{ActiveControlListener, BoundControlEndpoint, SocketEndpoint};
#[cfg(target_os = "linux")]
pub use error::{ControlError, IpcPathError};
#[cfg(target_os = "linux")]
pub use runtime::{
    production_runtime_dir, test_runtime_dir, RealmDir, RuntimeDir, RuntimeDirResolver,
};
#[cfg(target_os = "linux")]
pub use server::{
    ConnectionId, ControlAction, ControlServer, ControlToken, PollInterest, ReadyEvent,
};
#[cfg(all(test, target_os = "linux"))]
pub use server::{TestPeerCredential, TestReceive, TestSend};

#[cfg(all(test, target_os = "linux"))]
mod tests;
