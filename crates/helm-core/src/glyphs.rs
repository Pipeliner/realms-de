//! The glyph inventory and the contract for verifying it.
//!
//! helm draws runes, planetary symbols, braille sparklines and one Egyptian
//! hieroglyph. On a machine without the right fonts these become tofu boxes and
//! the desktop looks broken before the user has typed anything — a failure mode
//! every glyph-heavy TUI/DE eventually ships by accident.
//!
//! So the inventory is data, not scattered string literals: a renderer resolves
//! the font stack once at startup, reports coverage as a [`Probe`], and helm
//! substitutes documented fallbacks for anything missing rather than drawing
//! tofu. `helm ctl doctor` runs the same check from the command line.

use serde::{Deserialize, Serialize};

/// Where a glyph is used, so a coverage failure can be reported usefully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Surface {
    /// Top bar.
    Bar,
    /// Which-key strip.
    WhichKey,
    /// Window headers.
    Header,
    /// Monitor meters and sparklines.
    Instruments,
    /// Shell prompt.
    Prompt,
    /// Launcher and portal.
    Overlay,
}

/// One glyph helm intends to draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Glyph {
    /// The character itself.
    pub ch: char,
    /// What it means, for the doctor's report.
    pub name: &'static str,
    /// Where it appears.
    pub surface: Surface,
    /// What to draw when the font stack cannot render `ch`. `None` means the
    /// glyph is essential and its absence is a hard startup failure.
    pub fallback: Option<char>,
}

/// Every glyph helm draws, with its documented fallback.
pub fn inventory() -> Vec<Glyph> {
    use Surface::*;
    let g = |ch, name, surface, fallback| Glyph {
        ch,
        name,
        surface,
        fallback,
    };
    let mut v = vec![
        g('✦', "logo", Bar, Some('*')),
        g('⌗', "layout indicator", Bar, Some('#')),
        g('⌨', "mode badge", Bar, Some(':')),
        g('▸', "chord echo", Bar, Some('>')),
        g('☾', "moon, clock", Bar, Some(')')),
        g('⚡', "battery", Bar, Some('^')),
        g('♪', "volume", Bar, Some('~')),
        g('⊞', "modifier", WhichKey, Some('+')),
        g('↵', "return", WhichKey, Some('<')),
        g('⇥', "tab", Overlay, Some('>')),
        g('◈', "stow control", Header, Some('o')),
        g('─', "minimise control", Header, Some('-')),
        g('✕', "close control", Header, Some('x')),
        g('▰', "meter full", Instruments, Some('#')),
        g('▱', "meter empty", Instruments, Some('.')),
        g('⚷', "hecate", Overlay, Some('*')),
        g('☍', "charon", Overlay, Some('=')),
        g('ᛟ', "odin", Header, Some('O')),
        g('☽', "thoth", Header, Some('D')),
        g('⚚', "hermes", Header, Some('I')),
        g('◉', "horus", Header, Some('@')),
        g('✶', "urania", Header, Some('*')),
        // Essential: without runes the orbit indicator is meaningless, and
        // there is no sensible one-character substitute that still reads as a
        // workspace. helm falls back to digits and says so loudly.
        g('ᚠ', "orbit 1", Bar, Some('1')),
        g('ᚢ', "orbit 2", Bar, Some('2')),
        g('ᚦ', "orbit 3", Bar, Some('3')),
        g('ᚨ', "orbit 4", Bar, Some('4')),
        g('ᚱ', "orbit 5", Bar, Some('5')),
        g('ᚲ', "orbit 6", Bar, Some('6')),
        // The design brief singles this one out: verify it renders or use `~`.
        g('𓂃', "prompt sigil", Prompt, Some('~')),
    ];
    for (i, ch) in ['⡀', '⣀', '⣄', '⣤', '⣦', '⣶', '⣷', '⣿']
        .into_iter()
        .enumerate()
    {
        v.push(Glyph {
            ch,
            name: "sparkline ramp",
            surface: Instruments,
            fallback: Some([' ', '.', '.', ':', ':', '|', '|', '#'][i]),
        });
    }
    v
}

/// The result of asking a renderer which glyphs its font stack covers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Probe {
    /// Glyphs the resolved font stack can draw.
    pub covered: Vec<char>,
    /// Glyphs it cannot, which helm will substitute.
    pub missing: Vec<char>,
}

impl Probe {
    /// Build a probe from a coverage predicate — typically a closure over the
    /// renderer's font database.
    pub fn run(mut covers: impl FnMut(char) -> bool) -> Self {
        let mut p = Probe::default();
        for g in inventory() {
            if covers(g.ch) {
                p.covered.push(g.ch);
            } else {
                p.missing.push(g.ch);
            }
        }
        p
    }

    /// The character to actually draw for `ch`.
    pub fn resolve(&self, ch: char) -> char {
        if !self.missing.contains(&ch) {
            return ch;
        }
        inventory()
            .into_iter()
            .find(|g| g.ch == ch)
            .and_then(|g| g.fallback)
            .unwrap_or('?')
    }

    /// A one-line summary for `helm ctl doctor`.
    pub fn summary(&self) -> String {
        let total = self.covered.len() + self.missing.len();
        if self.missing.is_empty() {
            format!("fonts: {total}/{total} glyphs covered")
        } else {
            let list: String = self.missing.iter().collect();
            format!(
                "fonts: {}/{} glyphs covered; substituting for {}",
                self.covered.len(),
                total,
                list
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_unique_and_has_a_fallback() {
        let inv = inventory();
        let mut seen = std::collections::BTreeSet::new();
        for g in &inv {
            assert!(seen.insert(g.ch), "duplicate glyph {:?}", g.ch);
            assert!(
                g.fallback.is_some(),
                "{:?} has no documented fallback",
                g.ch
            );
            assert!(
                g.fallback.unwrap().is_ascii(),
                "fallback for {:?} is not ASCII",
                g.ch
            );
        }
        assert!(inv.len() > 30);
    }

    #[test]
    fn a_bare_ascii_font_degrades_instead_of_drawing_tofu() {
        let probe = Probe::run(|c| c.is_ascii());
        assert!(!probe.missing.is_empty());
        for g in inventory() {
            let drawn = probe.resolve(g.ch);
            assert!(
                drawn.is_ascii(),
                "{:?} resolved to non-ascii {drawn:?}",
                g.ch
            );
        }
        assert_eq!(probe.resolve('𓂃'), '~');
        assert_eq!(probe.resolve('ᚠ'), '1');
        assert!(probe.summary().contains("substituting"));
    }

    #[test]
    fn a_complete_font_substitutes_nothing() {
        let probe = Probe::run(|_| true);
        assert!(probe.missing.is_empty());
        assert_eq!(probe.resolve('𓂃'), '𓂃');
        assert!(probe.summary().ends_with("glyphs covered"));
    }
}
