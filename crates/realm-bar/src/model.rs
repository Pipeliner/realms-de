use std::ops::BitOr;

use realm_core::state::RealmState;
use realm_core::{color::Rgb, palette::Palette};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceKind {
    Bar,
    WhichKey,
    Grimoire,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layer {
    Bottom,
    Overlay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Anchor(u8);

impl Anchor {
    pub const TOP: Self = Self(1);
    pub const BOTTOM: Self = Self(2);
    pub const LEFT: Self = Self(4);
    pub const RIGHT: Self = Self(8);

    pub const fn all() -> Self {
        Self(Self::TOP.0 | Self::BOTTOM.0 | Self::LEFT.0 | Self::RIGHT.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Anchor {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceConfig {
    pub kind: SurfaceKind,
    pub layer: Layer,
    pub anchors: Anchor,
    pub logical_size: (u32, u32),
    pub buffer_size: (u32, u32),
    pub exclusive_zone: i32,
    pub keyboard_interactive: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceCommand {
    Create(SurfaceKind),
    Destroy(SurfaceKind),
    Draw(SurfaceKind),
}

pub struct SurfaceLifecycle {
    width: u32,
    height: u32,
    scale: u32,
    bar: bool,
    which_key: bool,
    grimoire: bool,
    last_state: Option<RealmState>,
}

impl SurfaceLifecycle {
    pub fn new(width: u32, height: u32, scale: u32) -> Self {
        Self {
            width,
            height,
            scale: scale.max(1),
            bar: false,
            which_key: false,
            grimoire: false,
            last_state: None,
        }
    }

    pub fn update(&mut self, state: &RealmState, _palette: &Palette) -> Vec<SurfaceCommand> {
        if self
            .last_state
            .as_ref()
            .is_some_and(|last| last.renders_same_as(state))
        {
            return Vec::new();
        }

        let mut commands = Vec::new();
        if !self.bar {
            self.bar = true;
            commands.push(SurfaceCommand::Create(SurfaceKind::Bar));
        }
        let which_key_changed = self.which_key != state.whichkey;
        let grimoire_changed = self.grimoire != state.grimoire;
        reconcile_visibility(
            &mut self.which_key,
            state.whichkey,
            SurfaceKind::WhichKey,
            &mut commands,
        );
        reconcile_visibility(
            &mut self.grimoire,
            state.grimoire,
            SurfaceKind::Grimoire,
            &mut commands,
        );
        commands.push(SurfaceCommand::Draw(SurfaceKind::Bar));
        if self.which_key && which_key_changed {
            commands.push(SurfaceCommand::Draw(SurfaceKind::WhichKey));
        }
        if self.grimoire && grimoire_changed {
            commands.push(SurfaceCommand::Draw(SurfaceKind::Grimoire));
        }
        self.last_state = Some(state.clone());
        commands
    }

    pub fn configure_output(&mut self, width: u32, height: u32, scale: u32) -> Vec<SurfaceCommand> {
        let scale = scale.max(1);
        if (self.width, self.height, self.scale) == (width, height, scale) {
            return Vec::new();
        }
        self.width = width;
        self.height = height;
        self.scale = scale;
        [
            (self.bar, SurfaceKind::Bar),
            (self.which_key, SurfaceKind::WhichKey),
            (self.grimoire, SurfaceKind::Grimoire),
        ]
        .into_iter()
        .filter_map(|(mapped, kind)| mapped.then_some(SurfaceCommand::Draw(kind)))
        .collect()
    }

    pub fn config(&self, kind: SurfaceKind, palette: &Palette) -> SurfaceConfig {
        surface_config(kind, palette, self.width, self.height, self.scale)
    }
}

fn reconcile_visibility(
    mapped: &mut bool,
    visible: bool,
    kind: SurfaceKind,
    commands: &mut Vec<SurfaceCommand>,
) {
    if *mapped == visible {
        return;
    }
    *mapped = visible;
    commands.push(if visible {
        SurfaceCommand::Create(kind)
    } else {
        SurfaceCommand::Destroy(kind)
    });
}

pub fn surface_config(
    kind: SurfaceKind,
    palette: &Palette,
    output_width: u32,
    output_height: u32,
    scale: u32,
) -> SurfaceConfig {
    let scale = scale.max(1);
    let (layer, anchors, logical_size, exclusive_zone) = match kind {
        SurfaceKind::Bar => (
            Layer::Bottom,
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            (0, palette.metrics.bar_height.max(1) as u32),
            palette.metrics.bar_height,
        ),
        SurfaceKind::WhichKey => (
            Layer::Bottom,
            Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            (0, palette.metrics.whichkey_height.max(1) as u32),
            palette.metrics.whichkey_height,
        ),
        SurfaceKind::Grimoire => (Layer::Overlay, Anchor::all(), (0, 0), -1),
    };
    let logical_width = if logical_size.0 == 0 {
        output_width
    } else {
        logical_size.0
    };
    let logical_height = if logical_size.1 == 0 {
        output_height
    } else {
        logical_size.1
    };
    SurfaceConfig {
        kind,
        layer,
        anchors,
        logical_size,
        buffer_size: (
            logical_width.saturating_mul(scale),
            logical_height.saturating_mul(scale),
        ),
        exclusive_zone,
        keyboard_interactive: false,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LogicalRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl LogicalRect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn union(self, other: Self) -> Self {
        let left = self.x.min(other.x);
        let top = self.y.min(other.y);
        let right = (self.x + self.width as i32).max(other.x + other.width as i32);
        let bottom = (self.y + self.height as i32).max(other.y + other.height as i32);
        Self::new(left, top, (right - left) as u32, (bottom - top) as u32)
    }

    pub fn scaled(self, scale: u32) -> Self {
        let scale = scale.max(1);
        Self::new(
            self.x.saturating_mul(scale as i32),
            self.y.saturating_mul(scale as i32),
            self.width.saturating_mul(scale),
            self.height.saturating_mul(scale),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Damage(Option<LogicalRect>);

impl Damage {
    pub const fn none() -> Self {
        Self(None)
    }

    pub const fn from_rect(rect: LogicalRect) -> Self {
        Self(Some(rect))
    }

    pub const fn rect(self) -> Option<LogicalRect> {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ElementRole {
    Background,
    Logo,
    Orbit,
    Layout,
    Mode,
    Title,
    Chord,
    Module,
    ModuleClock,
    Modifier,
    Hint,
    Elision,
    GrimoirePrompt,
    GrimoirePanel,
    GrimoireHeader,
    GrimoireDismiss,
    GrimoireMode,
    GrimoireBinding,
    GrimoireFooter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub role: ElementRole,
    pub id: Option<String>,
    pub text: String,
    pub rect: LogicalRect,
    pub foreground: Rgb,
    pub secondary_foreground: Option<Rgb>,
    pub background: Option<(Rgb, u8)>,
    pub bottom_rule: Option<(Rgb, u32)>,
    pub font_size: f32,
    pub chrome: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub kind: SurfaceKind,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub pixels: Vec<u8>,
    pub damage: Damage,
    pub elements: Vec<Element>,
    pub dropped_modules: Vec<String>,
}

impl Frame {
    pub fn element(&self, role: ElementRole) -> Option<&Element> {
        self.elements.iter().find(|element| element.role == role)
    }
}
