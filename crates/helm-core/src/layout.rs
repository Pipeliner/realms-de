//! Layouts as pure projections of the ledger onto a workarea.
//!
//! [`project`] never mutates anything and never reads the clock, so the same
//! ledger and the same workarea always produce byte-identical geometry. That
//! property is what makes undo trivial and what lets the compositor skip a
//! relayout entirely when the projection is unchanged.
//!
//! # Gapless means gapless
//!
//! helm tiles with 1px seams drawn *inside* each window, so the projected
//! rectangles must abut exactly: no gaps, no overlaps, and every pixel of the
//! workarea accounted for. Naive `w / n` arithmetic loses up to `n - 1` pixels
//! and leaves the void showing through as hairline cracks — one of the most
//! common and most visible tiling-WM bugs. [`partition`] distributes the
//! remainder instead, and the test suite asserts exact coverage across a wide
//! sweep of sizes and window counts.

use serde::{Deserialize, Serialize};

use crate::ledger::{Orbit, WinId};

/// An integer rectangle in output-local coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub w: i32,
    /// Height in pixels.
    pub h: i32,
}

impl Rect {
    /// Construct a rectangle.
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Area in pixels.
    pub fn area(&self) -> i64 {
        self.w as i64 * self.h as i64
    }

    /// True when the two rectangles share at least one pixel.
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    /// True when `self` lies entirely within `other`.
    pub fn contained_by(&self, other: &Rect) -> bool {
        self.x >= other.x
            && self.y >= other.y
            && self.x + self.w <= other.x + other.w
            && self.y + self.h <= other.y + other.h
    }
}

/// The region a layout may use, plus the full output it sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workarea {
    /// The whole output, used by fullscreen windows.
    pub output: Rect,
    /// The output minus the bar and which-key strip.
    pub tiles: Rect,
}

impl Workarea {
    /// Derive a workarea from an output size and the reserved strip heights.
    pub fn new(width: i32, height: i32, top_reserved: i32, bottom_reserved: i32) -> Self {
        let top = top_reserved.max(0);
        let bottom = bottom_reserved.max(0);
        let h = (height - top - bottom).max(0);
        Self {
            output: Rect::new(0, 0, width, height),
            tiles: Rect::new(0, top, width, h),
        }
    }
}

/// Layout algorithms available per orbit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Master column on the left, remaining windows in a two-column grid.
    #[default]
    Triptych,
    /// Only the focused window is mapped; the rest stack behind it.
    Mono,
    /// Equal-area grid, used when the ledger outgrows the triptych's shape.
    Even,
}

impl Layout {
    /// The label shown in the bar's layout indicator.
    pub fn label(self) -> &'static str {
        match self {
            Layout::Triptych => "triptych",
            Layout::Mono => "mono",
            Layout::Even => "even",
        }
    }

    /// Parse a layout name from the CLI or a config file.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "triptych" | "t" => Some(Layout::Triptych),
            "mono" | "m" => Some(Layout::Mono),
            "even" | "e" | "grid" => Some(Layout::Even),
            _ => None,
        }
    }

    /// Cycle to the next layout, for a single-key toggle.
    pub fn next(self) -> Self {
        match self {
            Layout::Triptych => Layout::Mono,
            Layout::Mono => Layout::Even,
            Layout::Even => Layout::Triptych,
        }
    }
}

/// Tunables for [`Layout::Triptych`].
///
/// Defaults reproduce the reference desktop at 1920x1080: a 640px master
/// column, a 700/580 split of the remaining width, and a 580/442 split of the
/// height. They are ratios rather than pixel constants so the same shape
/// survives a move to a 2560x1440 or 3840x2160 output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TriptychParams {
    /// Master column width as a fraction of the workarea.
    pub master_ratio: f32,
    /// How many columns the non-master windows are dealt into.
    pub stack_columns: usize,
    /// Width of the first stack column as a fraction of the stack region.
    pub stack_primary_ratio: f32,
    /// Height of the first stack row as a fraction of the workarea, used only
    /// when the stack happens to be exactly two rows deep.
    pub primary_row_ratio: f32,
}

impl Default for TriptychParams {
    fn default() -> Self {
        Self {
            master_ratio: 640.0 / 1920.0,
            stack_columns: 2,
            stack_primary_ratio: 700.0 / 1280.0,
            primary_row_ratio: 580.0 / 1022.0,
        }
    }
}

impl TriptychParams {
    /// Clamp every ratio into a sane range and guarantee at least one column.
    pub fn sanitised(mut self) -> Self {
        self.master_ratio = self.master_ratio.clamp(0.15, 0.85);
        self.stack_primary_ratio = self.stack_primary_ratio.clamp(0.15, 0.85);
        self.primary_row_ratio = self.primary_row_ratio.clamp(0.15, 0.85);
        self.stack_columns = self.stack_columns.clamp(1, 4);
        self
    }
}

/// One window's projected position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    /// Which window this is.
    pub win: WinId,
    /// Where it goes.
    pub rect: Rect,
    /// Whether it currently holds keyboard focus.
    pub focused: bool,
    /// True when the window is mapped but fully covered by another (mono
    /// stacks). The compositor may skip rendering it entirely.
    pub occluded: bool,
}

/// Project an orbit onto a workarea.
///
/// Returns placements in ledger order. Stowed windows are omitted; a fullscreen
/// window replaces the whole projection.
pub fn project(orbit: &Orbit, area: Workarea, params: TriptychParams) -> Vec<Placement> {
    let visible = orbit.visible();
    let focused = orbit.focused();

    if let Some(full) = orbit.fullscreen.filter(|w| visible.contains(w)) {
        return vec![Placement {
            win: full,
            rect: area.output,
            focused: focused == Some(full),
            occluded: false,
        }];
    }

    if visible.is_empty() || area.tiles.w <= 0 || area.tiles.h <= 0 {
        return Vec::new();
    }

    let rects = match orbit.layout {
        Layout::Triptych => triptych(&visible, area.tiles, params.sanitised()),
        Layout::Mono => vec![area.tiles; visible.len()],
        Layout::Even => even(visible.len(), area.tiles),
    };

    let top = match orbit.layout {
        // In a mono stack only the focused window is on top; everything else is
        // exactly behind it and need not be drawn.
        Layout::Mono => focused.or_else(|| visible.first().copied()),
        _ => None,
    };

    visible
        .iter()
        .copied()
        .zip(rects)
        .map(|(win, rect)| Placement {
            win,
            rect,
            focused: focused == Some(win),
            occluded: matches!(orbit.layout, Layout::Mono) && top != Some(win),
        })
        .collect()
}

/// Master column plus a row-major grid of the remainder.
fn triptych(wins: &[WinId], area: Rect, p: TriptychParams) -> Vec<Rect> {
    let n = wins.len();
    if n == 1 {
        return vec![area];
    }

    let cols = partition(area.w, &[p.master_ratio, 1.0 - p.master_ratio]);
    let master = Rect::new(area.x, area.y, cols[0], area.h);
    let stack_area = Rect::new(area.x + cols[0], area.y, cols[1], area.h);

    let mut out = Vec::with_capacity(n);
    out.push(master);
    out.extend(grid(n - 1, stack_area, p.stack_columns, Some(&p)));
    out
}

/// Equal-area grid sized to keep cells as square as practical.
fn even(n: usize, area: Rect) -> Vec<Rect> {
    let cols = (n as f64).sqrt().ceil().max(1.0) as usize;
    grid(n, area, cols, None)
}

/// Deal `n` rectangles row-major into `cols` columns filling `area` exactly.
///
/// A short final row stretches to the full width rather than leaving a hole —
/// gapless tiling has no concept of an empty cell.
fn grid(n: usize, area: Rect, cols: usize, p: Option<&TriptychParams>) -> Vec<Rect> {
    if n == 0 || area.w <= 0 || area.h <= 0 {
        return Vec::new();
    }
    let cols = cols.clamp(1, n);
    let rows = n.div_ceil(cols);

    let row_weights: Vec<f32> = match (rows, p) {
        (2, Some(p)) => vec![p.primary_row_ratio, 1.0 - p.primary_row_ratio],
        _ => vec![1.0 / rows as f32; rows],
    };
    let row_heights = partition(area.h, &row_weights);

    let mut out = Vec::with_capacity(n);
    let mut y = area.y;
    for (row, h) in row_heights.into_iter().enumerate() {
        let first = row * cols;
        if first >= n {
            break;
        }
        let in_row = cols.min(n - first);
        let col_weights: Vec<f32> = match (in_row, p) {
            (2, Some(p)) => vec![p.stack_primary_ratio, 1.0 - p.stack_primary_ratio],
            _ => vec![1.0 / in_row as f32; in_row],
        };
        let widths = partition(area.w, &col_weights);
        let mut x = area.x;
        for w in widths {
            out.push(Rect::new(x, y, w, h));
            x += w;
        }
        y += h;
    }
    out
}

/// Split `total` into integer parts proportional to `weights`, summing exactly
/// to `total`.
///
/// Uses the largest-remainder method: floor every share, then hand the leftover
/// pixels to the parts with the largest fractional loss. Deterministic, stable
/// and — unlike repeated rounding — never off by one.
pub fn partition(total: i32, weights: &[f32]) -> Vec<i32> {
    if weights.is_empty() {
        return Vec::new();
    }
    if total <= 0 {
        return vec![0; weights.len()];
    }
    let sum: f32 = weights.iter().copied().map(|w| w.max(0.0)).sum();
    if sum <= 0.0 {
        return partition(total, &vec![1.0; weights.len()]);
    }

    let mut parts = Vec::with_capacity(weights.len());
    let mut remainders = Vec::with_capacity(weights.len());
    let mut assigned = 0i32;
    for (i, w) in weights.iter().enumerate() {
        let exact = total as f32 * w.max(0.0) / sum;
        let floor = exact.floor().max(0.0) as i32;
        parts.push(floor);
        remainders.push((exact - floor as f32, i));
        assigned += floor;
    }

    let mut leftover = total - assigned;
    remainders.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let mut i = 0;
    while leftover > 0 && !remainders.is_empty() {
        parts[remainders[i % remainders.len()].1] += 1;
        leftover -= 1;
        i += 1;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Dir, Ledger, OrbitId};

    fn orbit_with(n: u64, layout: Layout) -> Orbit {
        let mut l = Ledger::new();
        for i in 0..n {
            l.summon(WinId(i), OrbitId::default());
        }
        l.set_layout(layout);
        l.active_orbit().clone()
    }

    fn area() -> Workarea {
        // 1920x1080 minus the 32px bar and the 26px which-key strip.
        Workarea::new(1920, 1080, 32, 26)
    }

    /// Every projection must cover the workarea exactly: no cracks, no overlap.
    fn assert_exact_tiling(places: &[Placement], region: Rect) {
        assert!(!places.is_empty());
        let covered: i64 = places.iter().map(|p| p.rect.area()).sum();
        assert_eq!(covered, region.area(), "coverage mismatch: {places:#?}");
        for p in places {
            assert!(p.rect.w > 0 && p.rect.h > 0, "degenerate rect {:?}", p.rect);
            assert!(
                p.rect.contained_by(&region),
                "escaped workarea: {:?}",
                p.rect
            );
        }
        for (i, a) in places.iter().enumerate() {
            for b in &places[i + 1..] {
                assert!(
                    !a.rect.intersects(&b.rect),
                    "overlap {:?} / {:?}",
                    a.rect,
                    b.rect
                );
            }
        }
    }

    #[test]
    fn partition_is_exact_and_stable() {
        for total in [0, 1, 3, 7, 1080, 1920, 2561, 3840] {
            for parts in 1..=9usize {
                let w = vec![1.0f32; parts];
                let p = partition(total, &w);
                assert_eq!(
                    p.iter().sum::<i32>(),
                    total.max(0),
                    "total={total} parts={parts}"
                );
                assert!(p.iter().all(|v| *v >= 0));
                let (min, max) = (p.iter().min().unwrap(), p.iter().max().unwrap());
                assert!(max - min <= 1, "unbalanced split {p:?}");
            }
        }
        assert_eq!(
            partition(1920, &[640.0 / 1920.0, 1280.0 / 1920.0]),
            vec![640, 1280]
        );
        assert_eq!(partition(10, &[0.0, 0.0]), vec![5, 5]);
        assert_eq!(partition(-5, &[1.0]), vec![0]);
    }

    #[test]
    fn triptych_matches_the_reference_desktop() {
        let o = orbit_with(5, Layout::Triptych);
        let places = project(&o, area(), TriptychParams::default());
        assert_eq!(places.len(), 5);
        // odin: full-height master column, 640px wide.
        assert_eq!(places[0].rect, Rect::new(0, 32, 640, 1022));
        // thoth and hermes share the top stack row (580px tall).
        assert_eq!(places[1].rect, Rect::new(640, 32, 700, 580));
        assert_eq!(places[2].rect, Rect::new(1340, 32, 580, 580));
        // horus and urania fill the remainder.
        assert_eq!(places[3].rect, Rect::new(640, 612, 700, 442));
        assert_eq!(places[4].rect, Rect::new(1340, 612, 580, 442));
        assert_exact_tiling(&places, area().tiles);
    }

    #[test]
    fn every_layout_tiles_exactly_for_every_plausible_size() {
        for (w, h) in [
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
            (1366, 768),
            (1281, 801),
        ] {
            let a = Workarea::new(w, h, 32, 26);
            for n in 1..=12u64 {
                for layout in [Layout::Triptych, Layout::Even] {
                    let o = orbit_with(n, layout);
                    let places = project(&o, a, TriptychParams::default());
                    assert_eq!(places.len(), n as usize);
                    assert_exact_tiling(&places, a.tiles);
                }
            }
        }
    }

    #[test]
    fn single_window_owns_the_whole_workarea() {
        for layout in [Layout::Triptych, Layout::Mono, Layout::Even] {
            let o = orbit_with(1, layout);
            let places = project(&o, area(), TriptychParams::default());
            assert_eq!(places[0].rect, area().tiles);
        }
    }

    #[test]
    fn mono_stacks_and_marks_everything_but_the_top_occluded() {
        let o = orbit_with(4, Layout::Mono);
        let places = project(&o, area(), TriptychParams::default());
        assert_eq!(places.len(), 4);
        assert!(places.iter().all(|p| p.rect == area().tiles));
        assert_eq!(places.iter().filter(|p| !p.occluded).count(), 1);
        assert!(places
            .iter()
            .find(|p| p.focused)
            .map(|p| !p.occluded)
            .unwrap());
    }

    #[test]
    fn fullscreen_covers_the_output_including_the_bar() {
        let mut l = Ledger::new();
        for i in 0..3 {
            l.summon(WinId(i), OrbitId::default());
        }
        l.toggle_fullscreen();
        let places = project(l.active_orbit(), area(), TriptychParams::default());
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].rect, area().output);
    }

    #[test]
    fn stowed_windows_are_not_projected() {
        let mut l = Ledger::new();
        for i in 0..3 {
            l.summon(WinId(i), OrbitId::default());
        }
        l.toggle_stow();
        let places = project(l.active_orbit(), area(), TriptychParams::default());
        assert_eq!(places.len(), 2);
        assert!(!places.iter().any(|p| p.win == WinId(2)));
        assert_exact_tiling(&places, area().tiles);
    }

    #[test]
    fn empty_orbit_and_degenerate_output_project_to_nothing() {
        let o = orbit_with(0, Layout::Triptych);
        assert!(project(&o, area(), TriptychParams::default()).is_empty());
        let o = orbit_with(3, Layout::Triptych);
        let tiny = Workarea::new(1920, 40, 32, 26); // bars taller than the output
        assert!(project(&o, tiny, TriptychParams::default()).is_empty());
    }

    #[test]
    fn projection_is_pure_and_focus_only_moves_the_flag() {
        let mut l = Ledger::new();
        for i in 0..5 {
            l.summon(WinId(i), OrbitId::default());
        }
        let a = project(l.active_orbit(), area(), TriptychParams::default());
        let b = project(l.active_orbit(), area(), TriptychParams::default());
        assert_eq!(a, b);
        l.focus_step(Dir::Next);
        let c = project(l.active_orbit(), area(), TriptychParams::default());
        let rects_a: Vec<_> = a.iter().map(|p| p.rect).collect();
        let rects_c: Vec<_> = c.iter().map(|p| p.rect).collect();
        assert_eq!(rects_a, rects_c, "focus must not move a single pixel");
    }

    #[test]
    fn absurd_params_are_clamped_rather_than_producing_negative_rects() {
        let o = orbit_with(5, Layout::Triptych);
        let p = TriptychParams {
            master_ratio: 9.0,
            stack_columns: 99,
            stack_primary_ratio: -3.0,
            primary_row_ratio: f32::NAN,
        };
        let places = project(&o, area(), p);
        assert_exact_tiling(&places, area().tiles);
    }
}
