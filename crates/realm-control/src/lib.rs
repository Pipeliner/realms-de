#![forbid(unsafe_code)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(not(target_os = "linux"))]
compile_error!("realm-control requires Linux");

#[cfg(target_os = "linux")]
mod endpoint;
#[cfg(target_os = "linux")]
mod error;
#[cfg(target_os = "linux")]
mod runtime;
#[cfg(target_os = "linux")]
mod sys;

#[cfg(target_os = "linux")]
pub use endpoint::SocketEndpoint;
#[cfg(target_os = "linux")]
pub use error::IpcPathError;
#[cfg(target_os = "linux")]
pub use runtime::{
    production_runtime_dir, test_runtime_dir, RealmDir, RuntimeDir, RuntimeDirResolver,
};

#[cfg(all(test, target_os = "linux"))]
mod tests;
