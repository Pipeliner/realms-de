//! The state snapshot the session broadcasts and the bar renders.
//!
//! Deliberately a plain data struct with no behaviour: the bar redraws when and
//! only when this value changes, which is what "render on state change, never
//! on a timer" means in practice.

use serde::{Deserialize, Serialize};

use crate::keys::Mode;
use crate::layout::Layout;

/// How an orbit is drawn in the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OrbitDisplay {
    /// Currently shown.
    Active,
    /// Holds windows but is not shown.
    Occupied,
    /// Holds nothing.
    Empty,
}

/// One orbit's bar cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrbitCell {
    /// One-based orbit number.
    pub number: usize,
    /// The rune to draw.
    pub rune: String,
    /// How to draw it.
    pub display: OrbitDisplay,
    /// Window count, for tooltips and `helm ctl orbit --list`.
    pub windows: usize,
}

/// A right-hand bar module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    /// Stable identifier, e.g. `cpu`, `battery`.
    pub id: String,
    /// Rendered text, e.g. `cpu 31%`.
    pub text: String,
    /// Accent name from the palette, or `None` for the default foreground.
    pub accent: Option<String>,
    /// Set when the module wants attention (low battery, thermal throttle).
    pub urgent: bool,
}

/// Everything the bar needs to draw one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelmState {
    /// Monotonic counter; the bar skips redrawing when it has not moved.
    pub revision: u64,
    /// Orbit cells, always six of them.
    pub orbits: Vec<OrbitCell>,
    /// Active orbit's layout.
    pub layout: Layout,
    /// Input mode.
    pub mode: Mode,
    /// Focused window title, centred in the bar.
    pub focused_title: String,
    /// Pending chord echo, e.g. `mod+ ▸ awaiting chord…`; empty when idle.
    pub chord_echo: String,
    /// Whether the which-key strip is visible.
    pub whichkey: bool,
    /// Right-hand modules in draw order.
    pub modules: Vec<Module>,
}

impl Default for HelmState {
    fn default() -> Self {
        Self {
            revision: 0,
            orbits: crate::ledger::OrbitId::all()
                .map(|o| OrbitCell {
                    number: o.human(),
                    rune: o.rune().to_string(),
                    display: if o.index() == 0 {
                        OrbitDisplay::Active
                    } else {
                        OrbitDisplay::Empty
                    },
                    windows: 0,
                })
                .collect(),
            layout: Layout::default(),
            mode: Mode::default(),
            focused_title: String::new(),
            chord_echo: String::new(),
            whichkey: true,
            modules: Vec::new(),
        }
    }
}

impl HelmState {
    /// True when `other` would draw identically, ignoring the revision counter.
    ///
    /// The bar uses this to drop redundant frames: a module that recomputes to
    /// the same string must not cost a redraw.
    pub fn renders_same_as(&self, other: &HelmState) -> bool {
        self.orbits == other.orbits
            && self.layout == other.layout
            && self.mode == other.mode
            && self.focused_title == other.focused_title
            && self.chord_echo == other.chord_echo
            && self.whichkey == other.whichkey
            && self.modules == other.modules
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_one_cell_per_orbit_with_the_first_active() {
        let s = HelmState::default();
        assert_eq!(s.orbits.len(), crate::ledger::ORBIT_COUNT);
        assert_eq!(s.orbits[0].display, OrbitDisplay::Active);
        assert_eq!(s.orbits[0].rune, "ᚠ");
        assert_eq!(s.orbits[5].rune, "ᚲ");
    }

    #[test]
    fn revision_alone_does_not_force_a_redraw() {
        let a = HelmState::default();
        let mut b = a.clone();
        b.revision += 9;
        assert!(a.renders_same_as(&b));
        b.mode = Mode::Resize;
        assert!(!a.renders_same_as(&b));
    }

    #[test]
    fn state_round_trips_through_json() {
        let s = HelmState::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<HelmState>(&json).unwrap(), s);
    }
}
