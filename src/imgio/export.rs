//! Baking edits into a new file: full-resolution render + encode.

use std::path::Path;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::engine::params::EditParams;
use crate::engine::pipeline::{self, SourceImage};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportFormat {
    Jpeg,
    Png,
    Tiff,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Tiff];

    pub fn label(&self) -> &'static str {
        match self {
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Png => "PNG",
            ExportFormat::Tiff => "TIFF",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Png => "png",
            ExportFormat::Tiff => "tif",
        }
    }
}

/// Render `src` at full resolution with `params` and write it to `dest`.
pub fn export(
    src: &SourceImage,
    params: &EditParams,
    dest: &Path,
    format: ExportFormat,
    jpeg_quality: u8,
) -> Result<()> {
    let rgb_f32 = pipeline::render_rgb(src, params);
    let mut bytes = vec![0u8; rgb_f32.len()];
    bytes
        .par_iter_mut()
        .zip(rgb_f32.par_iter())
        .for_each(|(d, s)| *d = (s * 255.0 + 0.5) as u8);
    let img: image::RgbImage =
        image::ImageBuffer::from_raw(src.width as u32, src.height as u32, bytes)
            .context("failed to build output image buffer")?;

    match format {
        ExportFormat::Jpeg => {
            let file = std::fs::File::create(dest)
                .with_context(|| format!("cannot create {}", dest.display()))?;
            let writer = std::io::BufWriter::new(file);
            let encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(writer, jpeg_quality);
            img.write_with_encoder(encoder)
                .with_context(|| format!("failed to encode JPEG {}", dest.display()))?;
        }
        ExportFormat::Png | ExportFormat::Tiff => {
            img.save(dest)
                .with_context(|| format!("failed to save {}", dest.display()))?;
        }
    }
    Ok(())
}
