//! Shared native icon encoding for build.rs and the export-icons example.
use crate::brand;
use image::{ImageFormat, RgbaImage, imageops::FilterType};
use std::io::Cursor;

pub fn source() -> RgbaImage {
    RgbaImage::from_raw(brand::SIZE as u32, brand::SIZE as u32, brand::pixels(true))
        .expect("valid Shina mark")
}

pub fn ico() -> Vec<u8> {
    let source = source();
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let frames: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&size| {
            let mut buffer = Cursor::new(Vec::new());
            image::imageops::resize(&source, size, size, FilterType::Nearest)
                .write_to(&mut buffer, ImageFormat::Png)
                .expect("encode icon frame");
            buffer.into_inner()
        })
        .collect();
    let mut ico = vec![0, 0, 1, 0];
    ico.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, frame) in sizes.iter().zip(&frames) {
        ico.extend_from_slice(&[size as u8, size as u8, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += frame.len() as u32;
    }
    for frame in frames {
        ico.extend_from_slice(&frame);
    }
    ico
}
