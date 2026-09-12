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
        if text.is_empty() {
            return 0;
        }
        let mut buffer = self.buffer(text, size, weight);
        let mut borrowed = buffer.borrow_with(&mut self.font_system);
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
    ) {
        if text.is_empty() {
            return;
        }
        let line_height = size * self.line_height;
        let top = y + ((box_height as f32 - line_height) / 2.0).floor() as i32;
        let mut buffer = self.buffer(text, size, weight);
        let cosmic_color = Color::rgb(color.r, color.g, color.b);
        let width = pixmap.width();
        let height = pixmap.height();
        buffer.draw(
            &mut self.font_system,
            &mut self.cache,
            cosmic_color,
            |gx, gy, gw, gh, pixel| {
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
            },
        );
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
