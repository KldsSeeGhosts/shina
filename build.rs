use image::{ImageFormat, imageops::FilterType};
use std::{env, fs, io::Cursor, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/project-icon.png");
    println!("cargo:rerun-if-changed=build.rs");
    let source = image::open("assets/project-icon.png").expect("read project icon");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    // Keep the supplied pixel artwork sharp at the larger Windows icon sizes.
    source
        .resize_exact(256, 256, FilterType::Nearest)
        .save(output.join("shina-icon.png"))
        .expect("write window icon");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sizes = [16u32, 20, 24, 32, 40, 48, 64, 128, 256];
    let frames: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&size| {
            let mut buffer = Cursor::new(Vec::new());
            source
                .resize_exact(size, size, FilterType::Nearest)
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
    let icon_path = output.join("shina.ico");
    fs::write(&icon_path, ico).expect("write executable icon");
    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("UTF-8 icon path"))
        .set("FileDescription", "Shina")
        .set("ProductName", "Shina")
        .compile()
        .expect("compile Windows icon resource");
}
