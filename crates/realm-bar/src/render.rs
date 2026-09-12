use std::collections::{HashMap, HashSet};

use realm_core::{
    color::Rgb,
    glyphs::{inventory, Probe},
    keys::{Action, Binding, Keymap},
    palette::Palette,
    state::{OrbitDisplay, RealmState},
};
use tiny_skia::{Paint, Pixmap, Rect, Transform};

use crate::text::TextSystem;
use crate::{Damage, Element, ElementRole, Frame, LogicalRect, SurfaceKind};

const WASH_ALPHA: u8 = 36;
const HORIZONTAL_MODULE_GAP: u32 = 16;

#[derive(Clone)]
struct PreviousFrame {
    state: RealmState,
    palette: Palette,
    width: u32,
    height: u32,
    scale: u32,
    elements: Vec<Element>,
}

pub struct BarRenderer {
    text: TextSystem,
    previous_bar: Option<PreviousFrame>,
    pending_bar: Option<PreviousFrame>,
    unknown_accents: HashSet<String>,
}

impl BarRenderer {
    pub fn new(palette: &Palette) -> anyhow::Result<Self> {
        Ok(Self {
            text: TextSystem::new(palette)?,
            previous_bar: None,
            pending_bar: None,
            unknown_accents: HashSet::new(),
        })
    }

    pub fn probe(&self) -> Probe {
        self.text.probe()
    }

    pub fn font_summary(&self) -> String {
        self.probe().summary()
    }

    pub fn commit_bar_frame(&mut self) {
        if let Some(committed) = self.pending_bar.take() {
            self.previous_bar = Some(committed);
        }
    }

    pub fn render_bar(
        &mut self,
        state: &RealmState,
        palette: &Palette,
        probe: &Probe,
        width: u32,
        scale: u32,
    ) -> Frame {
        let mut sanitized = state.clone();
        sanitized.focused_title = self.text.sanitize(&state.focused_title);
        sanitized.chord_echo = self.text.sanitize(&state.chord_echo);
        for (module, original) in sanitized.modules.iter_mut().zip(&state.modules) {
            module.text = self.text.sanitize(&original.text);
        }
        self.render_bar_with_coverage(&sanitized, palette, probe, width, scale, |_| true)
    }

    pub fn render_bar_with_coverage(
        &mut self,
        state: &RealmState,
        palette: &Palette,
        probe: &Probe,
        width: u32,
        scale: u32,
        mut covers: impl FnMut(char) -> bool,
    ) -> Frame {
        self.text.begin_bar_frame();
        let height = palette.metrics.bar_height.max(1) as u32;
        let scale = scale.max(1);
        let mut dropped_modules = Vec::new();
        let mut visible_modules: Vec<_> = state.modules.iter().collect();
        let mut show_layout_label = true;
        let mut show_logo_text = true;

        let full_title = sanitize_with(&state.focused_title, &mut covers);
        let mut title = full_title.clone();
        let mut title_keep = full_title.chars().count();
        let chord = sanitize_with(&state.chord_echo, &mut covers);
        let minimum_title = middle_elide(&full_title, 8, probe);

        loop {
            let left = self.left_width(state, palette, probe, show_logo_text, show_layout_label);
            let right = self.modules_width(&visible_modules, palette);
            let centre = self.centre_width(&title, &chord, palette);
            if left.saturating_add(right).saturating_add(centre) <= width {
                break;
            }
            if title != minimum_title {
                title_keep = title_keep.saturating_sub(1).max(8);
                title = middle_elide(&full_title, title_keep, probe);
                continue;
            }
            if !chord.is_empty() && !title.is_empty() {
                title.clear();
                continue;
            }
            if let Some(id) = ["gpu", "net", "mem", "vol", "cpu"]
                .into_iter()
                .find(|id| visible_modules.iter().any(|module| module.id == *id))
            {
                visible_modules.retain(|module| module.id != id);
                dropped_modules.push(id.to_owned());
                continue;
            }
            if show_layout_label {
                show_layout_label = false;
                continue;
            }
            if show_logo_text {
                show_logo_text = false;
                continue;
            }
            break;
        }

        let mut elements = Vec::new();
        elements.push(element(
            ElementRole::Background,
            None,
            String::new(),
            LogicalRect::new(0, 0, width, height),
            palette.text.bright,
            None,
            None,
            Some((
                palette.border.bar_bottom,
                palette.metrics.border_width.max(1) as u32,
            )),
            palette.typography.size_body,
            false,
        ));

        let body = palette.typography.size_body;
        let meta = palette.typography.size_meta;
        let regular = palette.typography.weight_regular;
        let medium = palette.typography.weight_medium;
        let logo_glyph = glyph("logo", probe);
        let logo_text = if show_logo_text {
            format!("{logo_glyph} realm")
        } else {
            logo_glyph.to_string()
        };
        let logo_width = self.text.measure_chrome(&logo_text, body, medium) + 28;
        elements.push(element(
            ElementRole::Logo,
            None,
            logo_text,
            LogicalRect::new(14, 0, logo_width.saturating_sub(28), height),
            palette.accent.violet,
            None,
            None,
            None,
            body,
            true,
        ));
        let mut x = logo_width as i32 + 6;

        for orbit in &state.orbits {
            let display = orbit.display;
            let foreground = match display {
                OrbitDisplay::Active => palette.text.bright,
                OrbitDisplay::Occupied => palette.text.mid,
                OrbitDisplay::Empty => palette.text.faint,
            };
            let resolved: String = orbit.rune.chars().map(|ch| probe.resolve(ch)).collect();
            elements.push(element(
                ElementRole::Orbit,
                Some(orbit.number.to_string()),
                resolved,
                LogicalRect::new(x, 0, 28, height),
                foreground,
                None,
                (display == OrbitDisplay::Active).then_some((palette.accent.violet, WASH_ALPHA)),
                (display == OrbitDisplay::Active).then_some((palette.accent.violet, 2)),
                body,
                true,
            ));
            x += 28;
        }
        x += 6 + 12;

        let layout_glyph = glyph("layout indicator", probe);
        let layout_text = if show_layout_label {
            format!("{layout_glyph} {}", state.layout.label())
        } else {
            layout_glyph.to_string()
        };
        let layout_width = self.text.measure_chrome(&layout_text, body, regular) + 12;
        elements.push(element(
            ElementRole::Layout,
            None,
            layout_text,
            LogicalRect::new(x, 0, layout_width, height),
            palette.text.dim,
            None,
            None,
            None,
            body,
            true,
        ));
        x += layout_width as i32 + 10;

        let mode_text = format!("{} {}", glyph("mode badge", probe), state.mode.badge());
        let mode_width = self.text.measure_chrome(&mode_text, meta, medium) + 16;
        elements.push(element(
            ElementRole::Mode,
            None,
            mode_text,
            LogicalRect::new(x, 1, mode_width, height.saturating_sub(2)),
            palette.accent.starlight,
            None,
            Some((palette.accent.starlight, WASH_ALPHA)),
            None,
            meta,
            true,
        ));
        let left_end = x + mode_width as i32;

        let mut module_elements = Vec::new();
        let mut right = width as i32 - 16;
        for module in visible_modules.iter().rev() {
            let module_text = sanitize_with(&module.text, &mut covers);
            let module_width = self.text.measure(&module_text, meta, regular) + 8;
            right -= module_width as i32;
            let foreground = match module.accent.as_deref() {
                Some(name) => palette.accent.by_name(name).unwrap_or_else(|| {
                    if self.unknown_accents.insert(name.to_owned()) {
                        tracing::warn!(accent = name, "unknown module accent; using text.mid");
                    }
                    palette.text.mid
                }),
                None if module.id == "clock" => palette.text.bright,
                None => palette.text.mid,
            };
            module_elements.push(element(
                if module.id == "clock" {
                    ElementRole::ModuleClock
                } else {
                    ElementRole::Module
                },
                Some(module.id.clone()),
                module_text,
                LogicalRect::new(right, 0, module_width, height),
                foreground,
                None,
                module.urgent.then_some((foreground, WASH_ALPHA)),
                None,
                meta,
                false,
            ));
            right -= HORIZONTAL_MODULE_GAP as i32;
        }
        module_elements.reverse();

        let centre_left = left_end + 14;
        let centre_right = (right + HORIZONTAL_MODULE_GAP as i32 - 14).max(centre_left);
        let title_width = self.text.measure(&title, body, regular);
        let chord_width = self.text.measure(&chord, meta, regular);
        let content_width = title_width
            + if title_width > 0 && chord_width > 0 {
                14
            } else {
                0
            }
            + chord_width;
        let centre_x =
            centre_left + ((centre_right - centre_left - content_width as i32).max(0) / 2);
        if !title.is_empty() {
            elements.push(element(
                ElementRole::Title,
                None,
                title,
                LogicalRect::new(centre_x, 0, title_width, height),
                palette.text.soft,
                None,
                None,
                None,
                body,
                false,
            ));
        }
        if !chord.is_empty() {
            let chord_x = centre_x + title_width as i32 + if title_width > 0 { 14 } else { 0 };
            elements.push(element(
                ElementRole::Chord,
                None,
                chord,
                LogicalRect::new(chord_x, 0, chord_width, height),
                palette.text.faint,
                None,
                None,
                None,
                meta,
                false,
            ));
        }
        elements.extend(module_elements);

        let damage = self.bar_damage(state, palette, width, height, scale, &elements);
        self.text.begin_bar_frame();
        let pixels = self.paint_bar(&elements, palette, width, height, scale);
        self.text.finish_bar_frame();
        self.pending_bar = Some(PreviousFrame {
            state: state.clone(),
            palette: palette.clone(),
            width,
            height,
            scale,
            elements: elements.clone(),
        });
        Frame {
            kind: SurfaceKind::Bar,
            width,
            height,
            scale,
            pixels,
            damage,
            elements,
            dropped_modules,
        }
    }

    pub fn render_which_key(
        &mut self,
        keymap: &Keymap,
        palette: &Palette,
        probe: &Probe,
        width: u32,
        scale: u32,
    ) -> Frame {
        let height = palette.metrics.whichkey_height.max(1) as u32;
        let meta = palette.typography.size_meta;
        let regular = palette.typography.weight_regular;
        let modifier_text = format!("{} {}", glyph("modifier", probe), keymap.modifier);
        let modifier_width = self.text.measure_chrome(&modifier_text, meta, regular) + 24;
        let prompt = "? grimoire - full spellbook".to_owned();
        let prompt_width = self.text.measure_chrome(&prompt, meta, regular) + 12;
        let all_hints: Vec<String> = keymap
            .strip()
            .map(|binding| {
                format!(
                    "{} {}",
                    resolve_chrome(&binding.hint_key, probe),
                    binding.label
                )
            })
            .collect();
        let mut hints = all_hints.clone();
        let mut show_prompt = true;
        let elision = glyph("elision", probe).to_string();
        loop {
            let hints_width = hints
                .iter()
                .map(|hint| self.text.measure_chrome(hint, meta, regular) + 18)
                .sum::<u32>();
            let elision_width = if hints.len() < all_hints.len() {
                self.text.measure_chrome(&elision, meta, regular) + 18
            } else {
                0
            };
            let total = 12
                + modifier_width
                + 14
                + hints_width
                + elision_width
                + 12
                + if show_prompt { prompt_width } else { 0 };
            if total <= width || hints.is_empty() {
                break;
            }
            if show_prompt {
                show_prompt = false;
            } else {
                hints.pop();
            }
        }

        let mut elements = vec![element(
            ElementRole::Background,
            None,
            String::new(),
            LogicalRect::new(0, 0, width, height),
            palette.text.bright,
            None,
            Some((palette.background.bar_bot, 255)),
            None,
            meta,
            false,
        )];
        let mut x = 12;
        elements.push(element(
            ElementRole::Modifier,
            None,
            modifier_text,
            LogicalRect::new(x as i32, 0, modifier_width, height),
            palette.accent.violet,
            None,
            None,
            None,
            meta,
            true,
        ));
        x += modifier_width + 14;
        for hint in hints {
            let hint_width = self.text.measure_chrome(&hint, meta, regular);
            elements.push(element(
                ElementRole::Hint,
                None,
                hint,
                LogicalRect::new(x as i32, 0, hint_width, height),
                palette.text.bright,
                Some(palette.text.mid),
                None,
                None,
                meta,
                true,
            ));
            x += hint_width + 18;
        }
        if x < width
            && elements
                .iter()
                .filter(|element| element.role == ElementRole::Hint)
                .count()
                < all_hints.len()
        {
            let elision_width = self.text.measure_chrome(&elision, meta, regular);
            elements.push(element(
                ElementRole::Elision,
                None,
                elision,
                LogicalRect::new(x as i32, 0, elision_width, height),
                palette.text.mid,
                None,
                None,
                None,
                meta,
                true,
            ));
        }
        if show_prompt {
            elements.push(element(
                ElementRole::GrimoirePrompt,
                None,
                prompt,
                LogicalRect::new(
                    (width.saturating_sub(prompt_width)) as i32,
                    0,
                    prompt_width,
                    height,
                ),
                palette.text.dim,
                Some(palette.accent.gold),
                None,
                None,
                meta,
                true,
            ));
        }
        let pixels = self.paint_flat(&elements, palette, width, height, scale.max(1));
        Frame {
            kind: SurfaceKind::WhichKey,
            width,
            height,
            scale: scale.max(1),
            pixels,
            damage: Damage::from_rect(LogicalRect::new(0, 0, width, height)),
            elements,
            dropped_modules: Vec::new(),
        }
    }

    pub fn render_grimoire(
        &mut self,
        keymap: &Keymap,
        palette: &Palette,
        probe: &Probe,
        width: u32,
        height: u32,
        scale: u32,
    ) -> Frame {
        let panel_width =
            (palette.metrics.portal_width.max(1) as u32).min(width.saturating_sub(24).max(1));
        let panel_x = (width.saturating_sub(panel_width) / 2) as i32;
        let panel_y = palette.metrics.bar_height.saturating_mul(2).max(0);
        let header_height = palette.metrics.header_height.max(1) as u32;
        let row_height = 25u32;
        let bindings: Vec<&Binding> = keymap
            .bindings
            .iter()
            .filter(|binding| !binding.hint_key.is_empty())
            .collect();
        let mut modes = Vec::new();
        for binding in &bindings {
            if !modes.contains(&binding.mode) {
                modes.push(binding.mode);
            }
        }
        let body_entries = bindings.len() + modes.len();
        let rows = body_entries.div_ceil(2) as u32;
        let max_panel_height =
            height.saturating_sub((palette.metrics.bar_height.max(0) as u32).saturating_mul(4));
        let panel_height =
            (header_height + rows * row_height + 28).min(max_panel_height.max(header_height));
        let body = palette.typography.size_body;
        let micro = palette.typography.size_micro;
        let meta = palette.typography.size_meta;
        let mut elements = vec![element(
            ElementRole::Background,
            None,
            String::new(),
            LogicalRect::new(0, 0, width, height),
            palette.text.bright,
            None,
            Some((palette.background.void, 153)),
            None,
            body,
            false,
        )];
        elements.push(element(
            ElementRole::GrimoirePanel,
            None,
            String::new(),
            LogicalRect::new(panel_x, panel_y, panel_width, panel_height),
            palette.text.normal,
            None,
            Some((palette.background.pane, 255)),
            None,
            body,
            false,
        ));
        elements.push(element(
            ElementRole::GrimoireHeader,
            None,
            format!("{} GRIMOIRE", glyph("logo", probe)),
            LogicalRect::new(
                panel_x + 16,
                panel_y,
                panel_width.saturating_sub(32),
                header_height,
            ),
            palette.accent.violet,
            None,
            None,
            Some((palette.border.seam, 1)),
            body,
            true,
        ));
        let dismiss = "esc dismiss".to_owned();
        let dismiss_width =
            self.text
                .measure_chrome(&dismiss, micro, palette.typography.weight_regular);
        elements.push(element(
            ElementRole::GrimoireDismiss,
            None,
            dismiss,
            LogicalRect::new(
                panel_x + panel_width as i32 - 16 - dismiss_width as i32,
                panel_y,
                dismiss_width,
                header_height,
            ),
            palette.text.faint,
            None,
            None,
            None,
            micro,
            true,
        ));

        let column_width = panel_width / 2;
        let mut entry_index = 0usize;
        for (mode_index, mode) in modes.into_iter().enumerate() {
            let (column, row) = grid_position(entry_index, rows);
            elements.push(element(
                ElementRole::GrimoireMode,
                Some(format!("mode-{mode_index}")),
                mode.badge().to_owned(),
                LogicalRect::new(
                    panel_x + 16 + (column * column_width) as i32,
                    panel_y + header_height as i32 + (row * row_height) as i32,
                    column_width.saturating_sub(32),
                    row_height,
                ),
                palette.text.dim,
                None,
                None,
                None,
                micro,
                true,
            ));
            entry_index += 1;

            for binding in bindings
                .iter()
                .copied()
                .filter(|binding| binding.mode == mode)
            {
                let (column, row) = grid_position(entry_index, rows);
                let bx = panel_x + (column * column_width) as i32 + 16;
                let by = panel_y + header_height as i32 + (row * row_height) as i32;
                let key = format!(
                    "{} {} {}",
                    glyph("modifier", probe),
                    keymap.modifier,
                    resolve_chrome(&binding.hint_key, probe)
                );
                let key_width =
                    self.text
                        .measure_chrome(&key, meta, palette.typography.weight_regular);
                let label_width = self.text.measure_chrome(
                    &binding.label,
                    meta,
                    palette.typography.weight_regular,
                );
                let id = Some(binding.key.clone());
                elements.push(element(
                    ElementRole::GrimoireBinding,
                    id.clone(),
                    key,
                    LogicalRect::new(bx, by, key_width, row_height),
                    palette.text.bright,
                    None,
                    None,
                    None,
                    meta,
                    true,
                ));
                elements.push(element(
                    ElementRole::GrimoireBinding,
                    id.clone(),
                    binding.label.clone(),
                    LogicalRect::new(bx + key_width as i32 + 10, by, label_width, row_height),
                    palette.text.mid,
                    None,
                    None,
                    None,
                    meta,
                    true,
                ));
                elements.push(element(
                    ElementRole::GrimoireBinding,
                    id,
                    action_label(&binding.action),
                    LogicalRect::new(
                        bx + key_width as i32 + label_width as i32 + 20,
                        by,
                        column_width
                            .saturating_sub(52)
                            .saturating_sub(key_width)
                            .saturating_sub(label_width),
                        row_height,
                    ),
                    palette.text.dim,
                    None,
                    None,
                    None,
                    micro,
                    true,
                ));
                entry_index += 1;
            }
        }
        elements.push(element(
            ElementRole::GrimoireFooter,
            None,
            format!("{} bindings - ? or esc dismiss", bindings.len()),
            LogicalRect::new(
                panel_x + 16,
                panel_y + panel_height as i32 - 28,
                panel_width.saturating_sub(32),
                24,
            ),
            palette.text.faint,
            None,
            None,
            None,
            micro,
            true,
        ));
        let pixels = self.paint_flat(&elements, palette, width, height, scale.max(1));
        Frame {
            kind: SurfaceKind::Grimoire,
            width,
            height,
            scale: scale.max(1),
            pixels,
            damage: Damage::from_rect(LogicalRect::new(0, 0, width, height)),
            elements,
            dropped_modules: Vec::new(),
        }
    }

    fn left_width(
        &mut self,
        state: &RealmState,
        palette: &Palette,
        probe: &Probe,
        logo: bool,
        layout: bool,
    ) -> u32 {
        let body = palette.typography.size_body;
        let meta = palette.typography.size_meta;
        let logo_text = if logo {
            format!("{} realm", glyph("logo", probe))
        } else {
            glyph("logo", probe).to_string()
        };
        let layout_text = if layout {
            format!(
                "{} {}",
                glyph("layout indicator", probe),
                state.layout.label()
            )
        } else {
            glyph("layout indicator", probe).to_string()
        };
        let mode = format!("{} {}", glyph("mode badge", probe), state.mode.badge());
        self.text
            .measure_chrome(&logo_text, body, palette.typography.weight_medium)
            + 28
            + 6
            + (28 * state.orbits.len() as u32)
            + 6
            + 12
            + self
                .text
                .measure_chrome(&layout_text, body, palette.typography.weight_regular)
            + 12
            + 10
            + self
                .text
                .measure_chrome(&mode, meta, palette.typography.weight_medium)
            + 16
            + 14
    }

    fn modules_width(&mut self, modules: &[&realm_core::state::Module], palette: &Palette) -> u32 {
        if modules.is_empty() {
            return 16;
        }
        modules
            .iter()
            .map(|module| {
                self.text.measure(
                    &module.text,
                    palette.typography.size_meta,
                    palette.typography.weight_regular,
                ) + 8
            })
            .sum::<u32>()
            + HORIZONTAL_MODULE_GAP * (modules.len().saturating_sub(1) as u32)
            + 16
    }

    fn centre_width(&mut self, title: &str, chord: &str, palette: &Palette) -> u32 {
        let title_width = self.text.measure(
            title,
            palette.typography.size_body,
            palette.typography.weight_regular,
        );
        let chord_width = self.text.measure(
            chord,
            palette.typography.size_meta,
            palette.typography.weight_regular,
        );
        title_width
            + chord_width
            + if title_width > 0 && chord_width > 0 {
                14
            } else {
                0
            }
    }

    fn bar_damage(
        &self,
        state: &RealmState,
        palette: &Palette,
        width: u32,
        height: u32,
        scale: u32,
        elements: &[Element],
    ) -> Damage {
        let Some(previous) = &self.previous_bar else {
            return Damage::from_rect(LogicalRect::new(0, 0, width, height));
        };
        if previous.width != width
            || previous.height != height
            || previous.scale != scale
            || previous.palette != *palette
        {
            return Damage::from_rect(LogicalRect::new(0, 0, width, height));
        }
        if previous.state.renders_same_as(state) {
            return Damage::none();
        }
        let old = keyed(&previous.elements);
        let new = keyed(elements);
        let mut damage: Option<LogicalRect> = None;
        for key in old.keys().chain(new.keys()) {
            let before = old.get(key);
            let after = new.get(key);
            if before == after {
                continue;
            }
            for rect in [
                before.map(|element| element.rect),
                after.map(|element| element.rect),
            ]
            .into_iter()
            .flatten()
            {
                damage = Some(damage.map_or(rect, |current| current.union(rect)));
            }
        }
        damage.map_or_else(Damage::none, Damage::from_rect)
    }

    fn paint_bar(
        &mut self,
        elements: &[Element],
        palette: &Palette,
        width: u32,
        height: u32,
        scale: u32,
    ) -> Vec<u8> {
        let device_width = width.saturating_mul(scale).max(1);
        let device_height = height.saturating_mul(scale).max(1);
        let mut pixmap = Pixmap::new(device_width, device_height).expect("valid bar pixmap");
        vertical_gradient(
            &mut pixmap,
            palette.background.bar_top,
            palette.background.bar_bot,
        );
        for (x_percent, y_percent, alpha) in
            [(18, 40, 153), (43, 70, 102), (67, 30, 128), (88, 60, 89)]
        {
            let x = device_width.saturating_mul(x_percent) / 100;
            let y = device_height.saturating_mul(y_percent) / 100;
            fill_device_rect(
                &mut pixmap,
                LogicalRect::new(x as i32, y as i32, scale, scale),
                palette.text.bright,
                alpha,
            );
        }
        self.paint_elements(&mut pixmap, elements, palette, scale);
        pixmap.data().to_vec()
    }

    fn paint_flat(
        &mut self,
        elements: &[Element],
        palette: &Palette,
        width: u32,
        height: u32,
        scale: u32,
    ) -> Vec<u8> {
        let mut pixmap = Pixmap::new(
            width.saturating_mul(scale).max(1),
            height.saturating_mul(scale).max(1),
        )
        .expect("valid surface pixmap");
        self.paint_elements(&mut pixmap, elements, palette, scale);
        pixmap.data().to_vec()
    }

    fn paint_elements(
        &mut self,
        pixmap: &mut Pixmap,
        elements: &[Element],
        palette: &Palette,
        scale: u32,
    ) {
        for item in elements {
            if let Some((background, alpha)) = item.background {
                fill_device_rect(pixmap, item.rect.scaled(scale), background, alpha);
            }
            if item.role == ElementRole::Background
                && item.background.map(|(color, _)| color) == Some(palette.background.bar_bot)
            {
                let rect = item.rect.scaled(scale);
                fill_device_rect(
                    pixmap,
                    LogicalRect::new(rect.x, rect.y, rect.width, scale.max(1)),
                    palette.border.seam,
                    (palette.border.seam_alpha * 255.0).round() as u8,
                );
            }
            if item.role == ElementRole::Background && item.background.is_none() {
                continue;
            }
            if item.role == ElementRole::GrimoirePanel {
                stroke_device_rect(
                    pixmap,
                    item.rect.scaled(scale),
                    palette.border.focused,
                    (palette.border.focused_alpha * 255.0).round() as u8,
                    scale.max(1),
                );
            }
            if !item.text.is_empty() {
                let rect = item.rect.scaled(scale);
                let measured = if item.chrome {
                    self.text.measure_chrome(
                        &item.text,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                    )
                } else {
                    self.text.measure(
                        &item.text,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                    )
                };
                let text_x = if item.role == ElementRole::Orbit || item.role == ElementRole::Mode {
                    rect.x + ((rect.width.saturating_sub(measured)) / 2) as i32
                } else {
                    rect.x
                };
                if item.role == ElementRole::Orbit && item.background.is_some() {
                    let glow = palette
                        .accent
                        .violet
                        .flatten_over(palette.background.bar_top, 0.35);
                    for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        self.text.draw(
                            pixmap,
                            &item.text,
                            text_x + dx * scale as i32,
                            rect.y + dy * scale as i32,
                            rect.height,
                            item.font_size * scale as f32,
                            palette.typography.weight_regular,
                            glow,
                            item.chrome,
                        );
                    }
                }
                if item.role == ElementRole::Hint {
                    if let Some((key, label)) = item.text.split_once(' ') {
                        self.text.draw(
                            pixmap,
                            key,
                            text_x,
                            rect.y,
                            rect.height,
                            item.font_size * scale as f32,
                            palette.typography.weight_regular,
                            item.foreground,
                            item.chrome,
                        );
                        let key_width = self.text.measure_chrome(
                            key,
                            item.font_size * scale as f32,
                            palette.typography.weight_regular,
                        );
                        self.text.draw(
                            pixmap,
                            label,
                            text_x + key_width as i32 + (scale as i32 * 5),
                            rect.y,
                            rect.height,
                            item.font_size * scale as f32,
                            palette.typography.weight_regular,
                            item.secondary_foreground.unwrap_or(item.foreground),
                            item.chrome,
                        );
                    }
                } else if item.role == ElementRole::GrimoirePrompt {
                    let (lead, rest) = item.text.split_at(1);
                    self.text.draw(
                        pixmap,
                        lead,
                        text_x,
                        rect.y,
                        rect.height,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                        item.secondary_foreground.unwrap_or(item.foreground),
                        item.chrome,
                    );
                    let lead_width = self.text.measure_chrome(
                        lead,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                    );
                    self.text.draw(
                        pixmap,
                        rest,
                        text_x + lead_width as i32,
                        rect.y,
                        rect.height,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                        item.foreground,
                        item.chrome,
                    );
                } else {
                    self.text.draw(
                        pixmap,
                        &item.text,
                        text_x,
                        rect.y,
                        rect.height,
                        item.font_size * scale as f32,
                        palette.typography.weight_regular,
                        item.foreground,
                        item.chrome,
                    );
                }
            }
            if let Some((color, thickness)) = item.bottom_rule {
                let rect = item.rect.scaled(scale);
                let alpha = match item.role {
                    ElementRole::Background => {
                        (palette.border.bar_bottom_alpha * 255.0).round() as u8
                    }
                    ElementRole::GrimoireHeader => {
                        (palette.border.seam_alpha * 255.0).round() as u8
                    }
                    _ => 255,
                };
                fill_device_rect(
                    pixmap,
                    LogicalRect::new(
                        rect.x,
                        rect.y + rect.height as i32 - (thickness * scale) as i32,
                        rect.width,
                        thickness * scale,
                    ),
                    color,
                    alpha,
                );
            }
            if item.role == ElementRole::Logo {
                let rect = item.rect.scaled(scale);
                fill_device_rect(
                    pixmap,
                    LogicalRect::new(
                        rect.x + rect.width as i32 + 13 * scale as i32,
                        rect.y,
                        scale.max(1),
                        rect.height,
                    ),
                    palette.border.seam,
                    (palette.border.seam_alpha * 255.0).round() as u8,
                );
            }
            if item.role == ElementRole::Layout {
                let rect = item.rect.scaled(scale);
                fill_device_rect(
                    pixmap,
                    LogicalRect::new(
                        rect.x - 12 * scale as i32,
                        rect.y,
                        scale.max(1),
                        rect.height,
                    ),
                    palette.border.neutral,
                    (palette.border.neutral_alpha * 255.0).round() as u8,
                );
            }
            if item.role == ElementRole::Modifier {
                let rect = item.rect.scaled(scale);
                fill_device_rect(
                    pixmap,
                    LogicalRect::new(
                        rect.x + rect.width as i32 - scale as i32,
                        rect.y,
                        scale.max(1),
                        rect.height,
                    ),
                    palette.border.neutral,
                    (palette.border.neutral_alpha * 255.0).round() as u8,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn element(
    role: ElementRole,
    id: Option<String>,
    text: String,
    rect: LogicalRect,
    foreground: Rgb,
    secondary_foreground: Option<Rgb>,
    background: Option<(Rgb, u8)>,
    bottom_rule: Option<(Rgb, u32)>,
    font_size: f32,
    chrome: bool,
) -> Element {
    Element {
        role,
        id,
        text,
        rect,
        foreground,
        secondary_foreground,
        background,
        bottom_rule,
        font_size,
        chrome,
    }
}

fn keyed(elements: &[Element]) -> HashMap<(ElementRole, Option<String>, usize), &Element> {
    let mut counts = HashMap::new();
    let mut map = HashMap::new();
    for element in elements {
        let base = (element.role, element.id.clone());
        let count = counts.entry(base.clone()).or_insert(0usize);
        map.insert((base.0, base.1, *count), element);
        *count += 1;
    }
    map
}

fn glyph(name: &str, probe: &Probe) -> char {
    let glyph = inventory()
        .into_iter()
        .find(|glyph| glyph.name == name)
        .unwrap_or_else(|| panic!("missing glyph inventory entry: {name}"));
    probe.resolve(glyph.ch)
}

fn resolve_chrome(text: &str, probe: &Probe) -> String {
    text.chars()
        .map(|ch| if ch.is_ascii() { ch } else { probe.resolve(ch) })
        .collect()
}

fn sanitize_with(text: &str, covers: &mut impl FnMut(char) -> bool) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_control() || covers(ch) {
                ch
            } else {
                '?'
            }
        })
        .collect()
}

fn middle_elide(text: &str, floor: usize, probe: &Probe) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= floor {
        return text.to_owned();
    }
    let left = floor.div_ceil(2);
    let right = floor / 2;
    let mut result: String = chars[..left].iter().collect();
    result.push(glyph("elision", probe));
    result.extend(chars[chars.len() - right..].iter());
    result
}

fn action_label(action: &Action) -> String {
    match action {
        Action::Spawn(argv) => format!("spawn {}", argv.join(" ")),
        Action::Launcher => "launcher".into(),
        Action::Focus(direction) => format!("focus {direction:?}").to_lowercase(),
        Action::Swap(direction) => format!("swap {direction:?}").to_lowercase(),
        Action::Orbit(number) => format!("orbit {number}"),
        Action::MoveToOrbit(number) => format!("move-to-orbit {number}"),
        Action::Stow => "stow".into(),
        Action::SetLayout(layout) => format!("set-layout {}", layout.label()),
        Action::Fullscreen => "fullscreen".into(),
        Action::EnterMode(mode) => format!("mode {}", mode.badge().to_lowercase()),
        Action::Banish => "banish".into(),
        Action::Undo => "undo".into(),
        Action::ToggleWhichKey => "toggle-which-key".into(),
        Action::Grimoire => "grimoire".into(),
        Action::ReloadTheme => "theme".into(),
        Action::Quit => "quit".into(),
    }
}

fn grid_position(index: usize, rows: u32) -> (u32, u32) {
    let rows = rows.max(1);
    let index = index as u32;
    (index / rows, index % rows)
}

fn vertical_gradient(pixmap: &mut Pixmap, top: Rgb, bottom: Rgb) {
    let height = pixmap.height().max(1);
    let width = pixmap.width();
    let data = pixmap.data_mut();
    for y in 0..height {
        let mix = if height == 1 {
            0.0
        } else {
            y as f32 / (height - 1) as f32
        };
        let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * mix).round() as u8;
        let rgba = [
            channel(top.r, bottom.r),
            channel(top.g, bottom.g),
            channel(top.b, bottom.b),
            255,
        ];
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            data[offset..offset + 4].copy_from_slice(&rgba);
        }
    }
}

fn fill_device_rect(pixmap: &mut Pixmap, rect: LogicalRect, color: Rgb, alpha: u8) {
    let Some(rect) = Rect::from_xywh(
        rect.x as f32,
        rect.y as f32,
        rect.width as f32,
        rect.height as f32,
    ) else {
        return;
    };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, alpha);
    pixmap.fill_rect(rect, &paint, Transform::identity(), None);
}

fn stroke_device_rect(
    pixmap: &mut Pixmap,
    rect: LogicalRect,
    color: Rgb,
    alpha: u8,
    thickness: u32,
) {
    fill_device_rect(
        pixmap,
        LogicalRect::new(rect.x, rect.y, rect.width, thickness),
        color,
        alpha,
    );
    fill_device_rect(
        pixmap,
        LogicalRect::new(
            rect.x,
            rect.y + rect.height as i32 - thickness as i32,
            rect.width,
            thickness,
        ),
        color,
        alpha,
    );
    fill_device_rect(
        pixmap,
        LogicalRect::new(rect.x, rect.y, thickness, rect.height),
        color,
        alpha,
    );
    fill_device_rect(
        pixmap,
        LogicalRect::new(
            rect.x + rect.width as i32 - thickness as i32,
            rect.y,
            thickness,
            rect.height,
        ),
        color,
        alpha,
    );
}
