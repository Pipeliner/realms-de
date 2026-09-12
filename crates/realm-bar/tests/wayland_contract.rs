use realm_bar::{SurfaceCommand, SurfaceKind, SurfaceLifecycle};
use realm_core::{palette::Palette, state::RealmState};

fn palette() -> Palette {
    Palette::from_toml(include_str!("../../../palette.toml")).unwrap()
}

#[test]
fn visibility_changes_create_and_destroy_only_state_driven_surfaces() {
    let palette = palette();
    let mut lifecycle = SurfaceLifecycle::new(1920, 1080, 1);
    let shown = RealmState::default();
    assert_eq!(
        lifecycle.update(&shown, &palette),
        vec![
            SurfaceCommand::Create(SurfaceKind::Bar),
            SurfaceCommand::Create(SurfaceKind::WhichKey),
            SurfaceCommand::Draw(SurfaceKind::Bar),
            SurfaceCommand::Draw(SurfaceKind::WhichKey),
        ]
    );

    let mut hidden = shown.clone();
    hidden.whichkey = false;
    assert_eq!(
        lifecycle.update(&hidden, &palette),
        vec![
            SurfaceCommand::Destroy(SurfaceKind::WhichKey),
            SurfaceCommand::Draw(SurfaceKind::Bar),
        ]
    );

    hidden.grimoire = true;
    assert_eq!(
        lifecycle.update(&hidden, &palette),
        vec![
            SurfaceCommand::Create(SurfaceKind::Grimoire),
            SurfaceCommand::Draw(SurfaceKind::Bar),
            SurfaceCommand::Draw(SurfaceKind::Grimoire),
        ]
    );
}

#[test]
fn revision_only_updates_produce_no_surface_work() {
    let palette = palette();
    let mut lifecycle = SurfaceLifecycle::new(1920, 1080, 1);
    let state = RealmState::default();
    lifecycle.update(&state, &palette);
    let mut revision = state.clone();
    revision.revision += 1;
    assert!(lifecycle.update(&revision, &palette).is_empty());
}

#[test]
fn module_updates_do_not_redraw_static_key_discovery_surfaces() {
    let palette = palette();
    let mut lifecycle = SurfaceLifecycle::new(1920, 1080, 1);
    let state = RealmState::default();
    lifecycle.update(&state, &palette);

    let mut sampled = state.clone();
    sampled.modules.push(realm_core::state::Module {
        id: "cpu".into(),
        text: "cpu 31%".into(),
        accent: Some("starlight".into()),
        urgent: false,
    });
    assert_eq!(
        lifecycle.update(&sampled, &palette),
        vec![SurfaceCommand::Draw(SurfaceKind::Bar)]
    );
}

#[test]
fn output_scale_change_redraws_mapped_surfaces_without_changing_exclusive_zones() {
    let palette = palette();
    let mut lifecycle = SurfaceLifecycle::new(1920, 1080, 1);
    let state = RealmState::default();
    lifecycle.update(&state, &palette);
    let commands = lifecycle.configure_output(1920, 1080, 2);
    assert_eq!(
        commands,
        vec![
            SurfaceCommand::Draw(SurfaceKind::Bar),
            SurfaceCommand::Draw(SurfaceKind::WhichKey),
        ]
    );
    assert_eq!(
        lifecycle.config(SurfaceKind::Bar, &palette).exclusive_zone,
        32
    );
    assert_eq!(
        lifecycle
            .config(SurfaceKind::WhichKey, &palette)
            .exclusive_zone,
        26
    );
    assert_eq!(
        lifecycle.config(SurfaceKind::Bar, &palette).buffer_size,
        (3840, 64)
    );
}
