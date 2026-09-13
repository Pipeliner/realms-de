//! Test-only client for the NixOS live-session VM.
//!
//! This binary is feature-gated and installed only into the VM fixture. It
//! exercises the production `realm_control::Client`; it is not a second wire
//! implementation or a supported CLI surface.

use std::error::Error;
use std::io;

use realm_control::production_runtime_dir;
use realm_core::ipc::{self, Request, Response};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let operation = arguments
        .next()
        .ok_or_else(|| invalid("usage: realm-vm-control state | spawn ARG... | quit"))?;
    let remaining = arguments.collect::<Vec<_>>();

    let request = match operation.as_str() {
        "state" if remaining.is_empty() => Request::GetState,
        "spawn" if !remaining.is_empty() => Request::Spawn(remaining),
        "quit" if remaining.is_empty() => Request::Quit,
        _ => return Err(invalid("invalid realm VM control operation").into()),
    };

    let endpoint = production_runtime_dir()?.client_endpoint();
    let mut client = endpoint.connect("realm-nixos-vm")?;
    let response = client.request(request)?;

    match operation.as_str() {
        "state" if matches!(response, Response::State(_)) => {}
        "spawn" | "quit" if response == Response::Ok => {}
        _ => {
            return Err(invalid(format!("unexpected response to {operation}: {response:?}")).into())
        }
    }

    print!("{}", ipc::encode(&response)?);
    Ok(())
}
