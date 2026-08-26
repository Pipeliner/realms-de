//! The placeholder renderer.
//!
//! The vocabulary is small and closed — the table in `docs/INTERFACES.md` §2 is
//! all of it — so this is a hand-written scanner rather than a template engine.
//! That buys the one property the design actually needs: an unknown placeholder
//! names itself and stops the apply, instead of rendering as the empty string
//! and shipping an invisible widget.
//!
//! ```text
//! placeholder := path ( "." transform )*
//! transform   := "bare" | "rgba(" alpha ")" | "over(" path "," alpha ")"
//! alpha       := a decimal literal, or a path to one of the palette's alphas
//! ```
//!
//! `over` yields a colour and so may be followed by `bare` or `rgba`; the other
//! two yield text and end the chain. Accepting a path where an alpha is
//! expected keeps `border.seam_alpha` written down once, in the same file as
//! the colour it belongs to.

use helm_core::color::Rgb;
use helm_core::Palette;

use crate::{Error, Result};

/// Render one template source against `palette`.
///
/// The palette is derived first, so `contrast` is folded in exactly once here
/// and templates never see — or apply — a contrast setting.
pub fn render(palette: &Palette, id: &str, source: &str) -> Result<String> {
    render_derived(&palette.derived(), id, source)
}

/// [`render`] against an already-derived palette.
///
/// `Palette::derived` is idempotent, so this is an optimisation for callers
/// rendering many templates from one palette, not a second contract.
pub(crate) fn render_derived(derived: &Palette, id: &str, source: &str) -> Result<String> {
    let mut out = String::with_capacity(source.len() + source.len() / 4);
    let mut rest = source;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let close = after
            .find("}}")
            .ok_or_else(|| Error::UnterminatedPlaceholder {
                template: id.to_owned(),
                offset: source.len() - rest.len() + open,
            })?;
        out.push_str(&expand(derived, id, after[..close].trim())?);
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A resolved palette value, before it is formatted for a particular file.
enum Value {
    Color(Rgb),
    Text(String),
    Number(f32),
}

impl Value {
    fn render(self) -> String {
        match self {
            Value::Color(c) => c.hex(),
            Value::Text(s) => s,
            Value::Number(n) => number(n),
        }
    }
}

fn expand(p: &Palette, id: &str, body: &str) -> Result<String> {
    let unknown = || Error::UnknownPlaceholder {
        template: id.to_owned(),
        placeholder: body.to_owned(),
    };

    let parts = split_unbracketed(body);
    let split = parts
        .iter()
        .position(|s| is_transform(s))
        .unwrap_or(parts.len());
    let mut value = resolve(p, &parts[..split].join(".")).ok_or_else(unknown)?;

    for t in &parts[split..] {
        let color = match value {
            Value::Color(c) => c,
            // Only colours have transforms, and `{{ metrics.bar_height.bare }}`
            // is a typo worth failing on rather than guessing at.
            _ => return Err(unknown()),
        };
        value = if *t == "bare" {
            Value::Text(color.hex_bare())
        } else if let Some(arg) = t.strip_prefix("rgba(").and_then(|s| s.strip_suffix(')')) {
            Value::Text(color.css_rgba(alpha(p, arg).ok_or_else(unknown)?))
        } else if let Some(args) = t.strip_prefix("over(").and_then(|s| s.strip_suffix(')')) {
            let (bg, a) = args.split_once(',').ok_or_else(unknown)?;
            let bg = match resolve(p, bg.trim()) {
                Some(Value::Color(c)) => c,
                _ => return Err(unknown()),
            };
            Value::Color(color.flatten_over(bg, alpha(p, a).ok_or_else(unknown)?))
        } else {
            return Err(unknown());
        };
    }
    Ok(value.render())
}

fn is_transform(segment: &str) -> bool {
    segment == "bare" || segment.starts_with("rgba(") || segment.starts_with("over(")
}

/// Split on `.` outside parentheses, so `rgba(0.3)` survives as one segment.
fn split_unbracketed(body: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (Vec::new(), 0usize, 0usize);
    for (i, c) in body.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '.' if depth == 0 => {
                out.push(body[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(body[start..].trim());
    out
}

/// A literal alpha, or the path of one of the palette's own alphas.
fn alpha(p: &Palette, arg: &str) -> Option<f32> {
    let arg = arg.trim();
    match arg.parse::<f32>() {
        Ok(v) => Some(v),
        Err(_) => match resolve(p, arg)? {
            Value::Number(n) => Some(n),
            _ => None,
        },
    }
}

fn resolve(p: &Palette, path: &str) -> Option<Value> {
    let (section, field) = path.split_once('.').unwrap_or((path, ""));
    Some(match (section, field) {
        ("name", "") => Value::Text(p.name.clone()),
        ("variant", "") => Value::Text(p.variant.clone()),
        ("contrast", "") => Value::Number(p.contrast),

        ("background", f) => Value::Color(match f {
            "void" => p.background.void,
            "pane" => p.background.pane,
            "focused" => p.background.focused,
            "raised" => p.background.raised,
            "bar_top" => p.background.bar_top,
            "bar_bot" => p.background.bar_bot,
            _ => return None,
        }),
        ("text", f) => Value::Color(match f {
            "bright" => p.text.bright,
            "normal" => p.text.normal,
            "mid" => p.text.mid,
            "soft" => p.text.soft,
            "dim" => p.text.dim,
            "faint" => p.text.faint,
            _ => return None,
        }),
        ("accent", f) => Value::Color(p.accent.by_name(f)?),
        // Not `Palette::pantheon_color`: that falls back to violet for an
        // unknown tool, which is the silent wrong colour this crate refuses.
        ("pantheon", f) => Value::Color(p.accent.by_name(p.pantheon.get(f)?)?),
        ("border", f) => match f {
            "focused" => Value::Color(p.border.focused),
            "seam" => Value::Color(p.border.seam),
            "neutral" => Value::Color(p.border.neutral),
            "bar_bottom" => Value::Color(p.border.bar_bottom),
            "focused_alpha" => Value::Number(p.border.focused_alpha),
            "seam_alpha" => Value::Number(p.border.seam_alpha),
            "neutral_alpha" => Value::Number(p.border.neutral_alpha),
            "bar_bottom_alpha" => Value::Number(p.border.bar_bottom_alpha),
            "pantheon_alpha" => Value::Number(p.border.pantheon_alpha),
            _ => return None,
        },
        ("typography", f) => match f {
            "family" => Value::Text(p.typography.family.clone()),
            "fallback" => Value::Text(p.typography.fallback.join(", ")),
            "weight_regular" => Value::Number(p.typography.weight_regular as f32),
            "weight_medium" => Value::Number(p.typography.weight_medium as f32),
            "weight_bold" => Value::Number(p.typography.weight_bold as f32),
            "size_body" => Value::Number(p.typography.size_body),
            "size_meta" => Value::Number(p.typography.size_meta),
            "size_micro" => Value::Number(p.typography.size_micro),
            "size_title" => Value::Number(p.typography.size_title),
            "line_height" => Value::Number(p.typography.line_height),
            _ => return None,
        },
        ("metrics", f) => Value::Number(match f {
            "bar_height" => p.metrics.bar_height,
            "header_height" => p.metrics.header_height,
            "whichkey_height" => p.metrics.whichkey_height,
            "border_width" => p.metrics.border_width,
            "radius" => p.metrics.radius,
            "launcher_width" => p.metrics.launcher_width,
            "portal_width" => p.metrics.portal_width,
            _ => return None,
        } as f32),
        ("glyphs", f) => Value::Text(match f {
            "meter_full" => p.glyphs.meter_full.clone(),
            "meter_empty" => p.glyphs.meter_empty.clone(),
            "prompt_sigil" => p.glyphs.prompt_sigil.clone(),
            "prompt_sigil_fallback" => p.glyphs.prompt_sigil_fallback.clone(),
            _ => return None,
        }),
        _ => return None,
    })
}

/// Format a number the way a config file wants it: `32`, not `32.000`.
fn number(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates;
    use helm_core::color::Rgb;

    fn shipped() -> Palette {
        Palette::from_toml(crate::SHIPPED_PALETTE).expect("shipped palette must parse")
    }

    /// The standing exception in docs/specs/README.md: colour maths gets a
    /// sweep, not an example. A template that renders correctly at the shipped
    /// contrast and breaks at 1.40 is exactly the failure that survives review.
    #[test]
    fn every_template_renders_across_the_whole_contrast_range() {
        for step in 0..=11 {
            let mut p = shipped();
            p.contrast = 0.85 + step as f32 * 0.05;
            let derived = p.derived();
            for t in templates() {
                let out = render(&p, t.id, t.source)
                    .unwrap_or_else(|e| panic!("{} at contrast {}: {e}", t.id, p.contrast));
                assert!(!out.contains("{{"), "{} at {}", t.id, p.contrast);
                for (name, source) in p.accent.named() {
                    let moved = derived.accent.by_name(name).unwrap();
                    if moved == source {
                        continue;
                    }
                    assert!(
                        !out.contains(&source.hex_bare()),
                        "{} at contrast {} emitted the underived {name}",
                        t.id,
                        p.contrast
                    );
                }
            }
        }
    }

    #[test]
    fn over_agrees_with_flatten_over_across_the_alpha_range() {
        let p = shipped();
        let d = p.derived();
        for step in 0..=10 {
            let alpha = step as f32 / 10.0;
            let src = format!("{{{{ accent.violet.over(background.pane, {alpha}) }}}}");
            let want = d.accent.violet.flatten_over(d.background.pane, alpha);
            assert_eq!(render(&p, "t", &src).unwrap(), want.hex(), "alpha {alpha}");
        }
    }

    #[test]
    fn scalar_and_colour_forms_agree_with_the_core_formatters() {
        let p = shipped();
        let d = p.derived();
        let cases = [
            ("{{ accent.violet }}", d.accent.violet.hex()),
            ("{{ accent.violet.bare }}", d.accent.violet.hex_bare()),
            (
                "{{ accent.violet.rgba(0.3) }}",
                d.accent.violet.css_rgba(0.3),
            ),
            (
                "{{ border.seam.rgba(border.seam_alpha) }}",
                d.border.seam.css_rgba(d.border.seam_alpha),
            ),
            ("{{ metrics.bar_height }}", d.metrics.bar_height.to_string()),
            ("{{ typography.family }}", d.typography.family.clone()),
            ("{{ pantheon.thoth }}", d.accent.teal.hex()),
        ];
        for (src, want) in cases {
            assert_eq!(render(&p, "t", src).unwrap(), want, "{src}");
        }
    }

    #[test]
    fn unknown_paths_and_transforms_are_errors_not_empty_strings() {
        let p = shipped();
        for src in [
            "{{ accent.mauve }}",
            "{{ background }}",
            "{{ accent.violet.chartreuse }}",
            "{{ metrics.bar_height.bare }}",
            "{{ accent.violet.over(accent.mauve, 0.3) }}",
            "{{ pantheon.zeus }}",
        ] {
            let err = render(&p, "t", src).expect_err(src).to_string();
            assert!(err.contains("t: "), "{src} -> {err}");
        }
        assert!(render(&p, "t", "{{ accent.violet ").is_err());
    }

    #[test]
    fn no_shipped_template_writes_a_colour_of_its_own() {
        for t in templates() {
            for (n, line) in t.source.lines().enumerate() {
                let bytes = line.as_bytes();
                for (i, b) in bytes.iter().enumerate() {
                    if *b != b'#' || bytes.len() < i + 7 {
                        continue;
                    }
                    let candidate = &bytes[i + 1..i + 7];
                    assert!(
                        !candidate.iter().all(u8::is_ascii_hexdigit)
                            || Rgb::parse(std::str::from_utf8(candidate).unwrap()).is_err(),
                        "{}:{}: colour literal outside palette.toml",
                        t.id,
                        n + 1
                    );
                }
            }
        }
    }
}
