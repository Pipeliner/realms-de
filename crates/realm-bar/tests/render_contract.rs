use realm_bar::{
    surface_config, Anchor, BarRenderer, ElementRole, Layer, LogicalRect, SurfaceKind,
};
use realm_core::{
    glyphs::{inventory, Probe},
    keys::{Keymap, Mode},
    layout::Layout,
    palette::Palette,
    state::{Module, OrbitDisplay, RealmState},
};

fn palette() -> Palette {
    Palette::from_toml(include_str!("../../../palette.toml")).unwrap()
}

fn state() -> RealmState {
    let mut state = RealmState {
        focused_title: "realm implementation notes".into(),
        layout: Layout::Mono,
        mode: Mode::Resize,
        modules: vec![
            Module {
                id: "gpu".into(),
                text: "gpu 44°".into(),
                accent: None,
                urgent: false,
            },
            Module {
                id: "net".into(),
                text: "up 18k down 1.2M".into(),
                accent: None,
                urgent: false,
            },
            Module {
                id: "mem".into(),
                text: "mem 9.8G".into(),
                accent: None,
                urgent: false,
            },
            Module {
                id: "vol".into(),
                text: "vol 64%".into(),
                accent: None,
                urgent: false,
            },
            Module {
                id: "cpu".into(),
                text: "cpu 31%".into(),
                accent: Some("starlight".into()),
                urgent: false,
            },
            Module {
                id: "battery".into(),
                text: "bat 87%".into(),
                accent: Some("gold".into()),
                urgent: false,
            },
            Module {
                id: "clock".into(),
                text: "12 Sep 2026 20:56".into(),
                accent: None,
                urgent: false,
            },
        ],
        ..RealmState::default()
    };
    for (index, orbit) in state.orbits.iter_mut().enumerate() {
        orbit.display = match index {
            0 => OrbitDisplay::Active,
            1 | 2 => OrbitDisplay::Occupied,
            _ => OrbitDisplay::Empty,
        };
    }
    state
}

#[test]
fn layer_surface_contract_uses_bottom_exclusive_strips_and_noninteractive_overlay() {
    let palette = palette();
    let bar = surface_config(SurfaceKind::Bar, &palette, 1920, 1080, 1);
    assert_eq!(bar.layer, Layer::Bottom);
    assert_eq!(bar.anchors, Anchor::TOP | Anchor::LEFT | Anchor::RIGHT);
    assert_eq!(bar.logical_size, (0, 32));
    assert_eq!(bar.buffer_size, (1920, 32));
    assert_eq!(bar.exclusive_zone, 32);
    assert!(!bar.keyboard_interactive);

    let strip = surface_config(SurfaceKind::WhichKey, &palette, 1920, 1080, 2);
    assert_eq!(strip.layer, Layer::Bottom);
    assert_eq!(strip.buffer_size, (3840, 52));
    assert_eq!(strip.exclusive_zone, 26);

    let grimoire = surface_config(SurfaceKind::Grimoire, &palette, 1920, 1080, 2);
    assert_eq!(grimoire.layer, Layer::Overlay);
    assert_eq!(grimoire.anchors, Anchor::all());
    assert_eq!(grimoire.logical_size, (0, 0));
    assert_eq!(grimoire.exclusive_zone, -1);
    assert!(!grimoire.keyboard_interactive);
}

#[test]
fn bar_uses_state_semantics_and_palette_treatments() {
    let palette = palette();
    let probe = Probe::run(|_| true);
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let frame = renderer.render_bar(&state(), &palette, &probe, 1920, 1);

    let orbits: Vec<_> = frame
        .elements
        .iter()
        .filter(|element| element.role == ElementRole::Orbit)
        .collect();
    assert_eq!(orbits.len(), 6);
    assert_eq!(orbits[0].rect.width, 28);
    assert_eq!(orbits[0].foreground, palette.text.bright);
    assert_eq!(orbits[0].background, Some((palette.accent.violet, 36)));
    assert_eq!(orbits[0].bottom_rule, Some((palette.accent.violet, 2)));
    assert_eq!(orbits[1].foreground, palette.text.mid);
    assert_eq!(orbits[4].foreground, palette.text.faint);

    let layout = frame.element(ElementRole::Layout).unwrap();
    assert!(layout.text.ends_with(Layout::Mono.label()));
    assert_eq!(layout.foreground, palette.text.dim);
    let badge = frame.element(ElementRole::Mode).unwrap();
    assert!(badge.text.ends_with(Mode::Resize.badge()));
    assert_eq!(badge.foreground, palette.accent.starlight);
    assert_eq!(badge.background, Some((palette.accent.starlight, 36)));
}

#[test]
fn unchanged_state_is_gated_and_fixed_width_clock_damages_only_its_box() {
    let palette = palette();
    let probe = Probe::run(|_| true);
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let first = renderer.render_bar(&state(), &palette, &probe, 1920, 1);
    assert_eq!(first.damage.rect(), Some(LogicalRect::new(0, 0, 1920, 32)));

    let unchanged = renderer.render_bar(&state(), &palette, &probe, 1920, 1);
    assert_eq!(unchanged.damage.rect(), None);

    let mut tick = state();
    tick.modules.last_mut().unwrap().text = "12 Sep 2026 20:57".into();
    let clock = renderer.render_bar(&tick, &palette, &probe, 1920, 1);
    assert_eq!(
        clock.damage.rect(),
        Some(clock.element(ElementRole::ModuleClock).unwrap().rect)
    );
}

#[test]
fn urgent_module_uses_its_palette_accent_without_animation() {
    let palette = palette();
    let probe = Probe::run(|_| true);
    let mut state = state();
    state
        .modules
        .iter_mut()
        .find(|module| module.id == "battery")
        .unwrap()
        .urgent = true;
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let frame = renderer.render_bar(&state, &palette, &probe, 1920, 1);
    let battery = frame
        .elements
        .iter()
        .find(|element| element.id.as_deref() == Some("battery"))
        .unwrap();
    assert_eq!(battery.foreground, palette.accent.gold);
    assert_eq!(battery.background, Some((palette.accent.gold, 36)));
}

#[test]
fn chrome_uses_probe_fallbacks_and_dynamic_text_is_sanitized_before_shaping() {
    let palette = palette();
    let ascii = Probe::run(|ch| ch.is_ascii());
    let mut state = state();
    state.focused_title = "realm 🦀".into();
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let frame =
        renderer.render_bar_with_coverage(&state, &palette, &ascii, 1920, 1, |ch| ch.is_ascii());

    for element in frame.elements.iter().filter(|element| element.chrome) {
        assert!(
            element.text.is_ascii(),
            "chrome leaked a non-ASCII glyph: {}",
            element.text
        );
    }
    assert_eq!(frame.element(ElementRole::Title).unwrap().text, "realm ?");
    assert!(inventory().iter().any(|glyph| glyph.name == "elision"));
}

#[test]
fn which_key_preserves_order_and_elides_only_from_the_right() {
    let palette = palette();
    let keymap = Keymap::default();
    let probe = Probe::run(|_| true);
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let wide = renderer.render_which_key(&keymap, &palette, &probe, 1920, 1);
    let hints: Vec<_> = wide
        .elements
        .iter()
        .filter(|element| element.role == ElementRole::Hint)
        .map(|element| element.text.clone())
        .collect();
    assert_eq!(
        hints,
        keymap
            .strip()
            .map(|binding| format!("{} {}", binding.hint_key, binding.label))
            .collect::<Vec<_>>()
    );
    assert!(wide.element(ElementRole::GrimoirePrompt).is_some());
    let prompt = wide.element(ElementRole::GrimoirePrompt).unwrap();
    assert_eq!(prompt.foreground, palette.text.dim);
    assert_eq!(prompt.secondary_foreground, Some(palette.accent.gold));

    let narrow = renderer.render_which_key(&keymap, &palette, &probe, 640, 1);
    let narrow_hints: Vec<_> = narrow
        .elements
        .iter()
        .filter(|element| element.role == ElementRole::Hint)
        .map(|element| element.text.clone())
        .collect();
    assert_eq!(narrow_hints, hints[..narrow_hints.len()]);
    assert!(narrow.element(ElementRole::GrimoirePrompt).is_none());
    assert!(narrow.element(ElementRole::Elision).is_some());
}

#[test]
fn narrow_bar_keeps_critical_elements_and_drops_modules_in_priority_order() {
    let palette = palette();
    let probe = Probe::run(|_| true);
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let frame = renderer.render_bar(&state(), &palette, &probe, 1024, 1);
    assert_eq!(
        frame
            .elements
            .iter()
            .filter(|element| element.role == ElementRole::Orbit)
            .count(),
        6
    );
    assert!(frame.element(ElementRole::Mode).is_some());
    assert!(frame
        .elements
        .iter()
        .any(|element| element.id.as_deref() == Some("battery")));
    assert!(frame
        .elements
        .iter()
        .any(|element| element.id.as_deref() == Some("clock")));
    assert!(frame
        .element(ElementRole::Title)
        .unwrap()
        .text
        .contains('…'));
    let dropped = &frame.dropped_modules;
    assert!(dropped
        .windows(2)
        .all(|pair| ["gpu", "net", "mem", "vol", "cpu"]
            .iter()
            .position(|id| id == &pair[0])
            .unwrap()
            < ["gpu", "net", "mem", "vol", "cpu"]
                .iter()
                .position(|id| id == &pair[1])
                .unwrap()));
}

#[test]
fn grimoire_lists_only_nonempty_hints_in_binding_order_and_is_deterministic() {
    let palette = palette();
    let keymap = Keymap::default();
    let probe = Probe::run(|_| true);
    let mut one = BarRenderer::new(&palette).unwrap();
    let mut two = BarRenderer::new(&palette).unwrap();
    let left = one.render_grimoire(&keymap, &palette, &probe, 1920, 1080, 1);
    let right = two.render_grimoire(&keymap, &palette, &probe, 1920, 1080, 1);
    let rows: Vec<_> = left
        .elements
        .iter()
        .filter(|element| element.role == ElementRole::GrimoireBinding)
        .map(|element| element.id.clone().unwrap())
        .collect();
    let expected: Vec<_> = keymap
        .bindings
        .iter()
        .filter(|binding| !binding.hint_key.is_empty())
        .map(|binding| binding.key.clone())
        .collect();
    assert_eq!(rows, expected);
    assert_eq!(
        left.element(ElementRole::GrimoireDismiss).unwrap().text,
        "esc dismiss"
    );
    assert_eq!(left.pixels, right.pixels);
}

#[test]
fn scale_two_raster_has_exact_device_dimensions() {
    let palette = palette();
    let probe = Probe::run(|_| true);
    let mut renderer = BarRenderer::new(&palette).unwrap();
    let frame = renderer.render_bar(&state(), &palette, &probe, 640, 2);
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 32);
    assert_eq!(frame.scale, 2);
    assert_eq!(frame.pixels.len(), 1280 * 64 * 4);
    assert!(frame
        .elements
        .iter()
        .filter(|element| element.role == ElementRole::Orbit)
        .all(|element| element.rect.width == 28));
}
