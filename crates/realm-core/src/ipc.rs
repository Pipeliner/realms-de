//! The realm control protocol.
//!
//! One newline-delimited JSON stream over a unix socket. Deliberately boring:
//! `realmctl` shells out to it from scripts, the bar subscribes to it, and a
//! human can drive the whole desktop with `socat`. Newline framing means a
//! partially written frame can never be mistaken for a complete one.

use serde::{Deserialize, Serialize};

use crate::layout::Layout;
use crate::ledger::{Dir, WinId};
use crate::state::RealmState;

/// Protocol version. Bumped on any breaking change; clients refuse a mismatch
/// rather than misinterpreting fields.
pub const PROTOCOL_VERSION: u32 = 1;

/// Window-manager features a backend can honour exactly.
///
/// Capability names in [`Self::unsupported`] are stable user-facing identifiers
/// shared by the session health response and `realmctl doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Whether the rendered rectangle exactly matches the projected rectangle.
    pub exact_geometry: bool,
    /// Whether the compositor draws Realm's window borders.
    pub server_side_borders: bool,
    /// Whether windows can remain managed while hidden and later be shown.
    pub hide_show: bool,
    /// Whether Realm can control rendering order directly.
    pub explicit_ordering: bool,
    /// Whether fullscreen can be requested and exited explicitly.
    pub fullscreen: bool,
    /// Stable names of Realm behaviours this backend cannot honour.
    pub unsupported: Vec<String>,
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
    /// Retired theme-reload request retained in the current wire enum.
    ///
    /// Supported theme apply does not send this request and does not assign it
    /// notify-only semantics. It publishes a sealed generation in the CLI
    /// process for future launches and never reloads on pointer switch. No wire
    /// compatibility behavior is promised for this legacy variant.
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
    State(Box<RealmState>),
    /// Window order for one or more orbits.
    Ledger(Vec<OrbitLedger>),
    /// Command refused.
    Error {
        /// Human-readable reason.
        message: String,
    },
}

/// Window order for a single orbit, as reported by `realmctl ledger show`.
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
    State(Box<RealmState>),
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
    fn unknown_frames_are_an_error_not_a_panic() {
        assert!(decode::<Request>("{\"cmd\":\"detonate\"}").is_err());
        assert!(decode::<Request>("not json").is_err());
    }

    #[test]
    fn backend_capabilities_are_an_owned_wire_type() {
        let capabilities = Capabilities {
            exact_geometry: true,
            server_side_borders: true,
            hide_show: true,
            explicit_ordering: true,
            fullscreen: true,
            unsupported: vec!["unclipped-dimension-quantisation".to_owned()],
        };

        let frame = encode(&capabilities).unwrap();
        let decoded: Capabilities = decode(&frame).unwrap();
        assert_eq!(decoded, capabilities);
    }
}
