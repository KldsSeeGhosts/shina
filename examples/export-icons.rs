//! Regenerate checked-in icons with `cargo run --example export-icons -- assets`.
#[path = "../src/brand.rs"]
mod brand;
#[path = "../tools/icon_assets.rs"]
mod icon_assets;

use image::imageops::{FilterType, resize};
use std::{env, fs, path::PathBuf};

fn main() {
    let mut args = env::args().skip(1);
    let output = PathBuf::from(args.next().expect("supply the assets output directory"));
    assert!(args.next().is_none(), "expected one output directory");
    fs::create_dir_all(&output).expect("create icon output");
    let source = icon_assets::source();
    resize(&source, 768, 768, FilterType::Nearest)
        .save(output.join("project-icon.png"))
        .expect("export app mark");
    resize(&source, 32, 32, FilterType::Nearest)
        .save(output.join("favicon.png"))
        .expect("export favicon");
    fs::write(output.join("favicon.ico"), icon_assets::ico()).expect("export ICO");
    let iconset = output.join("macos/Shina.iconset");
    fs::create_dir_all(&iconset).expect("create Mac iconset");
    for size in [16, 32, 128, 256, 512] {
        for scale in [1, 2] {
            let suffix = if scale == 2 { "@2x" } else { "" };
            resize(&source, size * scale, size * scale, FilterType::Nearest)
                .save(iconset.join(format!("icon_{size}x{size}{suffix}.png")))
                .expect("export Mac icon frame");
        }
    }
    println!(
        "Exported Shina app icons and favicons to {}",
        output.display()
    );
}
