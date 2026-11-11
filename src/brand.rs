//! The pixel mark shared by the sidebar, native app icons, and favicon exports: a small sailboat.
//! The day sail goes with the light theme and the night sail with the dark theme. Static icons
//! that cannot follow the theme (the executable, favicons, the Mac iconset) use the night sail.

pub const SIZE: usize = 32;

type Rgb = [u8; 3];

const fn rgb(hex: u32) -> Rgb {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

struct Scene {
    sky_top: Rgb,
    sky_bottom: Rgb,
    mast: Rgb,
    flag: Rgb,
    sail: Rgb,
    sail_shade: Rgb,
    jib: Rgb,
    jib_shade: Rgb,
    hull: Rgb,
    hull_shade: Rgb,
    porthole: Rgb,
    crest: Rgb,
    water: Rgb,
    deep: Rgb,
}

const DAY: Scene = Scene {
    sky_top: rgb(0x6fb6dc),
    sky_bottom: rgb(0xb9e0ee),
    mast: rgb(0x8a6a2f),
    flag: rgb(0xe07a54),
    sail: rgb(0xf8f4ea),
    sail_shade: rgb(0xd9d0bb),
    jib: rgb(0x70b9ad),
    jib_shade: rgb(0x3f8a7e),
    hull: rgb(0xe07a54),
    hull_shade: rgb(0xb2553a),
    porthole: rgb(0xf4efe3),
    crest: rgb(0xd6f1ea),
    water: rgb(0x3f9a8c),
    deep: rgb(0x2b7a6e),
};

const NIGHT: Scene = Scene {
    sky_top: rgb(0x26324d),
    sky_bottom: rgb(0x121a2b),
    mast: rgb(0xd3ba6b),
    flag: rgb(0xe07a54),
    sail: rgb(0xf4efe3),
    sail_shade: rgb(0xd9d0bb),
    jib: rgb(0x6aa8c8),
    jib_shade: rgb(0x4a86a6),
    hull: rgb(0xe07a54),
    hull_shade: rgb(0xb2553a),
    porthole: rgb(0xd3ba6b),
    crest: rgb(0x6aa8c8),
    water: rgb(0x24406a),
    deep: rgb(0x162a48),
};

/// The three service dots, carried over from the earlier mark as foam on the day water.
const DOTS: [(usize, Rgb); 3] = [(8, rgb(0x8fbc6a)), (15, rgb(0xd3ba6b)), (22, rgb(0x6aa8c8))];

fn in_tile(x: usize, y: usize) -> bool {
    const R: i32 = 7;
    let clamp = |v: usize| v.clamp(R as usize, SIZE - 1 - R as usize) as i32;
    let (dx, dy) = (x as i32 - clamp(x), y as i32 - clamp(y));
    dx * dx + dy * dy <= R * R
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

/// RGBA pixels of the mark. `night` selects the dark-theme night sail.
pub fn pixels(night: bool) -> Vec<u8> {
    let scene = if night { &NIGHT } else { &DAY };
    let mut grid: Vec<Option<Rgb>> = vec![None; SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            if in_tile(x, y) {
                let t = y as f32 / (SIZE - 1) as f32;
                grid[y * SIZE + x] = Some(mix(scene.sky_top, scene.sky_bottom, t));
            }
        }
    }
    // Paint only inside the rounded tile.
    let mut put = |x: usize, y: usize, color: Rgb| {
        if x < SIZE && y < SIZE && grid[y * SIZE + x].is_some() {
            grid[y * SIZE + x] = Some(color);
        }
    };

    if night {
        for (i, (x, y)) in [
            (6, 5),
            (10, 9),
            (3, 13),
            (27, 8),
            (24, 4),
            (29, 13),
            (21, 10),
        ]
        .into_iter()
        .enumerate()
        {
            put(x, y, if i % 3 == 0 { NIGHT.mast } else { NIGHT.sail });
        }
        // A crescent moon: a disc with an offset disc removed.
        for y in 3..=7usize {
            for x in 4..=8usize {
                let (fx, fy) = (x as f32, y as f32);
                let disc = (fx - 6.0).powi(2) + (fy - 5.0).powi(2);
                let bite = (fx - 7.3).powi(2) + (fy - 4.3).powi(2);
                if disc <= 5.0 && bite > 3.0 {
                    put(x, y, NIGHT.sail);
                }
            }
        }
    } else {
        // Sun with a soft halo.
        for y in 2..=10usize {
            for x in 2..=10usize {
                let d = (x as f32 - 6.0).powi(2) + (y as f32 - 6.0).powi(2);
                if d <= 5.0 {
                    put(x, y, rgb(0xf6d26b));
                } else if d <= 9.0 {
                    put(x, y, rgb(0xb8dde0));
                }
            }
        }
        // Two small clouds.
        for (x0, y0) in [(21usize, 6usize), (25, 11)] {
            for x in x0 + 1..=x0 + 3 {
                put(x, y0, rgb(0xffffff));
            }
            for x in x0..=x0 + 5 {
                put(x, y0 + 1, rgb(0xffffff));
            }
            for x in x0 + 1..=x0 + 4 {
                put(x, y0 + 2, rgb(0xdfeef3));
            }
        }
    }

    for y in 4..=22 {
        put(15, y, scene.mast);
    }
    for (x, y) in [(16, 4), (17, 4), (18, 4), (19, 4), (16, 5), (17, 5)] {
        put(x, y, scene.flag);
    }
    for y in 7..=21usize {
        let width = (((y - 6) as f32 * 0.72).round() as usize + 1).min(10);
        for x in 16..16 + width {
            let shade = x == 16 || y == 21;
            put(x, y, if shade { scene.sail_shade } else { scene.sail });
        }
    }
    for y in 10..=21usize {
        let width = ((y - 9) as f32 * 0.62).round() as usize;
        if width == 0 {
            continue;
        }
        let first = 15 - width;
        for x in first..=14 {
            put(
                x,
                y,
                if x == first {
                    scene.jib_shade
                } else {
                    scene.jib
                },
            );
        }
    }
    for x in 7..=25 {
        put(x, 22, scene.hull);
    }
    for x in 8..=24 {
        put(x, 23, scene.hull);
    }
    for x in 9..=23 {
        put(x, 24, scene.hull_shade);
    }
    for x in [11, 15, 19] {
        put(x, 23, scene.porthole);
    }
    for y in 25..SIZE {
        for x in 0..SIZE {
            let color = if y == 25 && (x + 3) % 7 < 2 {
                scene.crest
            } else if y < 27 {
                scene.water
            } else {
                scene.deep
            };
            put(x, y, color);
        }
    }
    if !night {
        for (x, color) in DOTS {
            put(x, 27, color);
            put(x + 1, 27, color);
        }
    }

    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    for (i, cell) in grid.iter().enumerate() {
        if let Some([r, g, b]) = cell {
            rgba[i * 4..i * 4 + 4].copy_from_slice(&[*r, *g, *b, 255]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_icons_match_the_night_mark_without_interpolation() {
        let source = image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, pixels(true)).unwrap();
        for (bytes, size) in [
            (include_bytes!("../assets/project-icon.png").as_slice(), 768),
            (include_bytes!("../assets/favicon.png").as_slice(), 32),
            (
                include_bytes!("../assets/macos/Shina.iconset/icon_512x512@2x.png").as_slice(),
                1024,
            ),
        ] {
            let exported = image::load_from_memory(bytes).unwrap().to_rgba8();
            assert_eq!(
                exported,
                image::imageops::resize(&source, size, size, image::imageops::FilterType::Nearest),
            );
        }
    }

    #[test]
    fn day_and_night_marks_differ_and_keep_transparent_corners() {
        let (day, night) = (pixels(false), pixels(true));
        assert_ne!(day, night);
        for mark in [&day, &night] {
            assert_eq!(&mark[..4], &[0, 0, 0, 0]);
            // The hull's centre is opaque coral in both.
            assert_eq!(&mark[(22 * SIZE + 16) * 4..][..4], &[0xe0, 0x7a, 0x54, 255]);
        }
    }
}
