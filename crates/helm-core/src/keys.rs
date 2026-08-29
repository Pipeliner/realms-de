//! Modes, chords and the grimoire.
//!
//! helm binds everything under one modifier. The bar shows the current mode and
//! echoes a pending prefix, so a half-typed chord is never a mystery — the
//! single most common complaint about chorded keyboard-driven desktops.

use serde::{Deserialize, Serialize};

use crate::layout::Layout;
use crate::ledger::{Dir, OrbitId};

/// Input mode. The bar renders this as a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Normal navigation.
    #[default]
    Nav,
    /// Arrow/hjkl resize the focused window until `esc`.
    Resize,
    /// A window is being moved between orbits.
    Move,
}

impl Mode {
    /// The badge text, e.g. `⌨ NAV`.
    pub fn badge(self) -> &'static str {
        match self {
            Mode::Nav => "NAV",
            Mode::Resize => "RESIZE",
            Mode::Move => "MOVE",
        }
    }
}

/// Everything a keybinding can ask helm to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "action", content = "arg")]
pub enum Action {
    /// Launch a program by argv.
    Spawn(Vec<String>),
    /// Open the hecate launcher.
    Launcher,
    /// Move focus along the ledger.
    Focus(Dir),
    /// Swap the focused window with a neighbour.
    Swap(Dir),
    /// Show an orbit.
    Orbit(usize),
    /// Send the focused window to an orbit.
    MoveToOrbit(usize),
    /// Toggle stow on the focused window.
    Stow,
    /// Set the active orbit's layout.
    SetLayout(Layout),
    /// Toggle fullscreen.
    Fullscreen,
    /// Enter a mode.
    EnterMode(Mode),
    /// Close the focused window.
    Banish,
    /// Restore the previous ledger.
    Undo,
    /// Toggle the which-key strip.
    ToggleWhichKey,
    /// Show the full keybinding sheet.
    Grimoire,
    /// Legacy action name for publishing a theme generation for future launches.
    /// It does not hot-reload clients on pointer switch.
    ReloadTheme,
    /// End the session.
    Quit,
}

/// One binding: a key under the modifier, an action, and how it is advertised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// Key name as typed, e.g. `d`, `j`, `Return`, `1`.
    pub key: String,
    /// What the bar's which-key strip shows in place of the raw key.
    pub hint_key: String,
    /// One-word label.
    pub label: String,
    /// What it does.
    pub action: Action,
    /// Which mode the binding is live in.
    pub mode: Mode,
    /// Whether it appears in the 26px which-key strip (the full set always
    /// appears in the grimoire).
    pub in_strip: bool,
}

/// The full keymap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    /// Modifier name shown as `⊞ mod`.
    pub modifier: String,
    /// All bindings.
    pub bindings: Vec<Binding>,
}

fn b(key: &str, hint: &str, label: &str, action: Action, in_strip: bool) -> Binding {
    Binding {
        key: key.into(),
        hint_key: hint.into(),
        label: label.into(),
        action,
        mode: Mode::Nav,
        in_strip,
    }
}

impl Default for Keymap {
    /// The keymap the reference which-key strip advertises, in strip order.
    fn default() -> Self {
        let mut bindings = vec![
            b(
                "Return",
                "↵",
                "thoth",
                Action::Spawn(vec!["helm-term".into()]),
                true,
            ),
            b("d", "d", "hecate", Action::Launcher, true),
            b(
                "b",
                "b",
                "hermes",
                Action::Spawn(vec!["helm-browser".into()]),
                true,
            ),
            b("j", "j/k", "focus", Action::Focus(Dir::Next), true),
            b("k", "", "focus", Action::Focus(Dir::Prev), false),
            b("h", "h/l", "swap", Action::Swap(Dir::Prev), true),
            b("l", "", "swap", Action::Swap(Dir::Next), false),
        ];
        for n in 1..=crate::ledger::ORBIT_COUNT {
            bindings.push(b(
                &n.to_string(),
                if n == 1 { "1-6" } else { "" },
                "orbit",
                Action::Orbit(n),
                n == 1,
            ));
        }
        bindings.extend([
            b("s", "s", "stow", Action::Stow, true),
            b("m", "m", "mono", Action::SetLayout(Layout::Mono), true),
            b("f", "f", "full", Action::Fullscreen, true),
            b("r", "r", "resize", Action::EnterMode(Mode::Resize), true),
            b("q", "q", "banish", Action::Banish, true),
            b(
                "t",
                "t",
                "triptych",
                Action::SetLayout(Layout::Triptych),
                false,
            ),
            b("u", "u", "undo", Action::Undo, false),
            b("w", "w", "which-key", Action::ToggleWhichKey, false),
            b("question", "?", "grimoire", Action::Grimoire, false),
            b("p", "p", "theme", Action::ReloadTheme, false),
            b("Escape", "esc", "nav", Action::EnterMode(Mode::Nav), false),
        ]);
        Self {
            modifier: "mod".into(),
            bindings,
        }
    }
}

impl Keymap {
    /// Bindings shown in the which-key strip, in order.
    pub fn strip(&self) -> impl Iterator<Item = &Binding> {
        self.bindings.iter().filter(|b| b.in_strip)
    }

    /// Resolve a key press in a mode.
    pub fn resolve(&self, key: &str, mode: Mode) -> Option<&Binding> {
        self.bindings
            .iter()
            .find(|b| b.key == key && b.mode == mode)
    }

    /// Validate: no key bound twice in the same mode, and every orbit reachable.
    pub fn conflicts(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, a) in self.bindings.iter().enumerate() {
            for c in &self.bindings[i + 1..] {
                if a.key == c.key && a.mode == c.mode {
                    out.push(format!("{} bound twice in {:?} mode", a.key, a.mode));
                }
            }
        }
        for o in OrbitId::all() {
            let n = o.human();
            if !self
                .bindings
                .iter()
                .any(|b| matches!(&b.action, Action::Orbit(x) if *x == n))
            {
                out.push(format!("orbit {n} is unreachable"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keymap_has_no_conflicts() {
        assert!(Keymap::default().conflicts().is_empty());
    }

    #[test]
    fn strip_matches_the_reference_which_key_row() {
        let km = Keymap::default();
        let row: Vec<String> = km
            .strip()
            .map(|b| format!("{} {}", b.hint_key, b.label))
            .collect();
        assert_eq!(
            row,
            vec![
                "↵ thoth",
                "d hecate",
                "b hermes",
                "j/k focus",
                "h/l swap",
                "1-6 orbit",
                "s stow",
                "m mono",
                "f full",
                "r resize",
                "q banish",
            ]
        );
    }

    #[test]
    fn resolve_respects_mode() {
        let km = Keymap::default();
        assert!(km.resolve("d", Mode::Nav).is_some());
        assert!(km.resolve("d", Mode::Resize).is_none());
        assert!(km.resolve("nonexistent", Mode::Nav).is_none());
    }

    #[test]
    fn duplicate_binding_is_reported() {
        let mut km = Keymap::default();
        km.bindings
            .push(b("d", "d", "dupe", Action::Grimoire, true));
        assert!(!km.conflicts().is_empty());
    }

    #[test]
    fn mode_badges_are_stable() {
        assert_eq!(Mode::Nav.badge(), "NAV");
        assert_eq!(Mode::Resize.badge(), "RESIZE");
    }
}
