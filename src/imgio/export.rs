//! Baking edits into new files: full-resolution render + encode, both for
//! a single photo (via the render worker's cached decode) and for batch
//! exports of a whole folder (own thread, cancellable, reports progress).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::engine::ops::geometry;
use crate::engine::params::EditParams;
use crate::engine::pipeline::{self, RenderCtx, SourceImage};
use crate::engine::tuning::Tuning;
use crate::imgio::{loader, sidecar};

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

/// Render `src` (geometry already applied) at full resolution with `params`
/// and write it to `dest`.
pub fn export(
    src: &SourceImage,
    params: &EditParams,
    tuning: &Tuning,
    dest: &Path,
    format: ExportFormat,
    jpeg_quality: u8,
) -> Result<()> {
    let rgb_f32 = pipeline::render_rgb(src, params, tuning, RenderCtx::full(src.width, src.height));
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
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, jpeg_quality);
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

// --- Batch export ---

pub enum BatchMsg {
    /// About to process file `index` (0-based) of the batch.
    Progress { index: usize, name: String },
    /// One file failed; the batch continues.
    Failed { name: String, error: String },
    /// All done (or cancelled).
    Finished { exported: usize, failed: usize },
}

pub struct BatchJob {
    pub rx: Receiver<BatchMsg>,
    pub total: usize,
    pub cancel: Arc<AtomicBool>,
}

/// Export every file with its own sidecar edits (or clean defaults) into
/// `dest_dir` as `{stem}_edited.{ext}`. Runs on its own thread so the
/// interactive render worker stays responsive; decodes each file fresh.
pub fn spawn_batch(
    files: Vec<PathBuf>,
    dest_dir: PathBuf,
    format: ExportFormat,
    jpeg_quality: u8,
    tuning: Tuning,
    ctx: egui::Context,
) -> BatchJob {
    let (tx, rx) = std::sync::mpsc::channel::<BatchMsg>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_flag = cancel.clone();
    let total = files.len();
    std::thread::Builder::new()
        .name("batch-export".into())
        .spawn(move || {
            let mut exported = 0usize;
            let mut failed = 0usize;
            for (index, src_path) in files.iter().enumerate() {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                let name = src_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let _ = tx.send(BatchMsg::Progress {
                    index,
                    name: name.clone(),
                });
                ctx.request_repaint();
                match export_one(src_path, &dest_dir, format, jpeg_quality, &tuning) {
                    Ok(()) => exported += 1,
                    Err(e) => {
                        failed += 1;
                        let _ = tx.send(BatchMsg::Failed {
                            name,
                            error: format!("{e:#}"),
                        });
                    }
                }
            }
            let _ = tx.send(BatchMsg::Finished { exported, failed });
            ctx.request_repaint();
        })
        .expect("failed to spawn batch export thread");
    BatchJob { rx, total, cancel }
}

fn export_one(
    src_path: &Path,
    dest_dir: &Path,
    format: ExportFormat,
    jpeg_quality: u8,
    tuning: &Tuning,
) -> Result<()> {
    let params = sidecar::load(src_path).unwrap_or_default();
    let src = loader::load(src_path)?;
    let geo;
    let render_src = if params.has_geometry(true) {
        geo = geometry::apply(&src, &params, true);
        &geo
    } else {
        &src
    };
    let stem = src_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let dest = dest_dir.join(format!("{stem}_edited.{}", format.extension()));
    export(render_src, &params, tuning, &dest, format, jpeg_quality)
}
