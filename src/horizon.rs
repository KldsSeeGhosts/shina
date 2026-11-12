//! Pixel scenery for the window, built from the palette of the app mark in `brand.rs`.
//!
//! `band` is the horizon over the new-thread screen: a sky in four flat steps, stars and a moon
//! (night) or a sun and clouds (day), and a two-tone sea with crest pixels along its top edge.
//! `strip` is the sea alone, for the foot of the sidebar. Both return flat rectangles in points,
//! with horizontal runs merged, so a band is a few dozen shapes and its pixels stay on one grid.
use crate::brand::{self, Rgb};

/// One pixel of scenery is this many points. Three points are six device pixels on a 2x display.
pub const CELL: f32 = 3.0;

/// A flat rectangle in points, relative to the scene's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgb,
}

struct Palette {
    sky_top: Rgb,
    sky_bottom: Rgb,
    crest: Rgb,
    water: Rgb,
    deep: Rgb,
    star: Rgb,
    star_alt: Rgb,
}

/// The night sky in four clear steps, from the icon's top `#26324d` to its bottom `#121a2b`,
/// with a brighter first step so the top reads as lit against the `#162036` panel.
const NIGHT_STEPS: [Rgb; 4] = [
    [0x2d, 0x3c, 0x5f],
    [0x26, 0x32, 0x4d],
    [0x1d, 0x28, 0x40],
    [0x12, 0x1a, 0x2b],
];

const NIGHT: Palette = Palette {
    sky_top: [0x26, 0x32, 0x4d],
    sky_bottom: [0x12, 0x1a, 0x2b],
    crest: [0x8c, 0xc0, 0xd8],
    // Brighter than the first pass (`#24406a`, `#162a48`), so the sea reads as water against
    // the `#162036` panel and the `#101727` sidebar.
    water: [0x2e, 0x54, 0x88],
    deep: [0x20, 0x3d, 0x68],
    star: [0xf4, 0xef, 0xe3],
    star_alt: [0xd3, 0xba, 0x6b],
};

const DAY: Palette = Palette {
    sky_top: [0x6f, 0xb6, 0xdc],
    sky_bottom: [0xb9, 0xe0, 0xee],
    crest: [0xd6, 0xf1, 0xea],
    water: [0x3f, 0x9a, 0x8c],
    deep: [0x2b, 0x7a, 0x6e],
    star: [0xff, 0xff, 0xff],
    star_alt: [0xdf, 0xee, 0xf3],
};

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// A tiny deterministic generator, so the same width always draws the same stars.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.0 >> 8) as usize) % bound.max(1)
    }
}

/// How far a captain's backdrop pulls the two lowest sky steps from their own colours: the step
/// above the sea first, then the one above it.
const BACKDROP_TINT: [f32; 2] = [0.6, 0.3];

/// Most distant sails on a horizon, one per working thread or crew task.
pub const FLEET_MAX: usize = 4;
/// Where each distant sail's mast stands, as a fraction of the band's width. The first sail
/// is the one nearest the right edge of the captain's portrait. None crosses the boat (at a
/// third or a quarter along) or the portrait that overlaps the middle of a captain's band.
const FLEET_AT: [f32; FLEET_MAX] = [0.70, 0.12, 0.90, 0.80];

/// Rows of sea at the foot of a band: crest, two of water and the rest deep.
const SEA_ROWS: usize = 8;

/// A grid of cells that merges each row's runs of one colour into rectangles.
struct Grid {
    cols: usize,
    rows: usize,
    cells: Vec<Option<Rgb>>,
}

impl Grid {
    fn new(width: f32, height: f32) -> Grid {
        let cols = (width / CELL).ceil().max(1.0) as usize;
        let rows = (height / CELL).round().max(1.0) as usize;
        Grid {
            cols,
            rows,
            cells: vec![None; cols * rows],
        }
    }
    fn put(&mut self, x: i32, y: i32, color: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < self.cols && (y as usize) < self.rows {
            self.cells[y as usize * self.cols + x as usize] = Some(color);
        }
    }
    fn fill_rows(&mut self, from: usize, to: usize, color: Rgb) {
        for y in from..to.min(self.rows) {
            for x in 0..self.cols {
                self.cells[y * self.cols + x] = Some(color);
            }
        }
    }
    fn rects(&self, width: f32) -> Vec<Rect> {
        let mut out = Vec::new();
        for y in 0..self.rows {
            let mut x = 0;
            while x < self.cols {
                let Some(color) = self.cells[y * self.cols + x] else {
                    x += 1;
                    continue;
                };
                let start = x;
                while x < self.cols && self.cells[y * self.cols + x] == Some(color) {
                    x += 1;
                }
                let left = start as f32 * CELL;
                out.push(Rect {
                    x: left,
                    y: y as f32 * CELL,
                    // The last run reaches the scene's edge exactly, however it divides.
                    w: if x == self.cols {
                        width - left
                    } else {
                        (x - start) as f32 * CELL
                    },
                    h: CELL,
                    color,
                });
            }
        }
        out
    }
}

fn sea(grid: &mut Grid, top: usize, palette: &Palette) {
    for y in top..grid.rows {
        let band = y - top;
        // The crest row and two rows below it are the lighter water; the rest is deep.
        let color = match band {
            0..=2 => palette.water,
            _ => palette.deep,
        };
        grid.fill_rows(y, y + 1, color);
    }
    // Crests along the top edge, one or two cells wide with uneven gaps of three to seven, so
    // the waterline does not read as a ruler. The same seed draws the same crests at any width.
    let mut random = Lcg(0xc2e5);
    let mut x = random.next(5) as i32;
    while x < grid.cols as i32 {
        let run = 1 + random.next(2) as i32;
        for dx in 0..run {
            grid.put(x + dx, top as i32, palette.crest);
        }
        x += run + 3 + random.next(5) as i32;
    }
}

/// The sea at the foot of the sidebar: crests, water and deep water, `height` points tall.
pub fn strip(width: f32, height: f32, night: bool) -> Vec<Rect> {
    let palette = if night { &NIGHT } else { &DAY };
    let mut grid = Grid::new(width, height);
    sea(&mut grid, 0, palette);
    grid.rects(width)
}

/// How the band is drawn: the boat on the waterline, or a bare sea for a portrait to overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Subject {
    Boat,
    /// A captain's home: the boat flies a pennant in this colour (a portrait's backdrop), and
    /// the two lowest sky steps take some of it.
    Sky(Rgb),
}

/// The horizon band, `width` by `height` points, with one distant sail for each of `fleet`
/// working threads or tasks, up to [`FLEET_MAX`].
pub fn band(width: f32, height: f32, night: bool, subject: Subject, fleet: usize) -> Vec<Rect> {
    let palette = if night { &NIGHT } else { &DAY };
    let mut grid = Grid::new(width, height);
    let waterline = grid.rows.saturating_sub(SEA_ROWS);
    // The sky, in four flat steps. A captain's home keeps the day or night sky and tints only
    // the step above the sea with the portrait's backdrop, so the band still reads as the same
    // place and the captain's colour shows as a glow at the horizon.
    let steps = 4;
    for step in 0..steps {
        let from = waterline * step / steps;
        let to = waterline * (step + 1) / steps;
        let t = step as f32 / (steps - 1) as f32;
        let mut color = if night {
            NIGHT_STEPS[step]
        } else {
            mix(palette.sky_top, palette.sky_bottom, t)
        };
        if let Subject::Sky(backdrop) = subject
            && let Some(tint) = (steps - 1)
                .checked_sub(step)
                .and_then(|from_sea| BACKDROP_TINT.get(from_sea))
        {
            color = mix(color, backdrop, *tint);
        }
        grid.fill_rows(from, to, color);
    }
    let cols = grid.cols as i32;
    let mut random = Lcg(0x5eed);
    if night {
        // Stars, one cell each, in the upper part of the sky.
        let count = (grid.cols / 9).max(4);
        for i in 0..count {
            let x = random.next(grid.cols) as i32;
            // Never on the first rows, where the frame would clip them.
            let y = 2 + random.next((waterline * 2 / 3).saturating_sub(2).max(1)) as i32;
            grid.put(
                x,
                y,
                if i % 3 == 0 {
                    palette.star_alt
                } else {
                    palette.star
                },
            );
        }
        // The mark's crescent, drawn from its own 5 by 5 art pixels at twice the size: each
        // art pixel is a 2 by 2 block of cells.
        const MOON: [&str; 5] = [".XX..", "XX...", "XX...", "XXX..", ".XXX."];
        let (mx, my) = (cols * 4 / 5, 4);
        for (row, line) in MOON.iter().enumerate() {
            for (column, art) in line.bytes().enumerate() {
                if art == b'X' {
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        grid.put(
                            mx + column as i32 * 2 + dx,
                            my + row as i32 * 2 + dy,
                            palette.star,
                        );
                    }
                }
            }
        }
    } else {
        // A sun in two steps and no halo: a 5 by 5 disc with its corners cut, in gold, around a
        // 3 by 3 core in pale gold.
        let (sx, sy) = (cols * 4 / 5, 6);
        for dy in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx.abs() == 2 && dy.abs() == 2 {
                    continue;
                }
                let core = dx.abs() <= 1 && dy.abs() <= 1;
                grid.put(
                    sx + dx,
                    sy + dy,
                    if core {
                        [0xfb, 0xe9, 0xa6]
                    } else {
                        [0xf6, 0xc8, 0x5a]
                    },
                );
            }
        }
        let (cx, cy) = (cols * 3 / 5, 8);
        for x in 1..=3 {
            grid.put(cx + x, cy, palette.star);
        }
        for x in 0..=5 {
            grid.put(cx + x, cy + 1, palette.star);
        }
        for x in 1..=4 {
            grid.put(cx + x, cy + 2, palette.star_alt);
        }
    }
    sea(&mut grid, waterline, palette);
    // The mark's boat, its keel on the first row of sea. The boat's own grid puts that row at 25.
    // It sits a third of the way along the new-thread screen's band. On a captain's band it is a
    // quarter along, clear of the portrait, and its flag becomes a 3 by 2 pennant in the
    // captain's colour.
    let centre = match subject {
        Subject::Boat => cols / 3,
        Subject::Sky(_) => cols / 4,
    };
    let rise = waterline as i32 - 25;
    brand::boat(night, |x, y, color| {
        if matches!(subject, Subject::Sky(_)) && y <= 5 && x >= 16 {
            return;
        }
        grid.put(x as i32 - 16 + centre, y as i32 + rise, color);
    });
    if let Subject::Sky(pennant) = subject {
        for (x, y) in [(16, 4), (17, 4), (18, 4), (16, 5), (17, 5), (18, 5)] {
            grid.put(x - 16 + centre, y + rise, pennant);
        }
    }
    // Distant sails keel-down on the waterline, in fixed places, so they never move.
    for fraction in FLEET_AT.iter().take(fleet.min(FLEET_MAX)) {
        let mast = (cols as f32 * fraction) as i32;
        let rise = waterline as i32 - 8;
        brand::distant_sail(night, |x, y, color| {
            grid.put(mast - 2 + x as i32, rise + y as i32, color);
        });
    }
    grid.rects(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covered(rects: &[Rect], x: f32, y: f32) -> Option<Rgb> {
        rects
            .iter()
            .rev()
            .find(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
            .map(|r| r.color)
    }

    #[test]
    fn a_band_fills_its_rectangle_and_stays_inside_it() {
        for width in [321.0, 600.0, 736.0, 864.0] {
            let rects = band(width, 108.0, true, Subject::Boat, 0);
            for r in &rects {
                assert!(r.x >= 0.0 && r.x + r.w <= width + 0.01, "{r:?}");
                assert!(r.y >= 0.0 && r.y + r.h <= 108.01, "{r:?}");
            }
            for y in (0..108).step_by(7) {
                for x in (0..width as i32).step_by(11) {
                    assert!(covered(&rects, x as f32, y as f32).is_some(), "{x},{y}");
                }
            }
        }
    }

    #[test]
    fn the_sea_is_two_tone_with_crests_on_its_top_edge() {
        let rects = strip(240.0, 24.0, true);
        // Top row has crests and water; the last row is deep water everywhere.
        assert!((0..240).all(|x| covered(&rects, x as f32, 23.0) == Some(NIGHT.deep)));
        let top: Vec<_> = (0..240)
            .filter_map(|x| covered(&rects, x as f32, 1.0))
            .collect();
        assert!(top.contains(&NIGHT.crest) && top.contains(&NIGHT.water));
    }

    #[test]
    fn a_captain_band_keeps_the_sky_and_tints_its_two_lowest_steps() {
        let backdrop = [0x6a, 0x94, 0xc0];
        for night in [false, true] {
            let tinted = band(600.0, 108.0, night, Subject::Sky(backdrop), 0);
            let plain = band(600.0, 108.0, night, Subject::Boat, 0);
            // Column 2 has no scenery in the sky: read each of the four steps there.
            let waterline = 108.0 - SEA_ROWS as f32 * CELL;
            let step = |rects: &[Rect], n: usize| {
                covered(rects, 1.0, waterline * (n as f32 + 0.5) / 4.0).unwrap()
            };
            for n in 0..2 {
                assert_eq!(step(&tinted, n), step(&plain, n), "night {night} step {n}");
            }
            assert_eq!(step(&tinted, 2), mix(step(&plain, 2), backdrop, 0.3));
            assert_eq!(step(&tinted, 3), mix(step(&plain, 3), backdrop, 0.6));
            assert_ne!(step(&tinted, 3), step(&plain, 3));
        }
    }

    #[test]
    fn a_captain_boat_flies_a_three_by_two_pennant_in_the_captains_colour() {
        let backdrop = [0x6a, 0x94, 0xc0];
        let rects = band(600.0, 108.0, false, Subject::Sky(backdrop), 0);
        let flag = [0xe0, 0x7a, 0x54];
        // The hull has the same coral, but sits in the lower half of the band.
        assert!(
            rects.iter().all(|r| r.color != flag || r.y >= 60.0),
            "the boat's own flag is gone"
        );
        let cells: Vec<_> = rects.iter().filter(|r| r.color == backdrop).collect();
        // Two rows, one run of three cells each.
        assert_eq!(cells.len(), 2);
        assert!(cells.iter().all(|r| r.w == 3.0 * CELL && r.h == CELL));
        assert_eq!(cells[1].y - cells[0].y, CELL);
        assert_eq!(cells[0].x, cells[1].x);
    }

    /// The cells a distant sail draws, as (column, row) from the band's top-left.
    fn sail_cells(rects: &[Rect], night: bool) -> Vec<(i32, i32)> {
        let mut mast = Vec::new();
        brand::distant_sail(night, |x, y, color| {
            if x == 2 && y == 0 {
                mast.push(color);
            }
        });
        let mast = mast[0];
        let mut found = Vec::new();
        for r in rects.iter().filter(|r| r.color == mast) {
            // A mast run is one cell wide. The boat's mast is much taller and is skipped.
            if r.w == CELL {
                found.push(((r.x / CELL) as i32, (r.y / CELL) as i32));
            }
        }
        found
    }

    #[test]
    fn one_distant_sail_flies_for_each_working_thread_up_to_four() {
        for night in [false, true] {
            let bare = band(600.0, 108.0, night, Subject::Boat, 0);
            let mut last = bare.clone();
            for (fleet, want) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (9, 4)] {
                let rects = band(600.0, 108.0, night, Subject::Boat, fleet);
                let added = sail_cells(&rects, night).len() - sail_cells(&bare, night).len();
                // Each sail's mast is six cells tall, one cell wide, in rows of one run each.
                assert_eq!(added, want * 6, "night {night}, fleet {fleet}");
                if fleet <= 4 {
                    // Every sail is on the 3 point grid, inside the band and above the sea.
                    for r in rects.iter().filter(|r| !bare.contains(r)) {
                        assert_eq!(r.x % CELL, 0.0);
                        assert_eq!(r.y % CELL, 0.0);
                        assert!(r.x >= 0.0 && r.x + r.w <= 600.0);
                        assert!(r.y + r.h <= 108.0 - (SEA_ROWS as f32 - 1.0) * CELL + 0.1);
                    }
                }
                last = rects;
            }
            // Fixed places: the first sail stays where it is when more join it.
            let one = sail_cells(&band(600.0, 108.0, night, Subject::Boat, 1), night);
            let four = sail_cells(&band(600.0, 108.0, night, Subject::Boat, 4), night);
            assert!(one.iter().all(|cell| four.contains(cell)), "night {night}");
            assert_eq!(band(600.0, 108.0, night, Subject::Boat, 4), last);
        }
    }

    #[test]
    fn a_distant_sail_is_a_third_the_size_of_the_boat() {
        let (mut boat_height, mut boat_width) = ((usize::MAX, 0), (usize::MAX, 0));
        brand::boat(false, |x, y, _| {
            boat_height = (boat_height.0.min(y), boat_height.1.max(y));
            boat_width = (boat_width.0.min(x), boat_width.1.max(x));
        });
        let (mut sail_height, mut sail_width) = ((usize::MAX, 0), (usize::MAX, 0));
        brand::distant_sail(false, |x, y, _| {
            sail_height = (sail_height.0.min(y), sail_height.1.max(y));
            sail_width = (sail_width.0.min(x), sail_width.1.max(x));
        });
        let ratio = |sail: (usize, usize), boat: (usize, usize)| {
            (sail.1 - sail.0 + 1) as f32 / (boat.1 - boat.0 + 1) as f32
        };
        assert!((0.3..=0.4).contains(&ratio(sail_height, boat_height)));
        assert!((0.3..=0.4).contains(&ratio(sail_width, boat_width)));
    }

    #[test]
    fn the_boat_sits_a_third_of_the_way_along_on_the_waterline() {
        let rects = band(600.0, 108.0, false, Subject::Boat, 0);
        // The mast is the brass column at the boat's centre, one cell wide.
        let mast = [0x8a, 0x6a, 0x2f];
        let found: Vec<_> = rects.iter().filter(|r| r.color == mast).collect();
        assert!(!found.is_empty());
        let x = found[0].x;
        assert!((x - 200.0).abs() <= 2.0 * CELL, "mast at {x}");
    }
}
