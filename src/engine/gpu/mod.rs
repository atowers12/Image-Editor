//! GPU rendering: `pipeline::render_rgb` as wgpu compute shaders.
//!
//! The render worker owns a `Renderer` on a headless device. Source images
//! are uploaded once (the preview base and the full-resolution image, each
//! re-sent only when geometry changes), every render runs the whole pipeline
//! on the GPU, and the finished pixels come back as RGBA8 — so everything
//! downstream (histogram, clipping marks, the color picker, brush
//! auto-masking) is unchanged. Export renders the full image in tiles, each
//! grown by the edit's halo so it matches a full-frame render exactly.
//!
//! The CPU pipeline stays the reference: any GPU error, an image too big for
//! the device, or the user turning the GPU off falls back to it.

mod context;
mod kernels;
mod pool;
mod render;
#[cfg(test)]
mod tests;
mod uniforms;

use anyhow::{anyhow, Result};

pub use context::{NoAdapter, OutOfMemory};
use context::Context;
use kernels::Kernels;
use pool::{Pool, SourceFormat, Texture};
use render::Job;

use crate::engine::params::{EditParams, Mask};
use crate::engine::pipeline::{RenderCtx, SourceImage, StageRadii};
use crate::engine::tiling::{plan_tiles, Tile};
use crate::engine::tuning::Tuning;

/// Outer tile edges tried for export, in order. Each tile keeps at least
/// half its edge, so the halo decides which (if any) fits.
const EXPORT_TILE_EDGES: [usize; 2] = [2048, 4096];

/// Which resident source image to render from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The downscaled, geometry-applied preview base.
    Preview,
    /// The full-resolution, geometry-applied image (zoomed regions).
    Full,
}

/// A rendered buffer, as RGBA8, plus the overlay mask's coverage if one was
/// asked for.
pub struct Rendered {
    pub rgba: Vec<u8>,
    pub coverage: Option<Vec<u8>>,
}

struct Resident {
    key: u64,
    size: (u32, u32),
    texture: Texture,
}

pub struct Renderer {
    gpu: Context,
    kernels: Kernels,
    pool: Pool,
    preview: Option<Resident>,
    full: Option<Resident>,
}

impl Renderer {
    /// Open the best hardware GPU. Fails when there is none — a software
    /// adapter would be slower than the CPU pipeline.
    pub fn new() -> Result<Self> {
        Self::open(false)
    }

    /// Like `new`, but settles for a software adapter (WARP) when there is
    /// no hardware one. For tests, which want *a* GPU path to check.
    pub fn new_allowing_software() -> Result<Self> {
        Self::open(true)
    }

    fn open(allow_software: bool) -> Result<Self> {
        let gpu = Context::new(allow_software)?;
        let kernels = gpu.scoped(|| Ok(Kernels::new(&gpu.device)))?;
        Ok(Renderer {
            gpu,
            kernels,
            pool: Pool::default(),
            preview: None,
            full: None,
        })
    }

    /// Adapter name and backend, for display.
    pub fn label(&self) -> &str {
        self.gpu.label()
    }

    /// Make `slot` hold `img`, uploading only when `key` differs from the
    /// key it was last given. `Ok(false)` means the image doesn't fit on
    /// this GPU (too large a texture, or out of memory): render that slot on
    /// the CPU. `Err` means the GPU itself has failed.
    pub fn set_source(&mut self, slot: Slot, key: u64, img: &SourceImage) -> Result<bool> {
        let held = match slot {
            Slot::Preview => &mut self.preview,
            Slot::Full => &mut self.full,
        };
        if held.as_ref().is_some_and(|r| r.key == key) {
            return Ok(true);
        }
        *held = None; // free the old one before allocating its replacement
        let max = self.gpu.limits.max_texture_dimension_2d as usize;
        if img.width == 0 || img.height == 0 || img.width > max || img.height > max {
            return Ok(false);
        }
        let size = (img.width as u32, img.height as u32);
        let gpu = &self.gpu;
        let uploaded = gpu.scoped(|| {
            let texture = pool::source_texture(&gpu.device, size, SourceFormat::Half);
            pool::upload(gpu, img, [0, 0, img.width, img.height], &texture, SourceFormat::Half);
            Ok(texture)
        });
        match uploaded {
            Ok(texture) => {
                *held = Some(Resident { key, size, texture });
                Ok(true)
            }
            Err(e) if e.is::<OutOfMemory>() => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Render a `size` buffer from `slot`, bilinear-sampling its normalized
    /// `sample` rect (like `SourceImage::sample_region`; the whole source at
    /// its own size reads texels 1:1). `ctx` places the buffer in the full
    /// image, exactly as for `pipeline::render_rgb`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        slot: Slot,
        size: (usize, usize),
        sample: [f32; 4],
        ctx: RenderCtx,
        p: &EditParams,
        t: &Tuning,
        overlay: Option<&Mask>,
    ) -> Result<Rendered> {
        let src = match slot {
            Slot::Preview => &self.preview,
            Slot::Full => &self.full,
        }
        .as_ref()
        .ok_or_else(|| anyhow!("no {slot:?} source on the GPU"))?;
        let (sw, sh) = src.size;
        let (w, h) = (size.0 as u32, size.1 as u32);
        let direct = sample == [0.0, 0.0, 1.0, 1.0] && (w, h) == (sw, sh);
        let sample_px = (!direct).then(|| {
            [
                sample[0] * sw as f32,
                sample[1] * sh as f32,
                sample[2] * sw as f32,
                sample[3] * sh as f32,
            ]
        });
        // A mask that selects nothing washes nothing (pipeline::mask_coverage).
        let wash = overlay.filter(|m| m.has_selection());
        let job = Job {
            src: &src.texture.view,
            src_size: (sw, sh),
            sample: sample_px,
            size: (w, h),
            ctx,
            overlay: wash,
            window: [0, 0, w, h],
        };
        let (gpu, kernels, pool) = (&self.gpu, &self.kernels, &mut self.pool);
        let mut rgba = gpu.scoped(|| render::render(gpu, kernels, pool, &job, p, t))?;
        let coverage = match (overlay, wash) {
            (None, _) => None,
            (Some(_), None) => Some(vec![0; size.0 * size.1]),
            (Some(_), Some(_)) => Some(
                rgba.chunks_exact_mut(4)
                    .map(|px| std::mem::replace(&mut px[3], 255))
                    .collect(),
            ),
        };
        Ok(Rendered { rgba, coverage })
    }

    /// Render `img` (geometry applied) at full resolution for export, as RGB8.
    /// `Ok(None)` means the edit reaches too far for tiles to pay off on this
    /// GPU (or the tiles don't fit in its memory): render it on the CPU.
    pub fn render_export(&mut self, img: &SourceImage, p: &EditParams, t: &Tuning) -> Result<Option<Vec<u8>>> {
        self.render_tiled(img, p, t, &EXPORT_TILE_EDGES)
    }

    fn render_tiled(
        &mut self,
        img: &SourceImage,
        p: &EditParams,
        t: &Tuning,
        edges: &[usize],
    ) -> Result<Option<Vec<u8>>> {
        let (w, h) = (img.width, img.height);
        let halo = StageRadii::new(p, t, w.max(h) as f32).halo();
        let max_edge = self.gpu.limits.max_texture_dimension_2d as usize;
        let Some(tiles) = edges
            .iter()
            .find_map(|&e| plan_tiles(w, h, halo, e.min(max_edge)))
        else {
            return Ok(None);
        };
        let result = self.render_tiles(img, p, t, &tiles);
        // An export's working set is far bigger than the preview's; don't
        // keep it around.
        self.pool.trim();
        match result {
            Err(e) if e.is::<OutOfMemory>() => Ok(None),
            other => other.map(Some),
        }
    }

    fn render_tiles(&mut self, img: &SourceImage, p: &EditParams, t: &Tuning, tiles: &[Tile]) -> Result<Vec<u8>> {
        let (w, h) = (img.width, img.height);
        let tex_w = tiles.iter().map(|t| t.outer[2]).max().unwrap_or(1) as u32;
        let tex_h = tiles.iter().map(|t| t.outer[3]).max().unwrap_or(1) as u32;
        let (gpu, kernels, pool) = (&self.gpu, &self.kernels, &mut self.pool);
        let tile_tex = gpu.scoped(|| {
            Ok(pool::source_texture(
                &gpu.device,
                (tex_w, tex_h),
                SourceFormat::Full,
            ))
        })?;
        let mut out = vec![0u8; w * h * 3];
        for tile in tiles {
            let [_, _, ow, oh] = tile.outer;
            let [ix, iy, iw, ih] = tile.inner_in_outer();
            let job = Job {
                src: &tile_tex.view,
                src_size: (ow as u32, oh as u32),
                sample: None,
                size: (ow as u32, oh as u32),
                ctx: tile.ctx(w, h),
                overlay: None,
                window: [ix as u32, iy as u32, iw as u32, ih as u32],
            };
            let rgba = gpu.scoped(|| {
                pool::upload(gpu, img, tile.outer, &tile_tex, SourceFormat::Full);
                render::render(gpu, kernels, pool, &job, p, t)
            })?;
            for (row, src) in rgba.chunks_exact(iw * 4).take(ih).enumerate() {
                let start = ((tile.inner[1] + row) * w + tile.inner[0]) * 3;
                for (d, s) in out[start..start + iw * 3]
                    .chunks_exact_mut(3)
                    .zip(src.chunks_exact(4))
                {
                    d.copy_from_slice(&s[..3]);
                }
            }
        }
        Ok(out)
    }
}
