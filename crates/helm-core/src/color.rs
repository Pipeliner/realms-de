//! sRGB / OKLab colour maths.
//!
//! helm derives its contrast variants perceptually instead of slapping a
//! `contrast()` filter over the compositor output the way the prototype did.
//! Filtering costs a full-screen pass every frame and mangles the accent hues;
//! recomputing the palette costs nothing at runtime because it happens once,
//! when the theme is applied.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Result};

/// An 8-bit-per-channel sRGB colour.
///
/// Serialises as a `#rrggbb` string so `palette.toml` stays human-editable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Rgb {
    /// Construct from channel values.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse a `#rrggbb` (or `rrggbb`) literal.
    pub fn parse(s: &str) -> Result<Self> {
        let h = s.strip_prefix('#').unwrap_or(s);
        if h.len() != 6 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::BadColor(s.to_owned()));
        }
        let v = u32::from_str_radix(h, 16).map_err(|_| Error::BadColor(s.to_owned()))?;
        Ok(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    /// Render as `#rrggbb`.
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Render as `rrggbb`, for templates that supply their own prefix.
    pub fn hex_bare(&self) -> String {
        format!("{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Render as a CSS `rgba(r, g, b, a)` literal.
    pub fn css_rgba(&self, alpha: f32) -> String {
        format!("rgba({}, {}, {}, {})", self.r, self.g, self.b, trim(alpha))
    }

    /// Pack into `0xAARRGGBB`, the layout `tiny-skia` and Kvantum both want.
    pub fn argb(&self, alpha: f32) -> u32 {
        let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u32;
        (a << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }

    /// Composite `self` at `alpha` over `bg`.
    ///
    /// Templates for formats without an alpha channel (ANSI, Kvantum SVG fills)
    /// use this so a 30%-violet seam still looks like a 30%-violet seam.
    pub fn flatten_over(&self, bg: Rgb, alpha: f32) -> Rgb {
        let a = alpha.clamp(0.0, 1.0);
        let mix = |f: u8, b: u8| {
            (f as f32 * a + b as f32 * (1.0 - a))
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Rgb::new(mix(self.r, bg.r), mix(self.g, bg.g), mix(self.b, bg.b))
    }

    /// Convert to OKLab.
    pub fn to_oklab(self) -> Oklab {
        let (r, g, b) = (
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
        );
        let l = 0.412_221_46 * r + 0.536_332_5 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_5 * b;
        let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
        Oklab {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        }
    }

    /// Relative luminance per WCAG 2.1.
    pub fn luminance(self) -> f32 {
        0.2126 * srgb_to_linear(self.r)
            + 0.7152 * srgb_to_linear(self.g)
            + 0.0722 * srgb_to_linear(self.b)
    }

    /// WCAG contrast ratio against `other`, in `[1.0, 21.0]`.
    pub fn contrast_ratio(self, other: Rgb) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// Hue angle in degrees, `[0, 360)`. Meaningless for near-greys.
    pub fn hue_degrees(self) -> f32 {
        let lab = self.to_oklab();
        let d = lab.b.atan2(lab.a).to_degrees();
        if d < 0.0 {
            d + 360.0
        } else {
            d
        }
    }

    /// Chroma in OKLab space. Near-zero for greys.
    pub fn chroma(self) -> f32 {
        let lab = self.to_oklab();
        (lab.a * lab.a + lab.b * lab.b).sqrt()
    }
}

/// A colour in the OKLab perceptual space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklab {
    /// Perceptual lightness, roughly `[0, 1]`.
    pub l: f32,
    /// Green/red axis.
    pub a: f32,
    /// Blue/yellow axis.
    pub b: f32,
}

impl Oklab {
    /// Convert to linear sRGB without clamping, so callers can detect
    /// out-of-gamut results.
    fn to_linear(self) -> (f32, f32, f32) {
        let l = self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b;
        let m = self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b;
        let s = self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b;
        let (l, m, s) = (l * l * l, m * m * m, s * s * s);
        (
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        )
    }

    /// True when this colour is representable in sRGB.
    pub fn in_gamut(self) -> bool {
        let (r, g, b) = self.to_linear();
        [r, g, b].iter().all(|c| (-1e-4..=1.000_1).contains(c))
    }

    /// Convert back to sRGB, clamping out-of-gamut results.
    ///
    /// Prefer [`Oklab::gamut_mapped`] when the hue matters.
    pub fn to_rgb(self) -> Rgb {
        let (r, g, b) = self.to_linear();
        Rgb::new(linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b))
    }

    /// Bring an out-of-gamut colour into sRGB by reducing chroma, holding hue
    /// and lightness fixed.
    ///
    /// Plain channel clamping is the usual shortcut and it rotates hues: push
    /// helm's violet bright enough and naive clamping drifts it toward blue.
    /// Since the contrast setting exists precisely to push foregrounds around,
    /// that would make high-contrast mode a different theme rather than the
    /// same theme read more easily.
    pub fn gamut_mapped(self) -> Rgb {
        if self.in_gamut() {
            return self.to_rgb();
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            let candidate = Oklab {
                l: self.l,
                a: self.a * mid,
                b: self.b * mid,
            };
            if candidate.in_gamut() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Oklab {
            l: self.l,
            a: self.a * lo,
            b: self.b * lo,
        }
        .to_rgb()
    }
}

/// Push `fg` away from `bg` in perceptual lightness by `factor`.
///
/// `factor == 1.0` is the identity. Chroma is preserved, so accents keep their
/// hue and the palette does not wash out at high contrast — the thing a
/// `backdrop-filter: contrast()` cannot promise.
pub fn apply_contrast(fg: Rgb, bg: Rgb, factor: f32) -> Rgb {
    if (factor - 1.0).abs() < f32::EPSILON {
        return fg;
    }
    let (f, b) = (fg.to_oklab(), bg.to_oklab());
    let target = (b.l + (f.l - b.l) * factor).clamp(0.0, 1.0);
    let l = reachable_lightness(f, target);
    Oklab { l, ..f }.gamut_mapped()
}

/// The lightness closest to `target` that keeps `base`'s hue *and* chroma
/// inside sRGB.
///
/// Saturated colours run out of gamut before they run out of lightness: there
/// is no such thing as a very light, very saturated violet in sRGB. Rather than
/// silently desaturating an accent into a near-grey — whose hue is then
/// numerical noise — helm stops pushing at the gamut boundary. The accent gets
/// as much extra contrast as sRGB can actually give it and stays recognisably
/// itself.
fn reachable_lightness(base: Oklab, target: f32) -> f32 {
    let candidate = Oklab { l: target, ..base };
    if candidate.in_gamut() {
        return target;
    }
    let (mut good, mut bad) = (base.l, target);
    for _ in 0..24 {
        let mid = (good + bad) / 2.0;
        if (Oklab { l: mid, ..base }).in_gamut() {
            good = mid;
        } else {
            bad = mid;
        }
    }
    good
}

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

fn trim(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Rgb::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_round_trips() {
        let c = Rgb::parse("#a692ec").unwrap();
        assert_eq!((c.r, c.g, c.b), (0xa6, 0x92, 0xec));
        assert_eq!(c.hex(), "#a692ec");
        assert!(Rgb::parse("#xyzxyz").is_err());
        assert!(Rgb::parse("#abc").is_err());
    }

    #[test]
    fn oklab_round_trips_within_one_step() {
        for hex in ["#05060c", "#a692ec", "#d9b06a", "#f2f5fb", "#7fd4c1"] {
            let c = Rgb::parse(hex).unwrap();
            let back = c.to_oklab().gamut_mapped();
            for (x, y) in [(c.r, back.r), (c.g, back.g), (c.b, back.b)] {
                assert!(x.abs_diff(y) <= 1, "{hex}: {x} vs {y}");
            }
        }
    }

    #[test]
    fn contrast_raises_separation() {
        let bg = Rgb::parse("#05060c").unwrap();
        for hex in ["#c2cbde", "#a692ec", "#7fd4c1", "#66748e"] {
            let fg = Rgb::parse(hex).unwrap();
            let boosted = apply_contrast(fg, bg, 1.30);
            assert!(
                boosted.contrast_ratio(bg) > fg.contrast_ratio(bg),
                "{hex} did not gain contrast"
            );
        }
    }

    #[test]
    fn contrast_preserves_accent_hue() {
        // Hue is only meaningful for colours with real chroma; near-greys are
        // excluded deliberately, since their hue angle is numerical noise.
        let bg = Rgb::parse("#05060c").unwrap();
        for hex in ["#a692ec", "#a3bff2", "#d9b06a", "#7fd4c1", "#7fc9e8"] {
            let fg = Rgb::parse(hex).unwrap();
            assert!(fg.chroma() > 0.02, "{hex} is not chromatic enough to test");
            for factor in [0.85, 1.20, 1.40] {
                let out = apply_contrast(fg, bg, factor);
                let mut delta = (out.hue_degrees() - fg.hue_degrees()).abs();
                if delta > 180.0 {
                    delta = 360.0 - delta;
                }
                assert!(delta < 4.0, "{hex} at {factor}: hue drifted {delta:.1} deg");
            }
        }
    }

    #[test]
    fn identity_contrast_is_a_no_op() {
        let bg = Rgb::parse("#05060c").unwrap();
        let fg = Rgb::parse("#a692ec").unwrap();
        assert_eq!(apply_contrast(fg, bg, 1.0), fg);
    }

    #[test]
    fn flatten_matches_manual_composite() {
        let violet = Rgb::parse("#a692ec").unwrap();
        let pane = Rgb::parse("#0a0c14").unwrap();
        assert_eq!(violet.flatten_over(pane, 1.0), violet);
        assert_eq!(violet.flatten_over(pane, 0.0), pane);
        let half = violet.flatten_over(pane, 0.5);
        assert_eq!(
            half.r,
            ((0xa6 as f32 * 0.5) + (0x0a as f32 * 0.5)).round() as u8
        );
    }

    #[test]
    fn contrast_stops_at_the_gamut_boundary_instead_of_desaturating() {
        let bg = Rgb::parse("#05060c").unwrap();
        for hex in ["#a692ec", "#a3bff2", "#7fd4c1", "#d9b06a", "#7fc9e8"] {
            let fg = Rgb::parse(hex).unwrap();
            let out = apply_contrast(fg, bg, 1.40);
            assert!(
                out.chroma() > fg.chroma() * 0.9,
                "{hex} lost chroma: {:.4} -> {:.4}",
                fg.chroma(),
                out.chroma()
            );
            assert!(out.contrast_ratio(bg) >= fg.contrast_ratio(bg));
        }
    }

    #[test]
    fn gamut_mapping_holds_hue_where_clamping_would_not() {
        // A violet pushed to near-white lightness is out of sRGB gamut.
        let base = Rgb::parse("#a692ec").unwrap();
        let lab = Oklab {
            l: 0.95,
            ..base.to_oklab()
        };
        assert!(!lab.in_gamut());
        let drift = |c: Rgb| {
            let mut d = (c.hue_degrees() - base.hue_degrees()).abs();
            if d > 180.0 {
                d = 360.0 - d;
            }
            d
        };
        assert!(drift(lab.gamut_mapped()) < drift(lab.to_rgb()));
    }

    #[test]
    fn known_contrast_ratios() {
        let white = Rgb::new(255, 255, 255);
        let black = Rgb::new(0, 0, 0);
        assert!((white.contrast_ratio(black) - 21.0).abs() < 0.05);
        assert!((white.contrast_ratio(white) - 1.0).abs() < 0.001);
    }
}
