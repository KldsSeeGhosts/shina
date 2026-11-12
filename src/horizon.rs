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
    /// A captain's home: the sky takes this colour (a portrait's backdrop) and no boat sails.
    Sky(Rgb),
}

/// The horizon band, `width` by `height` points.
pub fn band(width: f32, height: f32, night: bool, subject: Subject) -> Vec<Rect> {
    let palette = if night { &NIGHT } else { &DAY };
    let mut grid = Grid::new(width, height);
    let waterline = grid.rows.saturating_sub(SEA_ROWS);
    // The sky, in four flat steps.
    let (top, bottom) = match subject {
        Subject::Boat => (palette.sky_top, palette.sky_bottom),
        Subject::Sky(backdrop) => {
            let shade = if night {
                [0x0a, 0x0f, 0x1c]
            } else {
                [0xff, 0xff, 0xff]
            };
            (mix(backdrop, shade, 0.45), backdrop)
        }
    };
    let steps = 4;
    for step in 0..steps {
        let from = waterline * step / steps;
        let to = waterline * (step + 1) / steps;
        let t = step as f32 / (steps - 1) as f32;
        let color = match (night, subject) {
            (true, Subject::Boat) => NIGHT_STEPS[step],
            _ => mix(top, bottom, t),
        };
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
    if subject == Subject::Boat {
        // The mark's boat, centred a third of the way along, its keel on the first row of sea.
        // The boat's own grid puts that row at 25.
        let centre = cols / 3;
        let rise = waterline as i32 - 25;
        brand::boat(night, |x, y, color| {
            grid.put(x as i32 - 16 + centre, y as i32 + rise, color);
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
            let rects = band(width, 108.0, true, Subject::Boat);
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
    fn a_captain_band_takes_the_backdrop_and_sails_no_boat() {
        let backdrop = [0x6a, 0x94, 0xc0];
        let rects = band(600.0, 108.0, false, Subject::Sky(backdrop));
        assert!(rects.iter().any(|r| r.color == backdrop));
        let boat = band(600.0, 108.0, false, Subject::Boat);
        assert!(boat.len() > rects.len());
    }

    #[test]
    fn the_boat_sits_a_third_of_the_way_along_on_the_waterline() {
        let rects = band(600.0, 108.0, false, Subject::Boat);
        // The mast is the brass column at the boat's centre, one cell wide.
        let mast = [0x8a, 0x6a, 0x2f];
        let found: Vec<_> = rects.iter().filter(|r| r.color == mast).collect();
        assert!(!found.is_empty());
        let x = found[0].x;
        assert!((x - 200.0).abs() <= 2.0 * CELL, "mast at {x}");
    }
}
