//! Pixel art at exact device pixels.
//!
//! Rune draws an image with a filter, so a bust drawn at 2.4 device pixels per art cell comes
//! out soft. `fit` builds the buffer Rune should draw instead: the art scaled by a whole
//! number to the target's device-pixel size, so Rune samples it one texel to one pixel. When
//! the target is not a whole multiple of the art, `fit` picks the nearest whole factor and
//! either crops the art's edge or pads it with its background colour, whichever changes less.
//!
//! The portraits ship upscaled by a whole factor already (a 40 cell bust as 240 pixels), so the
//! source is first read back at its true cell size by `upscale`.

/// The whole-number factor a source image was upscaled by, or 1. A factor counts only when every
/// block of that size is one colour, so the art itself is never altered.
pub fn upscale(width: u32, height: u32, rgba: &[u8]) -> u32 {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() != w * h * 4 {
        return 1;
    }
    'factor: for k in [12usize, 10, 8, 6, 5, 4, 3, 2] {
        if w % k != 0 || h % k != 0 {
            continue;
        }
        for y in 0..h {
            let first = (y / k * k * w) * 4;
            for x in 0..w {
                let at = (y * w + x) * 4;
                let anchor = first + (x / k * k) * 4;
                if rgba[at..at + 4] != rgba[anchor..anchor + 4] {
                    continue 'factor;
                }
            }
        }
        return k as u32;
    }
    1
}

/// The whole factor to draw `cells` art cells at in `target` device pixels. An exact multiple
/// wins. Otherwise the smaller of the padding left by rounding down and the crop taken by
/// rounding up decides, and a tie pads.
fn factor(cells: u32, target: u32) -> u32 {
    let lower = (target / cells).max(1);
    if lower * cells == target {
        return lower;
    }
    let upper = lower + 1;
    let padded = if lower * cells <= target {
        (target - lower * cells) as f32 / target as f32
    } else {
        (lower * cells - target) as f32 / (lower * cells) as f32
    };
    let cropped = (upper * cells - target) as f32 / (upper * cells) as f32;
    if cropped < padded { upper } else { lower }
}

/// `rgba` (`width` by `height`, upscaled by `k`) as a `target` by `target` buffer of whole
/// scaled cells, centred. Padding takes the colour of the art's top-left cell, which for a
/// portrait is its tile.
pub fn fit(rgba: &[u8], width: u32, height: u32, k: u32, target: u32) -> Vec<u8> {
    let k = k.max(1);
    let (cw, ch) = ((width / k).max(1), (height / k).max(1));
    let f = factor(cw.max(ch), target.max(1)) as i64;
    let offset = |cells: u32| (target as i64 - cells as i64 * f).div_euclid(2);
    let (ox, oy) = (offset(cw), offset(ch));
    let cell = |cx: i64, cy: i64| -> [u8; 4] {
        let (cx, cy) = (
            cx.clamp(0, cw as i64 - 1) as u32,
            cy.clamp(0, ch as i64 - 1) as u32,
        );
        let x = (cx * k + k / 2).min(width - 1) as usize;
        let y = (cy * k + k / 2).min(height - 1) as usize;
        let at = (y * width as usize + x) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    };
    let pad = cell(0, 0);
    let mut out = Vec::with_capacity((target as usize).pow(2) * 4);
    for y in 0..target as i64 {
        let cy = (y - oy).div_euclid(f);
        for x in 0..target as i64 {
            let cx = (x - ox).div_euclid(f);
            let inside = (0..cw as i64).contains(&cx) && (0..ch as i64).contains(&cy);
            out.extend_from_slice(&if inside { cell(cx, cy) } else { pad });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `cells` by `cells` checkerboard of two colours, each cell `k` pixels wide.
    fn board(cells: u32, k: u32) -> Vec<u8> {
        let side = cells * k;
        let mut rgba = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let on = (x / k + y / k).is_multiple_of(2);
                rgba.extend_from_slice(if on {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        rgba
    }

    #[test]
    fn the_upscale_factor_is_read_back_from_the_blocks() {
        assert_eq!(upscale(240, 240, &board(40, 6)), 6);
        assert_eq!(upscale(32, 32, &board(32, 1)), 1);
        // Any block of a different colour breaks the factor.
        let mut art = board(8, 6);
        art[(6 * 48 + 7) * 4] = 9;
        assert_eq!(upscale(48, 48, &art), 1);
    }

    #[test]
    fn a_whole_multiple_is_scaled_exactly() {
        let art = board(40, 6);
        let out = fit(&art, 240, 240, 6, 80);
        assert_eq!(out.len(), 80 * 80 * 4);
        // Two device pixels per cell, so colours change every second pixel and never blend.
        let red = |x: usize, y: usize| out[(y * 80 + x) * 4] == 255;
        assert!(red(0, 0) && red(1, 1) && !red(2, 0) && !red(0, 2) && red(2, 2));
        assert!(
            out.chunks(4)
                .all(|p| p == [255, 0, 0, 255] || p == [0, 0, 255, 255])
        );
    }

    #[test]
    fn an_awkward_target_picks_the_smaller_loss() {
        // 28 cells in 40 pixels: padding to one pixel a cell loses 30%, cropping at two a cell 29%.
        assert_eq!(factor(28, 40), 2);
        // 40 cells in 96: two a cell pads 17%, three crops 20%.
        assert_eq!(factor(40, 96), 2);
        // 40 cells in 144: three pads 17%, four crops 10%.
        assert_eq!(factor(40, 144), 4);
        assert_eq!(factor(32, 64), 2);
        // Smaller than the art: one pixel a cell, cropped.
        assert_eq!(factor(40, 20), 1);
    }

    #[test]
    fn padding_and_cropping_keep_every_pixel_one_of_the_art_colours() {
        let art = board(40, 6);
        for target in [20u32, 40, 48, 72, 96, 144, 216] {
            let out = fit(&art, 240, 240, 6, target);
            assert_eq!(out.len() as u32, target * target * 4);
            assert!(
                out.chunks(4)
                    .all(|p| p == [255, 0, 0, 255] || p == [0, 0, 255, 255]),
                "target {target} blended"
            );
        }
    }
}
