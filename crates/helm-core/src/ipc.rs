//! The helm control protocol.
//!
//! One newline-delimited JSON stream over a unix socket. Deliberately boring:
//! `helm ctl` shells out to it from scripts, the bar subscribes to it, and a
//! human can drive the whole desktop with `socat`. Newline framing means a
//! partially written frame can never be mistaken for a complete one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::layout::Layout;
use crate::ledger::{Dir, WinId};
use crate::state::HelmState;

/// Protocol version. Bumped on any breaking change; clients refuse a mismatch
/// rather than misinterpreting fields.
pub const PROTOCOL_VERSION: u32 = 1;

/// Socket path: `$XDG_RUNTIME_DIR/helm/ctl.sock`, or a `/tmp` fallback.
///
/// The fallback is per-uid so two users on one machine never collide.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("HELM_SOCKET") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/helm-{}", uid())));
    base.join("helm").join("ctl.sock")
}

fn uid() -> u32 {
    // Avoid a libc dependency in a crate that is otherwise pure.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|l| l.split_whitespace().next().map(str::to_owned))
        })
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000)
}

/// A command sent to the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "cmd", content = "arg")]
pub enum Request {
    /// Handshake; always answer, even on version mismatch.
    Hello {
        /// Client's protocol version.
        version: u32,
        /// Client name, for logs.
        client: String,
    },
    /// Ask for the current state once.
    GetState,
    /// Stream a [`Event::State`] on every change until the connection closes.
    Subscribe,
    /// Show an orbit (one-based).
    SwitchOrbit(usize),
    /// Send the focused window to an orbit (one-based).
    MoveToOrbit(usize),
    /// Move focus.
    Focus(Dir),
    /// Swap the focused window.
    Swap(Dir),
    /// Close the focused window.
    Banish,
    /// Toggle stow.
    Stow,
    /// Toggle fullscreen.
    Fullscreen,
    /// Change the active orbit's layout.
    SetLayout(Layout),
    /// Restore the previous ledger.
    Undo,
    /// Re-read `palette.toml`, re-render templates, hot-reload clients.
    ReloadTheme,
    /// Launch a program.
    Spawn(Vec<String>),
    /// Report the ledger of an orbit, or of all orbits when `None`.
    ShowLedger(Option<usize>),
    /// End the session.
    Quit,
}

/// A reply to a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "reply", content = "data")]
pub enum Response {
    /// Handshake accepted.
    Hello {
        /// Session's protocol version.
        version: u32,
        /// Session build version.
        session: String,
    },
    /// Command applied.
    Ok,
    /// Full state snapshot.
    State(Box<HelmState>),
    /// Window order for one or more orbits.
    Ledger(Vec<OrbitLedger>),
    /// Command refused.
    Error {
        /// Human-readable reason.
        message: String,
    },
}

/// Window order for a single orbit, as reported by `helm ctl ledger show`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbitLedger {
    /// One-based orbit number.
    pub orbit: usize,
    /// The orbit's rune.
    pub rune: String,
    /// The orbit's name.
    pub name: String,
    /// Windows in ledger order.
    pub windows: Vec<LedgerEntry>,
}

/// One window as listed in a ledger dump.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Compositor handle.
    pub id: WinId,
    /// Application id, e.g. `odin`.
    pub app_id: String,
    /// Window title.
    pub title: String,
    /// Whether it holds focus.
    pub focused: bool,
    /// Whether it is stowed out of the projection.
    pub stowed: bool,
}

/// An unsolicited message pushed to subscribers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "event", content = "data")]
pub enum Event {
    /// The state changed. Sent coalesced, never more than once per change.
    State(Box<HelmState>),
    /// The session is shutting down; clients should exit cleanly.
    Shutdown,
}

/// Encode a value as one protocol frame (JSON plus a newline).
pub fn encode<T: Serialize>(value: &T) -> crate::Result<String> {
    let mut s = serde_json::to_string(value)?;
    s.push('\n');
    Ok(s)
}

/// Decode one frame.
pub fn decode<T: serde::de::DeserializeOwned>(line: &str) -> crate::Result<T> {
    Ok(serde_json::from_str(line.trim_end_matches('\n'))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_through_a_frame() {
        let cases = vec![
            Request::Hello {
                version: PROTOCOL_VERSION,
                client: "bar".into(),
            },
            Request::SwitchOrbit(3),
            Request::Focus(Dir::Next),
            Request::SetLayout(Layout::Mono),
            Request::Spawn(vec!["foot".into(), "-e".into(), "yazi".into()]),
            Request::ShowLedger(Some(1)),
            Request::ShowLedger(None),
        ];
        for c in cases {
            let frame = encode(&c).unwrap();
            assert!(frame.ends_with('\n'));
            assert_eq!(frame.matches('\n').count(), 1, "frames must be single-line");
            assert_eq!(decode::<Request>(&frame).unwrap(), c);
        }
    }

    #[test]
    fn responses_round_trip() {
        let r = Response::Ledger(vec![OrbitLedger {
            orbit: 1,
            rune: "ᚠ".into(),
            name: "triptych".into(),
            windows: vec![LedgerEntry {
                id: WinId(7),
                app_id: "odin".into(),
                title: "odin — harness".into(),
                focused: true,
                stowed: false,
            }],
        }]);
        assert_eq!(decode::<Response>(&encode(&r).unwrap()).unwrap(), r);
    }

    #[test]
    fn socket_path_honours_the_environment() {
        let prev = std::env::var_os("HELM_SOCKET");
        std::env::set_var("HELM_SOCKET", "/run/custom/helm.sock");
        assert_eq!(socket_path(), PathBuf::from("/run/custom/helm.sock"));
        match prev {
            Some(v) => std::env::set_var("HELM_SOCKET", v),
            None => std::env::remove_var("HELM_SOCKET"),
        }
    }

    #[test]
    fn unknown_frames_are_an_error_not_a_panic() {
        assert!(decode::<Request>("{\"cmd\":\"detonate\"}").is_err());
        assert!(decode::<Request>("not json").is_err());
    }
}
