//! The render worker thread. It owns the decoded photo plus derived caches
//! (downscaled preview base, geometry-applied versions of both) so the UI
//! thread never touches pixels. Commands arrive over a channel; stale
//! preview/region render requests are coalesced so only the newest slider
//! state is rendered.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

use crate::engine::histogram::{self, Histogram};
use crate::engine::ops::{color, geometry};
use crate::engine::params::EditParams;
use crate::engine::pipeline::{self, RenderCtx, SourceImage};
use crate::engine::tuning::Tuning;
use crate::imgio::export::ExportFormat;
use crate::imgio::loader;
use crate::imgio::metadata::{self, ExifInfo};

/// Cap on region render size (~4 MP) — bigger viewports just get downsampled.
const MAX_REGION_PIXELS: usize = 4_000_000;

/// Shadow/highlight clipping overlay toggles (view-only, not part of the edit).
pub type ClipFlags = (bool, bool);

pub enum Cmd {
    /// Decode a photo and render it with the given (sidecar-restored) params.
    Load {
        path: PathBuf,
        params: EditParams,
        tuning: Tuning,
        clip: ClipFlags,
    },
    /// Re-render the preview. `include_crop = false` while the crop tool is
    /// open, so the whole frame stays visible. `overlay` is the index of a
    /// mask whose coverage should be washed over the preview in red.
    Render {
        params: EditParams,
        tuning: Tuning,
        include_crop: bool,
        clip: ClipFlags,
        overlay: Option<usize>,
    },
    /// Render a zoomed-in region of the full-resolution image.
    /// `norm_rect` is (x, y, w, h) normalized to the geometry-applied image.
    RenderRegion {
        params: EditParams,
        tuning: Tuning,
        norm_rect: [f32; 4],
        target: (usize, usize),
        clip: ClipFlags,
    },
    /// Full-resolution render + encode of the currently loaded photo.
    Export {
        dest: PathBuf,
        params: EditParams,
        tuning: Tuning,
        format: ExportFormat,
        jpeg_quality: u8,
    },
    /// White-balance eyedropper: sample the geometry-applied source at a
    /// normalized point and reply with neutralizing temp/tint slider values.
    SampleNeutral {
        params: EditParams,
        norm_point: [f32; 2],
    },
}

pub enum Reply {
    Loaded {
        path: PathBuf,
        full_width: usize,
        full_height: usize,
        exif: ExifInfo,
    },
    /// Result of a white-balance eyedropper sample.
    Neutral {
        temp: f32,
        tint: f32,
    },
    Preview {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
        /// Size of the geometry-applied full image the preview represents —
        /// what region requests and crop overlays should map against.
        full_size: (usize, usize),
        /// Channel histogram of the render (computed before clip marking).
        histogram: Histogram,
        /// Coverage of the mask being edited, one byte per pixel, when the
        /// "show mask coverage" overlay is on.
        coverage: Option<Vec<u8>>,
    },
    Region {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
        norm_rect: [f32; 4],
    },
    ExportDone(PathBuf),
    Error(String),
}

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Reply>,
}

pub fn spawn(ctx: egui::Context) -> Worker {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<Cmd>();
    let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Reply>();
    std::thread::Builder::new()
        .name("render-worker".into())
        .spawn(move || WorkerState::default().run(cmd_rx, reply_tx, ctx))
        .expect("failed to spawn render worker");
    Worker {
        tx: cmd_tx,
        rx: reply_rx,
    }
}

type GeoSig = (u8, bool, bool, u32, [u32; 4]);

#[derive(Default)]
struct WorkerState {
    /// Full-resolution decode of the current photo (no geometry applied).
    full: Option<SourceImage>,
    /// Downscaled copy of `full` used for interactive preview renders.
    preview_base: Option<SourceImage>,
    /// `preview_base` with geometry applied (None = geometry is identity).
    preview_geo: Option<(GeoSig, Option<SourceImage>)>,
    /// `full` with geometry applied (crop always included) for region
    /// renders and export. Heavy — computed lazily, cached by signature.
    full_geo: Option<(GeoSig, Option<SourceImage>)>,
}

impl WorkerState {
    fn run(&mut self, rx: Receiver<Cmd>, tx: Sender<Reply>, ctx: egui::Context) {
        'outer: while let Ok(first) = rx.recv() {
            let mut queue: VecDeque<Cmd> = VecDeque::new();
            queue.push_back(first);
            while let Ok(next) = rx.try_recv() {
                queue.push_back(next);
            }
            while let Some(cmd) = queue.pop_front() {
                // Drop stale requests: a later render (or a load) makes an
                // earlier one of the same kind pointless.
                let superseded = match &cmd {
                    Cmd::Render { .. } => queue
                        .iter()
                        .any(|m| matches!(m, Cmd::Render { .. } | Cmd::Load { .. })),
                    Cmd::RenderRegion { .. } => queue
                        .iter()
                        .any(|m| matches!(m, Cmd::RenderRegion { .. } | Cmd::Load { .. })),
                    _ => false,
                };
                if superseded {
                    continue;
                }
                for reply in self.handle(cmd) {
                    if tx.send(reply).is_err() {
                        break 'outer; // UI is gone
                    }
                    ctx.request_repaint();
                }
            }
        }
    }

    fn handle(&mut self, cmd: Cmd) -> Vec<Reply> {
        match cmd {
            Cmd::Load {
                path,
                params,
                tuning,
                clip,
            } => match loader::load(&path) {
                Ok(src) => {
                    let preview_base = src.downscale(tuning.preview_edge.max(512) as usize);
                    let mut exif = metadata::read(&path);
                    exif.dimensions = Some(format!("{} × {}", src.width, src.height));
                    let loaded = Reply::Loaded {
                        path,
                        full_width: src.width,
                        full_height: src.height,
                        exif,
                    };
                    self.full = Some(src);
                    self.preview_base = Some(preview_base);
                    self.preview_geo = None;
                    self.full_geo = None;
                    let mut replies = vec![loaded];
                    replies.extend(self.render_preview(&params, &tuning, true, clip, None));
                    replies
                }
                Err(e) => vec![Reply::Error(format!("{e:#}"))],
            },
            Cmd::Render {
                params,
                tuning,
                include_crop,
                clip,
                overlay,
            } => self.render_preview(&params, &tuning, include_crop, clip, overlay),
            Cmd::RenderRegion {
                params,
                tuning,
                norm_rect,
                target,
                clip,
            } => self.render_region(&params, &tuning, norm_rect, target, clip),
            Cmd::Export {
                dest,
                params,
                tuning,
                format,
                jpeg_quality,
            } => {
                if self.full.is_none() {
                    return vec![Reply::Error("no photo loaded to export".into())];
                }
                let Some(src) = self.geo_full(&params) else {
                    return vec![Reply::Error("no photo loaded to export".into())];
                };
                match crate::imgio::export::export(
                    src,
                    &params,
                    &tuning,
                    &dest,
                    format,
                    jpeg_quality,
                ) {
                    Ok(()) => vec![Reply::ExportDone(dest)],
                    Err(e) => vec![Reply::Error(format!("export failed: {e:#}"))],
                }
            }
            Cmd::SampleNeutral { params, norm_point } => {
                let Some(src) = self.geo_full(&params) else {
                    return Vec::new();
                };
                let rgb = pipeline::sample_patch(src, norm_point);
                let (temp, tint) = color::neutral_to_temp_tint(rgb);
                vec![Reply::Neutral { temp, tint }]
            }
        }
    }

    fn render_preview(
        &mut self,
        params: &EditParams,
        tuning: &Tuning,
        include_crop: bool,
        clip: ClipFlags,
        overlay: Option<usize>,
    ) -> Vec<Reply> {
        let Some(base) = &self.preview_base else {
            return Vec::new();
        };
        // Refresh the geometry-applied preview cache if needed.
        let sig = params.geo_signature(include_crop);
        let stale = !matches!(&self.preview_geo, Some((s, _)) if *s == sig);
        if stale {
            let img = params
                .has_geometry(include_crop)
                .then(|| geometry::apply(base, params, include_crop));
            self.preview_geo = Some((sig, img));
        }
        let src = match &self.preview_geo {
            Some((_, Some(img))) => img,
            _ => base,
        };
        let ctx = RenderCtx::full(src.width, src.height);
        let rgb = pipeline::render_rgb(src, params, tuning, ctx);
        // The coverage wash travels as its own alpha map rather than being
        // baked into the pixels, so the preview the UI samples for brush
        // auto-masking and the color picker stays the real render.
        let coverage = overlay
            .and_then(|i| params.masks.get(i))
            .map(|m| pipeline::mask_coverage(m, &rgb, src.width, src.height, ctx))
            .map(|cov| cov.iter().map(|w| (w * 255.0 + 0.5) as u8).collect());
        let mut rgba = pipeline::rgb_to_rgba(&rgb);
        let hist = histogram::compute(&rgba);
        histogram::mark_clipping(&mut rgba, clip.0, clip.1);
        let full = self.full.as_ref().expect("preview_base implies full");
        vec![Reply::Preview {
            width: src.width,
            height: src.height,
            rgba,
            full_size: geometry::oriented_dims(full.width, full.height, params, include_crop),
            histogram: hist,
            coverage,
        }]
    }

    /// Get (building if stale) the geometry-applied full-resolution image.
    fn geo_full(&mut self, params: &EditParams) -> Option<&SourceImage> {
        let full = self.full.as_ref()?;
        let sig = params.geo_signature(true);
        let stale = !matches!(&self.full_geo, Some((s, _)) if *s == sig);
        if stale {
            let img = params
                .has_geometry(true)
                .then(|| geometry::apply(full, params, true));
            self.full_geo = Some((sig, img));
        }
        match &self.full_geo {
            Some((_, Some(img))) => Some(img),
            _ => self.full.as_ref(),
        }
    }

    fn render_region(
        &mut self,
        params: &EditParams,
        tuning: &Tuning,
        norm_rect: [f32; 4],
        target: (usize, usize),
        clip: ClipFlags,
    ) -> Vec<Reply> {
        let Some(src) = self.geo_full(params) else {
            return Vec::new();
        };
        let (mut tw, mut th) = (target.0.max(1), target.1.max(1));
        // Never render beyond 1:1 of the source pixels or the size cap.
        let region_w_px = (norm_rect[2] * src.width as f32).max(1.0);
        let region_h_px = (norm_rect[3] * src.height as f32).max(1.0);
        let mut scale = (tw as f32 / region_w_px)
            .min(th as f32 / region_h_px)
            .min(1.0);
        if region_w_px * region_h_px * scale * scale > MAX_REGION_PIXELS as f32 {
            scale *=
                (MAX_REGION_PIXELS as f32 / (region_w_px * region_h_px * scale * scale)).sqrt();
        }
        tw = ((region_w_px * scale) as usize).max(1);
        th = ((region_h_px * scale) as usize).max(1);

        let region_src = src.sample_region(norm_rect, tw, th);
        // Scale blur radii as if rendering the full image at this sampling
        // density, so texture/clarity look consistent with the export.
        let radius_dim = src.width.max(src.height) as f32 * scale;
        let ctx = RenderCtx {
            norm_rect,
            radius_dim,
        };
        let mut rgba = pipeline::render_rgba(&region_src, params, tuning, ctx);
        histogram::mark_clipping(&mut rgba, clip.0, clip.1);
        vec![Reply::Region {
            width: tw,
            height: th,
            rgba,
            norm_rect,
        }]
    }
}
