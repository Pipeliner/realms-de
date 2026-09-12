use std::collections::{HashMap, HashSet};

use cosmic_text::{
    fontdb::{self, Database, Family as DbFamily, Query},
    Attrs, Buffer, Color, Fallback, Family, FontSystem, Metrics, Shaping, Stretch, Style,
    SwashCache, Weight, Wrap,
};
use realm_core::{color::Rgb, glyphs::Probe, palette::Palette};
use tiny_skia::Pixmap;

#[derive(Clone)]
struct OrderedFallback {
    families: &'static [&'static str],
}

impl Fallback for OrderedFallback {
    fn common_fallback(&self) -> &[&'static str] {
        self.families
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: unicode_script::Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

pub(crate) struct TextSystem {
    font_system: FontSystem,
    cache: SwashCache,
    family: String,
    coverage_faces: Vec<fontdb::ID>,
    line_height: f32,
    chrome_buffers: HashMap<TextKey, Buffer>,
    dynamic_buffers: HashMap<TextKey, Buffer>,
    dynamic_used: HashSet<TextKey>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextKey {
    text: String,
    size_bits: u32,
    weight: u16,
}

impl TextSystem {
    pub(crate) fn new(palette: &Palette) -> anyhow::Result<Self> {
        let mut db = Database::new();
        db.load_system_fonts();

        let mut chain = Vec::with_capacity(palette.typography.fallback.len() + 1);
        chain.push(palette.typography.family.clone());
        for family in &palette.typography.fallback {
            if !chain.contains(family) {
                chain.push(family.clone());
            }
        }

        let coverage_faces = resolved_faces(&db, &chain);
        let leaked: &'static [&'static str] = Box::leak(
            chain
                .iter()
                .cloned()
                .map(|family| Box::leak(family.into_boxed_str()) as &'static str)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let font_system = FontSystem::new_with_locale_and_db_and_fallback(
            "en-US".to_owned(),
            db,
            OrderedFallback { families: leaked },
        );
        Ok(Self {
            font_system,
            cache: SwashCache::new(),
            family: palette.typography.family.clone(),
            coverage_faces,
            line_height: palette.typography.line_height,
            chrome_buffers: HashMap::new(),
            dynamic_buffers: HashMap::new(),
            dynamic_used: HashSet::new(),
        })
    }

    pub(crate) fn probe(&self) -> Probe {
        Probe::run(|ch| self.covers(ch))
    }

    pub(crate) fn covers(&self, ch: char) -> bool {
        self.coverage_faces.iter().any(|id| {
            self.font_system
                .db()
                .with_face_data(*id, |data, face_index| {
                    ttf_parser::Face::parse(data, face_index)
                        .ok()
                        .and_then(|face| face.glyph_index(ch))
                        .is_some()
                })
                .unwrap_or(false)
        })
    }

    pub(crate) fn sanitize(&self, text: &str) -> String {
        text.chars()
            .map(|ch| {
                if ch.is_control() || self.covers(ch) {
                    ch
                } else {
                    '?'
                }
            })
            .collect()
    }

    pub(crate) fn measure(&mut self, text: &str, size: f32, weight: u16) -> u32 {
        self.measure_cached(text, size, weight, false)
    }

    pub(crate) fn measure_chrome(&mut self, text: &str, size: f32, weight: u16) -> u32 {
        self.measure_cached(text, size, weight, true)
    }

    pub(crate) fn begin_bar_frame(&mut self) {
        self.dynamic_used.clear();
    }

    pub(crate) fn finish_bar_frame(&mut self) {
        self.dynamic_buffers
            .retain(|key, _| self.dynamic_used.contains(key));
    }

    fn measure_cached(&mut self, text: &str, size: f32, weight: u16, chrome: bool) -> u32 {
        if text.is_empty() {
            return 0;
        }
        let key = TextKey::new(text, size, weight);
        self.prepare_buffer(&key, chrome);
        let Self {
            font_system,
            chrome_buffers,
            dynamic_buffers,
            ..
        } = self;
        let buffer = if chrome {
            chrome_buffers.get_mut(&key)
        } else {
            dynamic_buffers.get_mut(&key)
        }
        .expect("prepared text buffer");
        let mut borrowed = buffer.borrow_with(font_system);
        borrowed
            .layout_runs()
            .map(|run| run.line_w.ceil().max(0.0) as u32)
            .max()
            .unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: i32,
        y: i32,
        box_height: u32,
        size: f32,
        weight: u16,
        color: Rgb,
        chrome: bool,
    ) {
        if text.is_empty() {
            return;
        }
        let line_height = size * self.line_height;
        let top = y + ((box_height as f32 - line_height) / 2.0).floor() as i32;
        let key = TextKey::new(text, size, weight);
        self.prepare_buffer(&key, chrome);
        let cosmic_color = Color::rgb(color.r, color.g, color.b);
        let width = pixmap.width();
        let height = pixmap.height();
        let Self {
            font_system,
            cache,
            chrome_buffers,
            dynamic_buffers,
            ..
        } = self;
        let buffer = if chrome {
            chrome_buffers.get_mut(&key)
        } else {
            dynamic_buffers.get_mut(&key)
        }
        .expect("prepared text buffer");
        buffer.draw(font_system, cache, cosmic_color, |gx, gy, gw, gh, pixel| {
            for py in 0..gh {
                for px in 0..gw {
                    blend_pixel(
                        pixmap,
                        x + gx + px as i32,
                        top + gy + py as i32,
                        pixel,
                        width,
                        height,
                    );
                }
            }
        });
    }

    fn prepare_buffer(&mut self, key: &TextKey, chrome: bool) {
        let missing = if chrome {
            !self.chrome_buffers.contains_key(key)
        } else {
            self.dynamic_used.insert(key.clone());
            !self.dynamic_buffers.contains_key(key)
        };
        if missing {
            let buffer = self.buffer(&key.text, f32::from_bits(key.size_bits), key.weight);
            if chrome {
                self.chrome_buffers.insert(key.clone(), buffer);
            } else {
                self.dynamic_buffers.insert(key.clone(), buffer);
            }
        }
    }

    fn buffer(&mut self, text: &str, size: f32, weight: u16) -> Buffer {
        let metrics = Metrics::new(size, size * self.line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_wrap(Wrap::None);
        let attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .weight(Weight(weight));
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer
    }

    #[cfg(test)]
    fn cache_sizes(&self) -> (usize, usize) {
        (self.chrome_buffers.len(), self.dynamic_buffers.len())
    }
}

impl TextKey {
    fn new(text: &str, size: f32, weight: u16) -> Self {
        Self {
            text: text.to_owned(),
            size_bits: size.to_bits(),
            weight,
        }
    }
}

fn resolved_faces(db: &Database, chain: &[String]) -> Vec<fontdb::ID> {
    let mut ids = Vec::new();
    for family in chain {
        let families = [DbFamily::Name(family)];
        if let Some(id) = db.query(&Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        }) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        let families = [DbFamily::Monospace];
        if let Some(id) = db.query(&Query {
            families: &families,
            weight: fontdb::Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        }) {
            ids.push(id);
        }
    }
    ids
}

fn blend_pixel(pixmap: &mut Pixmap, x: i32, y: i32, source: Color, width: u32, height: u32) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let offset = ((y as u32 * width + x as u32) * 4) as usize;
    let data = pixmap.data_mut();
    let alpha = source.a() as u16;
    let inverse = 255 - alpha;
    let blend =
        |src: u8, dst: u8| -> u8 { (((src as u16 * alpha) + (dst as u16 * inverse)) / 255) as u8 };
    data[offset] = blend(source.r(), data[offset]);
    data[offset + 1] = blend(source.g(), data[offset + 1]);
    data[offset + 2] = blend(source.b(), data[offset + 2]);
    data[offset + 3] = (alpha + (data[offset + 3] as u16 * inverse) / 255).min(255) as u8;
}

#[cfg(test)]
mod tests {
    use realm_core::Palette;
    use tiny_skia::Pixmap;

    use super::TextSystem;

    fn palette() -> Palette {
        Palette::from_toml(include_str!("../../../palette.toml")).unwrap()
    }

    #[test]
    fn chrome_and_unchanged_dynamic_text_reuse_shaped_buffers() {
        let palette = palette();
        let mut text = TextSystem::new(&palette).unwrap();

        text.begin_bar_frame();
        text.measure("focused title", 13.0, 400);
        text.measure("focused title", 13.0, 400);
        text.measure_chrome("* realm", 13.0, 500);
        text.measure_chrome("* realm", 13.0, 500);
        let mut pixmap = Pixmap::new(320, 32).unwrap();
        text.draw(
            &mut pixmap,
            "focused title",
            0,
            0,
            32,
            13.0,
            400,
            palette.text.bright,
            false,
        );
        text.draw(
            &mut pixmap,
            "* realm",
            160,
            0,
            32,
            13.0,
            500,
            palette.text.bright,
            true,
        );
        text.finish_bar_frame();
        assert_eq!(text.cache_sizes(), (1, 1));

        text.begin_bar_frame();
        text.measure("focused title", 13.0, 400);
        text.measure_chrome("* realm", 13.0, 500);
        text.finish_bar_frame();
        assert_eq!(text.cache_sizes(), (1, 1));
    }
}
