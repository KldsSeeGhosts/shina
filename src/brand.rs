//! The pixel mark shared by the sidebar, native app icons, and favicon exports.

pub const SIZE: usize = 24;

pub fn pixels(light: bool) -> Vec<u8> {
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let mut put = |x: usize, y: usize, color: [u8; 4]| {
        let at = (y * SIZE + x) * 4;
        rgba[at..at + 4].copy_from_slice(&color);
    };
    let (ink, cream) = if light {
        ([236, 236, 238, 255], [30, 30, 32, 255])
    } else {
        ([35, 35, 31, 255], [244, 239, 227, 255])
    };
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as i32 - x.clamp(5, SIZE - 6) as i32;
            let dy = y as i32 - y.clamp(5, SIZE - 6) as i32;
            if dx * dx + dy * dy <= 25 {
                put(x, y, ink);
            }
        }
    }
    const S: [&str; 7] = [
        "01110", "10000", "10000", "01110", "00001", "00001", "01110",
    ];
    for (row, bits) in S.iter().enumerate() {
        for (column, bit) in bits.bytes().enumerate() {
            if bit != b'0' {
                for dy in 0..2 {
                    for dx in 0..2 {
                        put(7 + column * 2 + dx, 5 + row * 2 + dy, cream);
                    }
                }
            }
        }
    }
    for (offset, color) in [
        (7, [143, 188, 106, 255]),
        (11, [224, 122, 84, 255]),
        (15, [106, 168, 200, 255]),
    ] {
        for dy in 0..2 {
            for dx in 0..2 {
                put(offset + dx, 20 + dy, color);
            }
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_icons_match_the_in_app_mark_without_interpolation() {
        let source = image::RgbaImage::from_raw(SIZE as u32, SIZE as u32, pixels(false)).unwrap();
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
        let light = pixels(true);
        assert_eq!(&light[..4], &[0, 0, 0, 0]);
        assert_eq!(&light[(12 * SIZE + 12) * 4..][..4], &[30, 30, 32, 255]);
    }
}
