//! `palette.toml`: the one place a colour may be written down.
//!
//! Loading is deliberately strict. A palette that fails [`Palette::lint`] is
//! rejected before any template is rendered, because a bad palette is far
//! cheaper to catch here than after it has been fanned out into a GTK
//! stylesheet, a Kvantum theme, four TUI configs and a live compositor.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::color::{apply_contrast, Rgb};
use crate::{Error, Result};

/// Minimum WCAG contrast ratio helm requires of body text on the pane
/// background. Below this, text on a dark surface stops being comfortable to
/// read for hours at a time — which is the only way anyone uses a DE.
pub const MIN_BODY_CONTRAST: f32 = 4.5;

/// Minimum contrast for meta/micro text, which is intentionally recessive.
pub const MIN_META_CONTRAST: f32 = 2.6;

/// Minimum separation, in OKLab hue degrees, between saturated accents.
pub const MIN_ACCENT_HUE_SEPARATION: f32 = 25.0;

/// Background surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Backgrounds {
    /// Root surface behind everything.
    pub void: Rgb,
    /// Unfocused window body.
    pub pane: Rgb,
    /// Focused window body.
    pub focused: Rgb,
    /// Inset blocks and result rows.
    pub raised: Rgb,
    /// Bar gradient start.
    pub bar_top: Rgb,
    /// Bar gradient end.
    pub bar_bot: Rgb,
}

/// Foreground ramp, brightest to faintest.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Texts {
    /// Primary values and the active orbit.
    pub bright: Rgb,
    /// Body copy.
    pub normal: Rgb,
    /// Which-key labels and occupied orbits.
    pub mid: Rgb,
    /// Secondary body copy.
    pub soft: Rgb,
    /// Meta rows.
    pub dim: Rgb,
    /// Micro text, epithets, empty orbits.
    pub faint: Rgb,
}

/// The five accent hues.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Accents {
    /// Arcane: focus, odin, hecate.
    pub violet: Rgb,
    /// Info: urania, cpu, mode badge.
    pub starlight: Rgb,
    /// Warn: horus, battery.
    pub gold: Rgb,
    /// Ok: thoth, marks.
    pub teal: Rgb,
    /// hermes.
    pub cyan: Rgb,
    /// Error, and ANSI colour 1/9. Not used in helm's own chrome.
    pub red: Rgb,
}

impl Accents {
    /// Look an accent up by the name used in `[pantheon]`.
    pub fn by_name(&self, name: &str) -> Option<Rgb> {
        Some(match name {
            "violet" => self.violet,
            "starlight" => self.starlight,
            "gold" => self.gold,
            "teal" => self.teal,
            "cyan" => self.cyan,
            "red" => self.red,
            _ => return None,
        })
    }

    /// All accents paired with their names.
    pub fn named(&self) -> [(&'static str, Rgb); 6] {
        [
            ("violet", self.violet),
            ("starlight", self.starlight),
            ("gold", self.gold),
            ("teal", self.teal),
            ("cyan", self.cyan),
            ("red", self.red),
        ]
    }
}

/// Border colours and their alphas, kept separate so alpha-less formats can
/// flatten them against a known background.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Borders {
    /// Focused window border.
    pub focused: Rgb,
    /// Focused border alpha.
    pub focused_alpha: f32,
    /// Seam between tiles.
    pub seam: Rgb,
    /// Seam alpha.
    pub seam_alpha: f32,
    /// Unfocused window border.
    pub neutral: Rgb,
    /// Unfocused border alpha.
    pub neutral_alpha: f32,
    /// Bar's bottom rule.
    pub bar_bottom: Rgb,
    /// Bar rule alpha.
    pub bar_bottom_alpha: f32,
    /// Alpha applied to a pane's pantheon colour in its header underline.
    pub pantheon_alpha: f32,
}

/// Font stack and type scale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Typography {
    /// Preferred family.
    pub family: String,
    /// Ordered fallback chain, tried left to right.
    pub fallback: Vec<String>,
    /// Regular weight.
    pub weight_regular: u16,
    /// Medium weight.
    pub weight_medium: u16,
    /// Bold weight.
    pub weight_bold: u16,
    /// Body size in px.
    pub size_body: f32,
    /// Meta size in px.
    pub size_meta: f32,
    /// Micro size in px.
    pub size_micro: f32,
    /// Page-title size in px.
    pub size_title: f32,
    /// Multiplier applied to font size for line boxes.
    pub line_height: f32,
}

/// Fixed pixel metrics. helm has no border radius; `radius` exists only so a
/// template can assert it is zero.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Top bar height.
    pub bar_height: i32,
    /// Window header height.
    pub header_height: i32,
    /// Which-key strip height.
    pub whichkey_height: i32,
    /// Border thickness.
    pub border_width: i32,
    /// Always zero.
    pub radius: i32,
    /// Hecate panel width.
    pub launcher_width: i32,
    /// Charon portal width.
    pub portal_width: i32,
}

/// The glyph inventory, verified against the resolved font stack at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphSet {
    /// Orbit runes, one per orbit.
    pub runes: Vec<String>,
    /// Window control glyphs.
    pub controls: Vec<String>,
    /// Filled meter cell.
    pub meter_full: String,
    /// Empty meter cell.
    pub meter_empty: String,
    /// Braille sparkline ramp, low to high.
    pub spark: Vec<String>,
    /// Shell prompt sigil.
    pub prompt_sigil: String,
    /// Substitute when `prompt_sigil` is unavailable.
    pub prompt_sigil_fallback: String,
}

/// The whole palette file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    /// Theme name, used in generated file headers.
    pub name: String,
    /// `dark` or `light`; helm ships dark only for now.
    pub variant: String,
    /// Perceptual contrast multiplier in `[0.85, 1.40]`.
    pub contrast: f32,
    /// Background surfaces.
    pub background: Backgrounds,
    /// Foreground ramp.
    pub text: Texts,
    /// Accent hues.
    pub accent: Accents,
    /// Border colours.
    pub border: Borders,
    /// Tool-to-accent mapping.
    pub pantheon: BTreeMap<String, String>,
    /// Font stack.
    pub typography: Typography,
    /// Pixel metrics.
    pub metrics: Metrics,
    /// Glyph inventory.
    pub glyphs: GlyphSet,
}

/// A problem found by [`Palette::lint`].
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// Dotted path of the offending value.
    pub path: String,
    /// What is wrong, in one line.
    pub message: String,
    /// True when the palette must be rejected rather than merely grumbled at.
    pub fatal: bool,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}: {}",
            if self.fatal { "error" } else { "warn " },
            self.path,
            self.message
        )
    }
}

impl Palette {
    /// Parse a palette from TOML text.
    pub fn from_toml(src: &str) -> Result<Self> {
        let p: Palette = toml::from_str(src)?;
        p.validate_ranges()?;
        Ok(p)
    }

    /// Read and parse a palette file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let src = std::fs::read_to_string(path.as_ref())
            .map_err(|e| Error::PaletteRange(format!("{}: {e}", path.as_ref().display())))?;
        Self::from_toml(&src)
    }

    /// Serialise back to TOML.
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("palette is always serialisable")
    }

    fn validate_ranges(&self) -> Result<()> {
        if !(0.85..=1.40).contains(&self.contrast) {
            return Err(Error::PaletteRange(format!(
                "contrast {} outside [0.85, 1.40]",
                self.contrast
            )));
        }
        if self.metrics.radius != 0 {
            return Err(Error::PaletteRange(
                "metrics.radius must be 0: helm has no rounded corners".into(),
            ));
        }
        if self.glyphs.runes.len() != crate::ledger::ORBIT_COUNT {
            return Err(Error::PaletteRange(format!(
                "glyphs.runes must list exactly {} runes",
                crate::ledger::ORBIT_COUNT
            )));
        }
        if self.typography.fallback.is_empty() {
            return Err(Error::PaletteRange(
                "typography.fallback must not be empty".into(),
            ));
        }
        for (tool, accent) in &self.pantheon {
            if self.accent.by_name(accent).is_none() {
                return Err(Error::PaletteRange(format!(
                    "pantheon.{tool} names unknown accent {accent:?}"
                )));
            }
        }
        Ok(())
    }

    /// Return the palette with `contrast` folded into every foreground.
    ///
    /// This is what templates render. Applying it here — once, at theme-apply
    /// time — is why helm needs no compositor-side contrast filter.
    pub fn derived(&self) -> Palette {
        let f = self.contrast;
        let bg = self.background.void;
        let pane = self.background.pane;
        let mut p = self.clone();
        p.text = Texts {
            bright: apply_contrast(self.text.bright, bg, f),
            normal: apply_contrast(self.text.normal, bg, f),
            mid: apply_contrast(self.text.mid, bg, f),
            soft: apply_contrast(self.text.soft, bg, f),
            dim: apply_contrast(self.text.dim, bg, f),
            faint: apply_contrast(self.text.faint, bg, f),
        };
        p.accent = Accents {
            violet: apply_contrast(self.accent.violet, pane, f),
            starlight: apply_contrast(self.accent.starlight, pane, f),
            gold: apply_contrast(self.accent.gold, pane, f),
            teal: apply_contrast(self.accent.teal, pane, f),
            cyan: apply_contrast(self.accent.cyan, pane, f),
            red: apply_contrast(self.accent.red, pane, f),
        };
        p.contrast = 1.0;
        p
    }

    /// The accent assigned to a tool, falling back to violet.
    pub fn pantheon_color(&self, tool: &str) -> Rgb {
        self.pantheon
            .get(tool)
            .and_then(|n| self.accent.by_name(n))
            .unwrap_or(self.accent.violet)
    }

    /// Check readability and hue discipline.
    ///
    /// Returns every finding rather than the first, so a palette author fixes
    /// one round of complaints instead of ten.
    pub fn lint(&self) -> Vec<Finding> {
        let d = self.derived();
        let mut out = Vec::new();
        let bg = d.background.pane;

        let body = [
            ("text.bright", d.text.bright, MIN_BODY_CONTRAST),
            ("text.normal", d.text.normal, MIN_BODY_CONTRAST),
            ("text.soft", d.text.soft, MIN_BODY_CONTRAST),
            ("text.mid", d.text.mid, MIN_META_CONTRAST),
            ("text.dim", d.text.dim, MIN_META_CONTRAST),
            ("text.faint", d.text.faint, MIN_META_CONTRAST),
        ];
        for (path, c, floor) in body {
            let ratio = c.contrast_ratio(bg);
            if ratio < floor {
                out.push(Finding {
                    path: path.into(),
                    message: format!(
                        "contrast {ratio:.2}:1 on background.pane is below {floor:.1}:1"
                    ),
                    fatal: floor == MIN_BODY_CONTRAST,
                });
            }
        }

        for (name, c) in d.accent.named() {
            let ratio = c.contrast_ratio(bg);
            if ratio < MIN_META_CONTRAST {
                out.push(Finding {
                    path: format!("accent.{name}"),
                    message: format!("contrast {ratio:.2}:1 on background.pane is below {MIN_META_CONTRAST:.1}:1"),
                    fatal: false,
                });
            }
        }

        let accents = d.accent.named();
        for (i, (an, a)) in accents.iter().enumerate() {
            for (bn, b) in &accents[i + 1..] {
                // Hue is meaningless for near-greys; skip them.
                if a.chroma() < 0.02 || b.chroma() < 0.02 {
                    continue;
                }
                let mut delta = (a.hue_degrees() - b.hue_degrees()).abs();
                if delta > 180.0 {
                    delta = 360.0 - delta;
                }
                if delta < MIN_ACCENT_HUE_SEPARATION {
                    out.push(Finding {
                        path: format!("accent.{an}/{bn}"),
                        message: format!(
                            "hues are only {delta:.1} deg apart; keep accents >{MIN_ACCENT_HUE_SEPARATION:.0} deg apart"
                        ),
                        fatal: false,
                    });
                }
            }
        }

        if d.background.void.contrast_ratio(d.background.pane) > 1.6 {
            out.push(Finding {
                path: "background.void/pane".into(),
                message: "surfaces differ too strongly; seams will read as gaps".into(),
                fatal: false,
            });
        }

        out
    }

    /// True when no fatal finding stands in the way of applying this palette.
    pub fn is_applicable(&self) -> bool {
        !self.lint().iter().any(|f| f.fatal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = include_str!("../../../palette.toml");

    fn palette() -> Palette {
        Palette::from_toml(SRC).expect("shipped palette must parse")
    }

    #[test]
    fn shipped_palette_parses_and_round_trips() {
        let p = palette();
        assert_eq!(p.name, "helm-void");
        assert_eq!(p.background.void, Rgb::parse("#05060c").unwrap());
        assert_eq!(p.accent.violet, Rgb::parse("#a692ec").unwrap());
        assert_eq!(p.metrics.bar_height, 32);
        let again = Palette::from_toml(&p.to_toml()).unwrap();
        assert_eq!(p, again);
    }

    #[test]
    fn shipped_palette_passes_its_own_lint() {
        let findings = palette().lint();
        let fatal: Vec<_> = findings.iter().filter(|f| f.fatal).collect();
        assert!(
            fatal.is_empty(),
            "shipped palette has fatal findings: {fatal:#?}"
        );
    }

    #[test]
    fn shipped_palette_survives_the_whole_contrast_range() {
        for step in 0..=11 {
            let mut p = palette();
            p.contrast = 0.85 + step as f32 * 0.05;
            let fatal: Vec<_> = p.lint().into_iter().filter(|f| f.fatal).collect();
            assert!(fatal.is_empty(), "contrast {}: {fatal:#?}", p.contrast);
        }
    }

    #[test]
    fn pantheon_maps_every_tool_to_a_real_accent() {
        let p = palette();
        for tool in [
            "odin", "thoth", "hermes", "horus", "urania", "hecate", "charon",
        ] {
            assert!(p.pantheon.contains_key(tool), "missing pantheon.{tool}");
        }
        assert_eq!(p.pantheon_color("thoth"), p.accent.teal);
        assert_eq!(p.pantheon_color("nobody"), p.accent.violet);
    }

    #[test]
    fn out_of_range_values_are_rejected_at_parse_time() {
        let bad = SRC.replace("contrast = 1.08", "contrast = 2.5");
        assert!(Palette::from_toml(&bad).is_err());
        let bad = SRC.replace("radius           = 0", "radius           = 6");
        assert!(Palette::from_toml(&bad).is_err());
        let bad = SRC.replace(r#"thoth  = "teal""#, r#"thoth  = "chartreuse""#);
        assert!(Palette::from_toml(&bad).is_err());
    }

    #[test]
    fn lint_catches_muddy_text_and_duplicate_accents() {
        let mut p = palette();
        p.text.normal = p.background.pane;
        p.accent.cyan = p.accent.teal;
        let findings = p.lint();
        assert!(findings.iter().any(|f| f.path == "text.normal" && f.fatal));
        assert!(findings.iter().any(|f| f.path.contains("teal")));
        assert!(!p.is_applicable());
    }

    #[test]
    fn derived_palette_is_idempotent() {
        let d = palette().derived();
        assert_eq!(d.contrast, 1.0);
        assert_eq!(d.derived(), d);
    }
}
