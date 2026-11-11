#[path = "src/brand.rs"]
mod brand;
#[path = "tools/icon_assets.rs"]
mod icon_assets;
use image::imageops::FilterType;
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/brand.rs");
    println!("cargo:rerun-if-changed=tools/icon_assets.rs");
    println!("cargo:rerun-if-changed=build.rs");
    let source = icon_assets::source();
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    image::imageops::resize(&source, 256, 256, FilterType::Nearest)
        .save(output.join("shina-icon.png"))
        .expect("write window icon");
    let day =
        image::RgbaImage::from_raw(brand::SIZE as u32, brand::SIZE as u32, brand::pixels(false))
            .expect("valid day mark");
    image::imageops::resize(&day, 256, 256, FilterType::Nearest)
        .save(output.join("shina-icon-day.png"))
        .expect("write day window icon");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon_path = output.join("shina.ico");
    fs::write(&icon_path, icon_assets::ico()).expect("write executable icon");
    winresource::WindowsResource::new()
        .set_icon(icon_path.to_str().expect("UTF-8 icon path"))
        .set("FileDescription", "Shina")
        .set("ProductName", "Shina")
        .compile()
        .expect("compile Windows icon resource");
}
