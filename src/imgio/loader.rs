//! Decoding photos (standard formats via `image`, camera RAW via `rawler`)
//! into the pipeline's linear-RGB f32 working format.

use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::engine::pipeline::{srgb_decode, SourceImage};

pub const STD_EXTS: [&str; 7] = ["jpg", "jpeg", "png", "tif", "tiff", "webp", "bmp"];
pub const RAW_EXTS: [&str; 16] = [
    "cr2", "cr3", "nef", "nrw", "arw", "dng", "raf", "orf", "rw2", "pef", "srw", "erf", "kdc",
    "dcr", "3fr", "iiq",
];

pub fn is_raw(path: &Path) -> bool {
    ext_in(path, &RAW_EXTS)
}

pub fn is_supported(path: &Path) -> bool {
    ext_in(path, &STD_EXTS) || ext_in(path, &RAW_EXTS)
}

fn ext_in(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            exts.iter().any(|x| *x == e)
        })
        .unwrap_or(false)
}

/// Decode any supported file into linear RGB f32.
pub fn load(path: &Path) -> Result<SourceImage> {
    let dyn_img = if is_raw(path) {
        decode_raw(path)?
    } else {
        image::open(path).with_context(|| format!("failed to decode {}", path.display()))?
    };
    Ok(dynamic_to_linear(dyn_img))
}

/// Decode a RAW file to an sRGB-encoded DynamicImage using rawler's
/// develop pipeline (demosaic, camera white balance, color calibration).
pub fn decode_raw(path: &Path) -> Result<image::DynamicImage> {
    let raw_image = rawler::decode_file(path)
        .with_context(|| format!("failed to decode RAW {}", path.display()))?;
    let developed = rawler::imgop::develop::RawDevelop::default()
        .develop_intermediate(&raw_image)
        .with_context(|| format!("failed to develop RAW {}", path.display()))?;
    developed
        .to_dynamic_image()
        .context("RAW develop produced no image")
}

/// Convert a decoded (sRGB gamma) image to the linear working format.
fn dynamic_to_linear(img: image::DynamicImage) -> SourceImage {
    let rgb = img.to_rgb32f();
    let (w, h) = (rgb.width() as usize, rgb.height() as usize);
    let mut data = rgb.into_raw();
    data.par_iter_mut().for_each(|v| *v = srgb_decode(*v));
    SourceImage {
        width: w,
        height: h,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extension_detection() {
        assert!(is_supported(&PathBuf::from("a.JPG")));
        assert!(is_supported(&PathBuf::from("a.cr3")));
        assert!(is_raw(&PathBuf::from("a.NEF")));
        assert!(!is_raw(&PathBuf::from("a.png")));
        assert!(!is_supported(&PathBuf::from("a.txt")));
        assert!(!is_supported(&PathBuf::from("noext")));
    }
}
