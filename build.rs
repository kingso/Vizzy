use std::env;
use std::error::Error;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};

use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use image::RgbaImage;
use resvg::{tiny_skia, usvg};

const ICON_SIZES: [u32; 6] = [16, 32, 48, 64, 128, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vizzy.svg");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let svg_path = manifest_dir.join("vizzy.svg");
    let png_path = out_dir.join("vizzy-icon-256.png");
    let ico_path = out_dir.join("vizzy.ico");

    generate_icon_assets(&svg_path, &png_path, &ico_path).expect("failed to generate Vizzy icon assets");

    #[cfg(target_os = "windows")]
    embed_windows_icon(&out_dir).expect("failed to embed Vizzy icon resource");
}

fn generate_icon_assets(svg_path: &Path, png_path: &Path, ico_path: &Path) -> Result<(), Box<dyn Error>> {
    let svg_bytes = fs::read(svg_path)?;
    let options = usvg::Options::default();
    let tree = usvg::Tree::from_data(&svg_bytes, &options)?;
    let tree_size = tree.size();
    let base_scale = 256.0 / tree_size.width().max(tree_size.height());

    let png_rgba = render_svg_rgba(&tree, 256, base_scale)?;
    let png_image = RgbaImage::from_raw(256, 256, png_rgba)
        .ok_or("failed to assemble PNG icon buffer")?;
    png_image.save(png_path)?;

    let mut icon_dir = IconDir::new(ResourceType::Icon);
    for size in ICON_SIZES {
        let scale = size as f32 / tree_size.width().max(tree_size.height());
        let rgba = render_svg_rgba(&tree, size, scale)?;
        let icon_image = IconImage::from_rgba_data(size, size, rgba);
        icon_dir.add_entry(IconDirEntry::encode(&icon_image)?);
    }

    let mut icon_file = File::create(ico_path)?;
    icon_dir.write(&mut icon_file)?;
    Ok(())
}

fn render_svg_rgba(tree: &usvg::Tree, size: u32, scale: f32) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("failed to create icon pixmap")?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap.data().to_vec())
}

#[cfg(target_os = "windows")]
fn embed_windows_icon(out_dir: &Path) -> Result<(), Box<dyn Error>> {
    let rc_path = out_dir.join("vizzy.rc");
    fs::write(&rc_path, "1 ICON \"vizzy.ico\"\n")?;
    let _ = embed_resource::compile(rc_path, embed_resource::NONE);
    Ok(())
}