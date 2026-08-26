//! The ledger: the single ordered record of every window helm manages.
//!
//! There are six orbits (workspaces), each holding a strictly ordered
//! `Vec<WinId>`. Every user action — summon, banish, swap, focus, stow — is a
//! mutation of this list and nothing else. Window *positions* are never stored;
//! they are recomputed by [`crate::layout::project`] whenever the ledger or the
//! workarea changes.
//!
//! Because the ledger is small and cheap to clone, undo is a stack of whole
//! snapshots rather than a fiddly inverse-operation log.

use serde::{Deserialize, Serialize};

use crate::layout::Layout;

/// Number of orbits. Fixed at six — one per rune in `ᚠᚢᚦᚨᚱᚲ`.
pub const ORBIT_COUNT: usize = 6;

/// How many ledger snapshots are retained for undo.
pub const HISTORY_DEPTH: usize = 64;

/// Opaque compositor-assigned window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WinId(pub u64);

impl std::fmt::Display for WinId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "w{}", self.0)
    }
}

/// An orbit index in `0..ORBIT_COUNT`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub struct OrbitId(u8);

impl OrbitId {
    /// Construct from a zero-based index, returning `None` when out of range.
    pub fn new(i: usize) -> Option<Self> {
        (i < ORBIT_COUNT).then_some(Self(i as u8))
    }

    /// Construct from the one-based number the user types (`mod+1`..`mod+6`).
    pub fn from_human(n: usize) -> Option<Self> {
        n.checked_sub(1).and_then(Self::new)
    }

    /// Zero-based index.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// One-based number as shown in keybindings.
    pub fn human(self) -> usize {
        self.0 as usize + 1
    }

    /// The rune that labels this orbit in the bar.
    pub fn rune(self) -> char {
        // U+16A0 block: ᚠ ᚢ ᚦ ᚨ ᚱ ᚲ — the elder futhark's first six.
        const RUNES: [char; ORBIT_COUNT] = ['ᚠ', 'ᚢ', 'ᚦ', 'ᚨ', 'ᚱ', 'ᚲ'];
        RUNES[self.index()]
    }

    /// Every orbit, in order.
    pub fn all() -> impl Iterator<Item = OrbitId> {
        (0..ORBIT_COUNT).map(|i| OrbitId(i as u8))
    }
}

/// Direction for relative focus and swap operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    /// Towards the end of the ledger (`j`, `l`).
    Next,
    /// Towards the start of the ledger (`k`, `h`).
    Prev,
}

/// One workspace: an ordered window list plus its own layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Orbit {
    /// Which orbit this is.
    pub id: OrbitId,
    /// Windows in ledger order. Index 0 is the master slot.
    pub windows: Vec<WinId>,
    /// Index into `windows` of the focused window.
    pub focus: Option<usize>,
    /// Windows temporarily removed from the projection but not from the ledger.
    pub stowed: Vec<WinId>,
    /// Layout applied to this orbit.
    pub layout: Layout,
    /// Window promoted to cover the whole workarea, if any (`mod+f`).
    pub fullscreen: Option<WinId>,
    /// Human-facing name shown by `helm ctl orbit --list`.
    pub name: String,
}

impl Orbit {
    fn new(id: OrbitId) -> Self {
        const NAMES: [&str; ORBIT_COUNT] = [
            "triptych",
            "scriptorium",
            "observatory",
            "forge",
            "athenaeum",
            "crypt",
        ];
        Self {
            id,
            windows: Vec::new(),
            focus: None,
            stowed: Vec::new(),
            layout: Layout::default(),
            fullscreen: None,
            name: NAMES[id.index()].to_owned(),
        }
    }

    /// Windows that the layout should actually place: ledger order minus stowed.
    pub fn visible(&self) -> Vec<WinId> {
        self.windows
            .iter()
            .copied()
            .filter(|w| !self.stowed.contains(w))
            .collect()
    }

    /// The focused window, if the orbit is non-empty.
    pub fn focused(&self) -> Option<WinId> {
        self.focus.and_then(|i| self.windows.get(i).copied())
    }

    /// True when the orbit holds at least one window.
    pub fn occupied(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Clamp `focus` back into range after a mutation.
    fn reseat_focus(&mut self, hint: usize) {
        self.focus = if self.windows.is_empty() {
            None
        } else {
            Some(hint.min(self.windows.len() - 1))
        };
    }
}

/// The complete window-management state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ledger {
    orbits: Vec<Orbit>,
    active: OrbitId,
    #[serde(skip)]
    history: Vec<Snapshot>,
    #[serde(skip)]
    redo: Vec<Snapshot>,
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    orbits: Vec<Orbit>,
    active: OrbitId,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

impl Ledger {
    /// An empty ledger with six empty orbits.
    pub fn new() -> Self {
        Self {
            orbits: OrbitId::all().map(Orbit::new).collect(),
            active: OrbitId::default(),
            history: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Borrow an orbit.
    pub fn orbit(&self, id: OrbitId) -> &Orbit {
        &self.orbits[id.index()]
    }

    /// All orbits in order.
    pub fn orbits(&self) -> &[Orbit] {
        &self.orbits
    }

    /// The orbit currently shown on the output.
    pub fn active(&self) -> OrbitId {
        self.active
    }

    /// Borrow the active orbit.
    pub fn active_orbit(&self) -> &Orbit {
        self.orbit(self.active)
    }

    /// The focused window of the active orbit.
    pub fn focused(&self) -> Option<WinId> {
        self.active_orbit().focused()
    }

    /// Find which orbit holds `win`.
    pub fn orbit_of(&self, win: WinId) -> Option<OrbitId> {
        self.orbits
            .iter()
            .find(|o| o.windows.contains(&win))
            .map(|o| o.id)
    }

    /// Total windows across every orbit.
    pub fn len(&self) -> usize {
        self.orbits.iter().map(|o| o.windows.len()).sum()
    }

    /// True when no orbit holds a window.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    // ---- mutations -------------------------------------------------------
    //
    // Every mutation checkpoints first, so `undo()` restores the state as it
    // was before the *last* user-visible change.

    fn checkpoint(&mut self) {
        self.history.push(Snapshot {
            orbits: self.orbits.clone(),
            active: self.active,
        });
        if self.history.len() > HISTORY_DEPTH {
            self.history.remove(0);
        }
        self.redo.clear();
    }

    /// Enter a new window into an orbit, directly after the focused window.
    ///
    /// Inserting next to focus (rather than appending) is what makes
    /// "open a terminal beside this one" behave the way muscle memory expects.
    pub fn summon(&mut self, win: WinId, orbit: OrbitId) {
        if self.orbit_of(win).is_some() {
            return;
        }
        self.checkpoint();
        let o = &mut self.orbits[orbit.index()];
        let at = o.focus.map(|i| i + 1).unwrap_or(0).min(o.windows.len());
        o.windows.insert(at, win);
        o.focus = Some(at);
    }

    /// Remove a window from wherever it lives.
    pub fn banish(&mut self, win: WinId) -> bool {
        let Some(orbit) = self.orbit_of(win) else {
            return false;
        };
        self.checkpoint();
        let o = &mut self.orbits[orbit.index()];
        let Some(at) = o.windows.iter().position(|w| *w == win) else {
            return false;
        };
        o.windows.remove(at);
        o.stowed.retain(|w| *w != win);
        if o.fullscreen == Some(win) {
            o.fullscreen = None;
        }
        // Focus the window that slid into the vacated slot, else the new last.
        o.reseat_focus(at.saturating_sub(usize::from(at >= o.windows.len().max(1))));
        true
    }

    /// Focus a specific window, switching orbits if needed.
    pub fn focus_window(&mut self, win: WinId) -> bool {
        let Some(orbit) = self.orbit_of(win) else {
            return false;
        };
        self.checkpoint();
        self.active = orbit;
        let o = &mut self.orbits[orbit.index()];
        o.focus = o.windows.iter().position(|w| *w == win);
        true
    }

    /// Move focus along the ledger, wrapping at the ends.
    pub fn focus_step(&mut self, dir: Dir) {
        let o = &mut self.orbits[self.active.index()];
        let n = o.windows.len();
        if n == 0 {
            return;
        }
        self.checkpoint();
        let o = &mut self.orbits[self.active.index()];
        let cur = o.focus.unwrap_or(0);
        o.focus = Some(match dir {
            Dir::Next => (cur + 1) % n,
            Dir::Prev => (cur + n - 1) % n,
        });
    }

    /// Swap the focused window with its neighbour, carrying focus along.
    ///
    /// This is the only way a window changes position in the layout, which is
    /// why the layout can stay a pure function.
    pub fn swap(&mut self, dir: Dir) -> bool {
        let o = &self.orbits[self.active.index()];
        let n = o.windows.len();
        let Some(cur) = o.focus else { return false };
        if n < 2 {
            return false;
        }
        let target = match dir {
            Dir::Next => (cur + 1) % n,
            Dir::Prev => (cur + n - 1) % n,
        };
        self.checkpoint();
        let o = &mut self.orbits[self.active.index()];
        o.windows.swap(cur, target);
        o.focus = Some(target);
        true
    }

    /// Send the focused window to another orbit, keeping it focused there.
    pub fn move_to_orbit(&mut self, target: OrbitId) -> bool {
        let Some(win) = self.focused() else {
            return false;
        };
        if target == self.active {
            return false;
        }
        self.checkpoint();
        let src = &mut self.orbits[self.active.index()];
        let at = src.windows.iter().position(|w| *w == win).unwrap();
        src.windows.remove(at);
        src.stowed.retain(|w| *w != win);
        if src.fullscreen == Some(win) {
            src.fullscreen = None;
        }
        src.reseat_focus(at.saturating_sub(1));
        let dst = &mut self.orbits[target.index()];
        let insert_at = dst.focus.map(|i| i + 1).unwrap_or(0).min(dst.windows.len());
        dst.windows.insert(insert_at, win);
        dst.focus = Some(insert_at);
        true
    }

    /// Show a different orbit.
    pub fn switch_orbit(&mut self, target: OrbitId) {
        if target == self.active {
            return;
        }
        self.checkpoint();
        self.active = target;
    }

    /// Toggle whether the focused window participates in the layout.
    pub fn toggle_stow(&mut self) -> bool {
        let Some(win) = self.focused() else {
            return false;
        };
        self.checkpoint();
        let o = &mut self.orbits[self.active.index()];
        if let Some(at) = o.stowed.iter().position(|w| *w == win) {
            o.stowed.remove(at);
        } else {
            o.stowed.push(win);
        }
        true
    }

    /// Toggle fullscreen for the focused window.
    pub fn toggle_fullscreen(&mut self) -> bool {
        let Some(win) = self.focused() else {
            return false;
        };
        self.checkpoint();
        let o = &mut self.orbits[self.active.index()];
        o.fullscreen = if o.fullscreen == Some(win) {
            None
        } else {
            Some(win)
        };
        true
    }

    /// Replace the active orbit's layout.
    pub fn set_layout(&mut self, layout: Layout) {
        if self.orbits[self.active.index()].layout == layout {
            return;
        }
        self.checkpoint();
        self.orbits[self.active.index()].layout = layout;
    }

    /// Restore the ledger as it was before the last mutation.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.history.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            orbits: self.orbits.clone(),
            active: self.active,
        });
        self.orbits = prev.orbits;
        self.active = prev.active;
        true
    }

    /// Re-apply the most recently undone mutation.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.history.push(Snapshot {
            orbits: self.orbits.clone(),
            active: self.active,
        });
        self.orbits = next.orbits;
        self.active = next.active;
        true
    }

    /// How many undo steps are available.
    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger_with(n: u64) -> Ledger {
        let mut l = Ledger::new();
        for i in 0..n {
            l.summon(WinId(i), OrbitId::default());
        }
        l
    }

    #[test]
    fn runes_cover_every_orbit() {
        let runes: Vec<char> = OrbitId::all().map(|o| o.rune()).collect();
        assert_eq!(runes, vec!['ᚠ', 'ᚢ', 'ᚦ', 'ᚨ', 'ᚱ', 'ᚲ']);
        assert_eq!(OrbitId::from_human(1), OrbitId::new(0));
        assert_eq!(OrbitId::from_human(7), None);
        assert_eq!(OrbitId::from_human(0), None);
    }

    #[test]
    fn summon_inserts_after_focus_not_at_the_end() {
        let mut l = Ledger::new();
        l.summon(WinId(1), OrbitId::default());
        l.summon(WinId(2), OrbitId::default());
        // focus is on w2 (index 1); w3 must land at index 2.
        l.focus_step(Dir::Prev); // focus w1
        l.summon(WinId(3), OrbitId::default());
        assert_eq!(l.active_orbit().windows, vec![WinId(1), WinId(3), WinId(2)]);
        assert_eq!(l.focused(), Some(WinId(3)));
    }

    #[test]
    fn summoning_a_known_window_twice_is_ignored() {
        let mut l = ledger_with(2);
        let before = l.active_orbit().windows.clone();
        l.summon(WinId(0), OrbitId::new(3).unwrap());
        assert_eq!(l.active_orbit().windows, before);
        assert!(l.orbit(OrbitId::new(3).unwrap()).windows.is_empty());
    }

    #[test]
    fn focus_wraps_in_both_directions() {
        let mut l = ledger_with(3);
        l.focus_step(Dir::Next);
        assert_eq!(l.focused(), Some(WinId(0)));
        l.focus_step(Dir::Prev);
        assert_eq!(l.focused(), Some(WinId(2)));
    }

    #[test]
    fn focus_step_on_empty_orbit_does_not_panic() {
        let mut l = Ledger::new();
        l.focus_step(Dir::Next);
        assert_eq!(l.focused(), None);
        assert!(!l.swap(Dir::Next));
        assert!(!l.toggle_stow());
        assert!(!l.toggle_fullscreen());
    }

    #[test]
    fn swap_carries_focus_with_the_window() {
        let mut l = ledger_with(3); // focus on w2
        assert!(l.swap(Dir::Prev));
        assert_eq!(l.active_orbit().windows, vec![WinId(0), WinId(2), WinId(1)]);
        assert_eq!(l.focused(), Some(WinId(2)));
    }

    #[test]
    fn banish_keeps_focus_inside_the_ledger() {
        let mut l = ledger_with(3);
        assert!(l.banish(WinId(2)));
        assert_eq!(l.active_orbit().windows, vec![WinId(0), WinId(1)]);
        let f = l.focused().unwrap();
        assert!(l.active_orbit().windows.contains(&f));
        assert!(l.banish(WinId(1)));
        assert!(l.banish(WinId(0)));
        assert_eq!(l.focused(), None);
        assert!(!l.banish(WinId(0)));
    }

    #[test]
    fn move_to_orbit_transfers_and_refocuses() {
        let mut l = ledger_with(2);
        let target = OrbitId::new(2).unwrap();
        assert!(l.move_to_orbit(target));
        assert_eq!(l.orbit(target).windows, vec![WinId(1)]);
        assert_eq!(l.orbit(target).focus, Some(0));
        assert_eq!(l.active_orbit().windows, vec![WinId(0)]);
        assert!(!l.move_to_orbit(l.active()));
    }

    #[test]
    fn stow_removes_from_projection_but_not_from_the_ledger() {
        let mut l = ledger_with(3);
        assert!(l.toggle_stow());
        let o = l.active_orbit();
        assert_eq!(o.windows.len(), 3);
        assert_eq!(o.visible().len(), 2);
        assert!(!o.visible().contains(&WinId(2)));
        l.toggle_stow();
        assert_eq!(l.active_orbit().visible().len(), 3);
    }

    #[test]
    fn undo_restores_the_exact_previous_ledger() {
        let mut l = ledger_with(3);
        let before = l.active_orbit().clone();
        l.swap(Dir::Prev);
        assert_ne!(l.active_orbit().windows, before.windows);
        assert!(l.undo());
        assert_eq!(l.active_orbit(), &before);
        assert!(l.redo());
        assert_ne!(l.active_orbit().windows, before.windows);
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut l = Ledger::new();
        for i in 0..(HISTORY_DEPTH as u64 * 3) {
            l.summon(WinId(i), OrbitId::default());
        }
        assert_eq!(l.undo_depth(), HISTORY_DEPTH);
    }

    #[test]
    fn a_new_mutation_clears_the_redo_stack() {
        let mut l = ledger_with(3);
        l.swap(Dir::Prev);
        l.undo();
        l.summon(WinId(99), OrbitId::default());
        assert!(!l.redo());
    }

    #[test]
    fn no_op_mutations_do_not_consume_undo_depth() {
        let mut l = ledger_with(2);
        let depth = l.undo_depth();
        l.switch_orbit(l.active());
        l.set_layout(l.active_orbit().layout);
        assert_eq!(l.undo_depth(), depth);
    }
}
