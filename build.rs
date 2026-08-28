use std::error::Error;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

const LOGO: &str = "LOGO 2.jpg";
const ICON_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={LOGO}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR")?);
    let icon_path = out_dir.join("photo-editor.ico");
    write_windows_icon(Path::new(LOGO), &icon_path)?;

    winresource::WindowsResource::new()
        .set_icon(icon_path.to_string_lossy().as_ref())
        .set("FileDescription", "Photo Editor")
        .set("ProductName", "Photo Editor")
        .set("OriginalFilename", "photo-editor.exe")
        .compile()?;
    Ok(())
}

/// Build a multi-resolution ICO so desktop, Explorer, and taskbar sizes all
/// have an appropriately sized rendition of the source artwork.
fn write_windows_icon(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    let logo = image::open(source)?;
    let mut frames = Vec::with_capacity(ICON_SIZES.len());
    for size in ICON_SIZES {
        let rgba = logo
            .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(rgba).write_to(&mut png, image::ImageFormat::Png)?;
        frames.push((size, png.into_inner()));
    }

    let mut file = std::io::BufWriter::new(std::fs::File::create(destination)?);
    file.write_all(&0u16.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&(frames.len() as u16).to_le_bytes())?;

    let mut offset = 6 + frames.len() * 16;
    for (size, png) in &frames {
        let dimension = if *size == 256 { 0 } else { *size as u8 };
        file.write_all(&[dimension, dimension, 0, 0])?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&32u16.to_le_bytes())?;
        file.write_all(&(png.len() as u32).to_le_bytes())?;
        file.write_all(&(offset as u32).to_le_bytes())?;
        offset += png.len();
    }
    for (_, png) in frames {
        file.write_all(&png)?;
    }
    Ok(())
}
