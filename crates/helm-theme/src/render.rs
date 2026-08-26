//! The placeholder renderer.

use helm_core::Palette;

use crate::Result;

/// Render one template source against `palette`.
pub fn render(palette: &Palette, id: &str, source: &str) -> Result<String> {
    let _ = (palette, id, source);
    unimplemented!()
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
                        "{} at contrast {} emitted the undelivered {name}",
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
                    if *b != b'#' {
                        continue;
                    }
                    let rest = &bytes[i + 1..];
                    let literal = rest.len() >= 6
                        && rest[..6].iter().all(u8::is_ascii_hexdigit)
                        && Rgb::parse(std::str::from_utf8(&rest[..6]).unwrap()).is_ok();
                    assert!(
                        !literal,
                        "{}:{}: colour literal outside palette.toml",
                        t.id,
                        n + 1
                    );
                }
            }
        }
    }
}
