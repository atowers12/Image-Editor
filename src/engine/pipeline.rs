//! The render pipeline: takes a decoded source image (linear RGB f32) and
//! EditParams, produces display/export-ready pixels.
//!
//! Stage order (Lightroom-like):
//!   geometry: orientation -> straighten -> crop   (see ops/geometry.rs)
//!   linear:   white balance -> exposure
//!   encode:   linear -> sRGB gamma
//!   gamma:    tone ranges -> contrast -> levels -> texture/clarity ->
//!             color mixer -> vibrance/saturation/dehaze -> vignette
//!
//! Rendering is region-aware: `RenderCtx` says which part of the full
//! (geometry-applied) image the buffer covers, so position-dependent ops
//! (vignette) and resolution-dependent ops (blur radii) stay consistent
//! between the fit-to-window preview, zoomed-in full-res regions, and export.

use rayon::prelude::*;

use crate::engine::blur;
use crate::engine::ops::{color, curve, detail, hsl, local, mask, sharpen, tone, vignette};
use crate::engine::params::{EditParams, LocalAdjust, Mask, MaskKind, MaskOp, RangeMask};
use crate::engine::tuning::Tuning;

/// A decoded photo held as linear RGB, 3 floats per pixel.
#[derive(Clone)]
pub struct SourceImage {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl SourceImage {
    /// Downscaled copy for interactive previews (bilinear).
    pub fn downscale(&self, max_edge: usize) -> SourceImage {
        let long = self.width.max(self.height);
        if long <= max_edge {
            return self.clone();
        }
        let scale = max_edge as f32 / long as f32;
        let nw = ((self.width as f32 * scale) as usize).max(1);
        let nh = ((self.height as f32 * scale) as usize).max(1);
        self.sample_region([0.0, 0.0, 1.0, 1.0], nw, nh)
    }

    /// Bilinear-sample a normalized sub-rect (x, y, w, h in 0..1) of this
    /// image into a `target_w` x `target_h` buffer. Cost is proportional to
    /// the *target* size, not the region size — this is what makes zoomed
    /// region rendering fast regardless of source resolution.
    pub fn sample_region(&self, rect: [f32; 4], target_w: usize, target_h: usize) -> SourceImage {
        let (sw, sh) = (self.width, self.height);
        let x0 = rect[0] * sw as f32;
        let y0 = rect[1] * sh as f32;
        let rw = rect[2] * sw as f32;
        let rh = rect[3] * sh as f32;
        let mut out = vec![0.0f32; target_w * target_h * 3];
        out.par_chunks_mut(target_w * 3)
            .enumerate()
            .for_each(|(y, row)| {
                let sy = y0 + (y as f32 + 0.5) / target_h as f32 * rh - 0.5;
                let iy0 = (sy.floor().max(0.0) as usize).min(sh - 1);
                let iy1 = (iy0 + 1).min(sh - 1);
                let fy = (sy - iy0 as f32).clamp(0.0, 1.0);
                for x in 0..target_w {
                    let sx = x0 + (x as f32 + 0.5) / target_w as f32 * rw - 0.5;
                    let ix0 = (sx.floor().max(0.0) as usize).min(sw - 1);
                    let ix1 = (ix0 + 1).min(sw - 1);
                    let fx = (sx - ix0 as f32).clamp(0.0, 1.0);
                    for c in 0..3 {
                        let p00 = self.data[(iy0 * sw + ix0) * 3 + c];
                        let p10 = self.data[(iy0 * sw + ix1) * 3 + c];
                        let p01 = self.data[(iy1 * sw + ix0) * 3 + c];
                        let p11 = self.data[(iy1 * sw + ix1) * 3 + c];
                        let top = p00 + (p10 - p00) * fx;
                        let bot = p01 + (p11 - p01) * fx;
                        row[x * 3 + c] = top + (bot - top) * fy;
                    }
                }
            });
        SourceImage {
            width: target_w,
            height: target_h,
            data: out,
        }
    }
}

/// Where the buffer being rendered sits inside the full geometry-applied
/// image, and what dimension blur radii should be scaled against.
#[derive(Clone, Copy)]
pub struct RenderCtx {
    /// Normalized rect (x, y, w, h) of the full image this buffer covers.
    pub norm_rect: [f32; 4],
    /// Long-edge-equivalent dimension for scaling blur radii.
    pub radius_dim: f32,
}

impl RenderCtx {
    /// The buffer covers the whole image (preview and export case).
    pub fn full(width: usize, height: usize) -> Self {
        Self {
            norm_rect: [0.0, 0.0, 1.0, 1.0],
            radius_dim: width.max(height) as f32,
        }
    }
}

#[inline]
pub fn srgb_encode(x: f32) -> f32 {
    if x <= 0.0031308 {
        12.92 * x
    } else {
        1.055 * x.max(0.0).powf(1.0 / 2.4) - 0.055
    }
}

#[inline]
pub fn srgb_decode(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else {
        ((x + 0.055) / 1.055).powf(2.4)
    }
}

/// Render to gamma-encoded RGB f32 (clamped 0..1), 3 floats per pixel.
/// Geometry must already be applied to `src`.
pub fn render_rgb(src: &SourceImage, p: &EditParams, t: &Tuning, ctx: RenderCtx) -> Vec<f32> {
    let w = src.width;
    let h = src.height;
    let mut buf = src.data.clone();

    // --- Pass 1 (per pixel): WB + exposure in linear, encode, tone, levels ---
    let exp = 2f32.powf(p.exposure);
    let (wr, wg, wb) = color::white_balance_gains(p.temp / 100.0, p.tint / 100.0);
    let (gain_r, gain_g, gain_b) = (wr * exp, wg * exp, wb * exp);
    let c = p.contrast / 100.0;
    let (hi, sh, wh, bl) = (
        p.highlights / 100.0,
        p.shadows / 100.0,
        p.whites / 100.0,
        p.blacks / 100.0,
    );
    let any_range = hi != 0.0 || sh != 0.0 || wh != 0.0 || bl != 0.0;
    let has_levels = p.has_levels();
    let range_strength = t.tone_range_strength;
    let lv = (
        p.lv_in_black,
        p.lv_in_white,
        p.lv_gamma,
        p.lv_out_black,
        p.lv_out_white,
    );
    let curve_luts = curve::CurveLuts::build(&p.curve);
    let has_curve = !curve_luts.is_identity();

    buf.par_chunks_mut(3).for_each(|px| {
        px[0] = srgb_encode(px[0] * gain_r);
        px[1] = srgb_encode(px[1] * gain_g);
        px[2] = srgb_encode(px[2] * gain_b);
        if any_range {
            let l = tone::luma(px[0], px[1], px[2]);
            let d = tone::range_delta(l, hi, sh, wh, bl, range_strength);
            if d != 0.0 {
                let nl = (l + d).max(0.0);
                let ratio = nl / l.max(1e-5);
                px[0] *= ratio;
                px[1] *= ratio;
                px[2] *= ratio;
            }
        }
        if c != 0.0 {
            px[0] = tone::contrast(px[0], c);
            px[1] = tone::contrast(px[1], c);
            px[2] = tone::contrast(px[2], c);
        } else {
            px[0] = px[0].clamp(0.0, 1.0);
            px[1] = px[1].clamp(0.0, 1.0);
            px[2] = px[2].clamp(0.0, 1.0);
        }
        if has_levels {
            px[0] = tone::levels(px[0], lv.0, lv.1, lv.2, lv.3, lv.4);
            px[1] = tone::levels(px[1], lv.0, lv.1, lv.2, lv.3, lv.4);
            px[2] = tone::levels(px[2], lv.0, lv.1, lv.2, lv.3, lv.4);
        }
        if has_curve {
            px[0] = curve_luts.apply(0, px[0]);
            px[1] = curve_luts.apply(1, px[1]);
            px[2] = curve_luts.apply(2, px[2]);
        }
    });

    // --- Pass 2a (spatial): noise reduction (luma bilateral + chroma blur) ---
    let lum_nr = p.luminance_nr / 100.0;
    let col_nr = p.color_nr / 100.0;
    if lum_nr > 0.0 || col_nr > 0.0 {
        if lum_nr > 0.0 {
            let lum: Vec<f32> = buf
                .par_chunks(3)
                .map(|px| tone::luma(px[0], px[1], px[2]))
                .collect();
            sharpen::luminance_nr(&mut buf, w, h, &lum, lum_nr);
        }
        if col_nr > 0.0 {
            sharpen::color_nr(&mut buf, w, h, col_nr, ctx.radius_dim);
        }
    }

    // --- Pass 2b (spatial): texture + clarity + sharpen via blurred luma ---
    let tx = p.texture / 100.0;
    let cl = p.clarity / 100.0;
    let sharp = p.sharpen / 100.0 * 1.5; // full slider ≈ 1.5× unsharp
    if tx != 0.0 || cl != 0.0 || sharp > 0.0 {
        let lum: Vec<f32> = buf
            .par_chunks(3)
            .map(|px| tone::luma(px[0], px[1], px[2]))
            .collect();
        let dim = ctx.radius_dim;
        let blur_small = if tx != 0.0 {
            blur::gaussian_approx(&lum, w, h, ((dim * t.texture_radius) as usize).max(1))
        } else {
            Vec::new()
        };
        let blur_large = if cl != 0.0 {
            blur::gaussian_approx(&lum, w, h, ((dim * t.clarity_radius) as usize).max(2))
        } else {
            Vec::new()
        };
        if tx != 0.0 || cl != 0.0 {
            let (tx_s, cl_s) = (t.texture_strength, t.clarity_strength);
            buf.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
                let l = lum[i];
                let bs = if tx != 0.0 { blur_small[i] } else { l };
                let blg = if cl != 0.0 { blur_large[i] } else { l };
                let nl = detail::texture_clarity(l, bs, blg, tx, cl, tx_s, cl_s);
                if nl != l {
                    let ratio = (nl.max(0.0)) / l.max(1e-5);
                    px[0] = (px[0] * ratio).clamp(0.0, 1.0);
                    px[1] = (px[1] * ratio).clamp(0.0, 1.0);
                    px[2] = (px[2] * ratio).clamp(0.0, 1.0);
                }
            });
        }
        if sharp > 0.0 {
            // Sharpen radius from the user slider (in px of the render).
            let radius = ((dim * 0.001 * p.sharpen_radius) as usize).max(1);
            let sharp_lum: Vec<f32> = buf
                .par_chunks(3)
                .map(|px| tone::luma(px[0], px[1], px[2]))
                .collect();
            let sharp_blur = blur::gaussian_approx(&sharp_lum, w, h, radius);
            sharpen::unsharp(&mut buf, &sharp_lum, &sharp_blur, sharp);
        }
    }

    // --- Pass 3 (per pixel, position-aware): mixer, color, dehaze,
    //     local masked adjustments, vignette ---
    let any_hsl = p.any_hsl();
    let vib = p.vibrance / 100.0;
    let dh = p.dehaze / 100.0;
    let sat = p.saturation / 100.0 + detail::dehaze_sat_boost(dh, t.dehaze_sat);
    let vig = p.vignette / 100.0;

    // Prepare local adjustment masks (brush coverage + blur buffers).
    let local = LocalPass::prepare(p, &buf, w, h, ctx, t);

    if any_hsl || vib != 0.0 || sat != 0.0 || dh != 0.0 || vig != 0.0 || local.active() {
        // Map buffer pixel coords to normalized full-image coords.
        let nx0 = ctx.norm_rect[0];
        let ny0 = ctx.norm_rect[1];
        let nxs = ctx.norm_rect[2] / w as f32;
        let nys = ctx.norm_rect[3] / h as f32;
        let (dh_s, vg_s, vg_m, vg_f) = (
            t.dehaze_strength,
            t.vignette_strength,
            t.vignette_midpoint,
            t.vignette_feather,
        );
        buf.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
            let ny = ny0 + (y as f32 + 0.5) * nys;
            for x in 0..w {
                let i = y * w + x;
                let nx = nx0 + (x as f32 + 0.5) * nxs;
                let px = &mut row[x * 3..x * 3 + 3];
                if dh != 0.0 {
                    px[0] = detail::dehaze_channel(px[0], dh, dh_s);
                    px[1] = detail::dehaze_channel(px[1], dh, dh_s);
                    px[2] = detail::dehaze_channel(px[2], dh, dh_s);
                }
                if any_hsl {
                    hsl::apply(px, p);
                }
                if vib != 0.0 || sat != 0.0 {
                    let l = tone::luma(px[0], px[1], px[2]);
                    if vib != 0.0 {
                        color::vibrance(px, l, vib);
                    }
                    if sat != 0.0 {
                        color::saturate(px, l, sat);
                    }
                }
                if local.active() {
                    local.apply(px, i, nx, ny);
                }
                if vig != 0.0 {
                    let g = vignette::gain(nx, ny, vig, vg_s, vg_m, vg_f);
                    px[0] *= g;
                    px[1] *= g;
                    px[2] *= g;
                }
                px[0] = px[0].clamp(0.0, 1.0);
                px[1] = px[1].clamp(0.0, 1.0);
                px[2] = px[2].clamp(0.0, 1.0);
            }
        });
    }

    buf
}

/// One prepared shape inside a mask: its geometry, how it combines with the
/// shapes above it, and (for brush shapes) a rasterized coverage buffer.
struct PreparedComponent {
    kind: MaskKind,
    op: MaskOp,
    inverted: bool,
    brush: Option<Vec<f32>>,
}

impl PreparedComponent {
    #[inline]
    fn coverage(&self, i: usize, nx: f32, ny: f32) -> f32 {
        let raw = match &self.brush {
            Some(cov) => cov[i],
            None => mask::weight_at(&self.kind, nx, ny),
        };
        mask::invert_if(raw, self.inverted)
    }
}

/// One prepared local-adjustment mask: its composed shapes, its range mask,
/// and the adjustment they gate.
struct PreparedMask {
    components: Vec<PreparedComponent>,
    range: RangeMask,
    range_active: bool,
    adjust: LocalAdjust,
    inverted: bool,
}

impl PreparedMask {
    fn new(m: &Mask, buf: &[f32], w: usize, h: usize, ctx: RenderCtx) -> Self {
        let components = m
            .components
            .iter()
            .map(|c| PreparedComponent {
                kind: c.kind.clone(),
                op: c.op,
                inverted: c.inverted,
                brush: match &c.kind {
                    MaskKind::Brush { dabs } => {
                        Some(mask::rasterize_brush(dabs, w, h, ctx.norm_rect, buf))
                    }
                    _ => None,
                },
            })
            .collect();
        PreparedMask {
            components,
            range: m.range,
            range_active: m.range.is_active(),
            adjust: m.adjust,
            inverted: m.inverted,
        }
    }

    /// How strongly this mask selects the pixel at buffer index `i`, sitting
    /// at normalized full-image coords (nx, ny).
    #[inline]
    fn weight(&self, px: [f32; 3], i: usize, nx: f32, ny: f32) -> f32 {
        // Shapes compose into a base weight; a mask with no shapes covers the
        // whole frame and relies on its range mask to narrow it down.
        let base = if self.components.is_empty() {
            1.0
        } else {
            mask::fold(
                self.components
                    .iter()
                    .map(|c| (c.op, c.coverage(i, nx, ny))),
            )
        };
        let mut weight = mask::invert_if(base, self.inverted);
        if self.range_active && weight > 1e-4 {
            weight *= mask::range_weight(px, &self.range);
        }
        weight
    }
}

/// All local (masked) adjustments prepared for the position-aware pass:
/// brush coverage buffers, the blurred-luma buffers texture/clarity/sharpness
/// need, and a denoised copy of the buffer for local noise reduction.
struct LocalPass {
    masks: Vec<PreparedMask>,
    blur_texture: Vec<f32>,
    blur_clarity: Vec<f32>,
    blur_sharpen: Vec<f32>,
    denoised: Vec<f32>,
}

impl LocalPass {
    fn prepare(
        p: &EditParams,
        buf: &[f32],
        w: usize,
        h: usize,
        ctx: RenderCtx,
        t: &Tuning,
    ) -> Self {
        let masks: Vec<PreparedMask> = p
            .active_masks()
            .map(|m| PreparedMask::new(m, buf, w, h, ctx))
            .collect();

        let needs = |f: fn(&LocalAdjust) -> bool| masks.iter().any(|m| f(&m.adjust));
        let (need_tx, need_cl) = (
            needs(local::needs_texture_blur),
            needs(local::needs_clarity_blur),
        );
        let need_sp = needs(local::needs_sharpen_blur);
        let need_nr = needs(local::needs_denoise);

        let dim = ctx.radius_dim;
        let lum = (need_tx || need_cl || need_sp || need_nr).then(|| {
            buf.par_chunks(3)
                .map(|px| tone::luma(px[0], px[1], px[2]))
                .collect::<Vec<f32>>()
        });
        let blur_at = |on: bool, radius: usize| match (&lum, on) {
            (Some(l), true) => blur::gaussian_approx(l, w, h, radius),
            _ => Vec::new(),
        };
        let blur_texture = blur_at(need_tx, ((dim * t.texture_radius) as usize).max(1));
        let blur_clarity = blur_at(need_cl, ((dim * t.clarity_radius) as usize).max(2));
        let blur_sharpen = blur_at(need_sp, ((dim * 0.0015) as usize).max(1));
        // The denoised copy is computed at full strength; each mask's Noise
        // slider then decides how far toward it that mask's pixels travel.
        let denoised = match (&lum, need_nr) {
            (Some(l), true) => {
                let mut d = buf.to_vec();
                sharpen::luminance_nr(&mut d, w, h, l, 1.0);
                d
            }
            _ => Vec::new(),
        };

        LocalPass {
            masks,
            blur_texture,
            blur_clarity,
            blur_sharpen,
            denoised,
        }
    }

    #[inline]
    fn active(&self) -> bool {
        !self.masks.is_empty()
    }

    /// The spatial inputs for the pixel at buffer index `i`, falling back to
    /// the pixel's own values wherever a buffer wasn't needed.
    #[inline]
    fn neighborhood(&self, px: [f32; 3], l: f32, i: usize) -> local::Neighborhood {
        let at = |b: &Vec<f32>| if b.is_empty() { l } else { b[i] };
        local::Neighborhood {
            blur_texture: at(&self.blur_texture),
            blur_clarity: at(&self.blur_clarity),
            blur_sharpen: at(&self.blur_sharpen),
            denoised: if self.denoised.is_empty() {
                px
            } else {
                [
                    self.denoised[i * 3],
                    self.denoised[i * 3 + 1],
                    self.denoised[i * 3 + 2],
                ]
            },
        }
    }

    /// Apply every active mask to one pixel at buffer index `i` / normalized
    /// full-image coords (nx, ny).
    #[inline]
    fn apply(&self, px: &mut [f32], i: usize, nx: f32, ny: f32) {
        for m in &self.masks {
            let weight = m.weight([px[0], px[1], px[2]], i, nx, ny);
            if weight <= 1e-4 {
                continue;
            }
            let pixel = [px[0], px[1], px[2]];
            let l = tone::luma(px[0], px[1], px[2]);
            let n = self.neighborhood(pixel, l, i);
            let target = local::target(pixel, &n, &m.adjust);
            px[0] += (target[0] - px[0]) * weight;
            px[1] += (target[1] - px[1]) * weight;
            px[2] += (target[2] - px[2]) * weight;
        }
    }
}

/// Average linear RGB in a small patch around a normalized point — used by
/// the white-balance eyedropper. Operates on the geometry-applied source.
pub fn sample_patch(src: &SourceImage, norm_point: [f32; 2]) -> [f32; 3] {
    let cx = (norm_point[0] * src.width as f32) as isize;
    let cy = (norm_point[1] * src.height as f32) as isize;
    let r = 3isize;
    let mut sum = [0.0f64; 3];
    let mut n = 0u32;
    for dy in -r..=r {
        let y = (cy + dy).clamp(0, src.height as isize - 1);
        for dx in -r..=r {
            let x = (cx + dx).clamp(0, src.width as isize - 1);
            let idx = (y as usize * src.width + x as usize) * 3;
            sum[0] += src.data[idx] as f64;
            sum[1] += src.data[idx + 1] as f64;
            sum[2] += src.data[idx + 2] as f64;
            n += 1;
        }
    }
    let n = n.max(1) as f64;
    [
        (sum[0] / n) as f32,
        (sum[1] / n) as f32,
        (sum[2] / n) as f32,
    ]
}

/// Render straight to RGBA bytes for display in egui.
pub fn render_rgba(src: &SourceImage, p: &EditParams, t: &Tuning, ctx: RenderCtx) -> Vec<u8> {
    rgb_to_rgba(&render_rgb(src, p, t, ctx))
}

/// Pack a rendered gamma-space buffer into opaque RGBA bytes.
pub fn rgb_to_rgba(rgb: &[f32]) -> Vec<u8> {
    let mut out = vec![255u8; rgb.len() / 3 * 4];
    out.par_chunks_mut(4)
        .zip(rgb.par_chunks(3))
        .for_each(|(d, s)| {
            d[0] = (s[0] * 255.0 + 0.5) as u8;
            d[1] = (s[1] * 255.0 + 0.5) as u8;
            d[2] = (s[2] * 255.0 + 0.5) as u8;
        });
    out
}

/// Per-pixel coverage of one mask over a rendered (gamma-space) buffer, using
/// the same shape composition, inversion and range logic as the render — this
/// is what the UI washes over the photo so you can see what a mask selects.
///
/// Coverage is measured against the *finished* pixels, so a range mask's wash
/// can differ very slightly from the selection the render itself used, which
/// tests each pixel before that mask's own adjustments land on it.
pub fn mask_coverage(m: &Mask, buf: &[f32], w: usize, h: usize, ctx: RenderCtx) -> Vec<f32> {
    // A mask with neither a shape nor a range selects nothing in the render,
    // so it must not wash the whole frame here either.
    if !m.has_selection() {
        return vec![0.0; w * h];
    }
    let prepared = PreparedMask::new(m, buf, w, h, ctx);
    let nx0 = ctx.norm_rect[0];
    let ny0 = ctx.norm_rect[1];
    let nxs = ctx.norm_rect[2] / w as f32;
    let nys = ctx.norm_rect[3] / h as f32;
    let mut out = vec![0.0f32; w * h];
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let ny = ny0 + (y as f32 + 0.5) * nys;
        for (x, slot) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let px = [buf[i * 3], buf[i * 3 + 1], buf[i * 3 + 2]];
            *slot = prepared
                .weight(px, i, nx0 + (x as f32 + 0.5) * nxs, ny)
                .clamp(0.0, 1.0);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> SourceImage {
        // 4x2 gradient-ish image in linear space.
        let mut data = Vec::new();
        for i in 0..8 {
            let v = i as f32 / 7.0;
            data.extend_from_slice(&[v, v * 0.5, 1.0 - v]);
        }
        SourceImage {
            width: 4,
            height: 2,
            data,
        }
    }

    fn render_full(src: &SourceImage, p: &EditParams) -> Vec<f32> {
        render_rgb(
            src,
            p,
            &Tuning::default(),
            RenderCtx::full(src.width, src.height),
        )
    }

    #[test]
    fn identity_params_only_apply_gamma() {
        let src = test_image();
        let out = render_full(&src, &EditParams::default());
        for (o, s) in out.iter().zip(src.data.iter()) {
            assert!((o - srgb_encode(*s)).abs() < 1e-4);
        }
    }

    #[test]
    fn exposure_plus_one_ev_doubles_linear() {
        let src = test_image();
        let mut p = EditParams::default();
        p.exposure = 1.0;
        let out = render_full(&src, &p);
        // Check an un-clipped pixel: decode back to linear, expect 2x.
        let lin_out = srgb_decode(out[3]); // pixel 1, red = 1/7 linear
        let expected = 2.0 * (1.0 / 7.0);
        assert!((lin_out - expected).abs() < 1e-3, "{lin_out} vs {expected}");
    }

    #[test]
    fn srgb_round_trip() {
        for i in 0..=20 {
            let x = i as f32 / 20.0;
            assert!((srgb_decode(srgb_encode(x)) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn downscale_halves_dimensions() {
        let src = SourceImage {
            width: 100,
            height: 50,
            data: vec![0.5; 100 * 50 * 3],
        };
        let small = src.downscale(50);
        assert_eq!(small.width, 50);
        assert_eq!(small.height, 25);
        assert!((small.data[0] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn sample_region_extracts_correct_area() {
        // 8x8 image where red channel = x coordinate.
        let mut data = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                data.extend_from_slice(&[x as f32, y as f32, 0.0]);
            }
        }
        let src = SourceImage {
            width: 8,
            height: 8,
            data,
        };
        // Right half at 1:1.
        let region = src.sample_region([0.5, 0.0, 0.5, 1.0], 4, 8);
        assert_eq!((region.width, region.height), (4, 8));
        // First column of the region should be around x = 4.
        assert!(
            (region.data[0] - 4.0).abs() < 0.51,
            "got {}",
            region.data[0]
        );
        // Last column around x = 7.
        let last = region.data[(3) * 3];
        assert!((last - 7.0).abs() < 0.51, "got {last}");
    }

    #[test]
    fn vignette_region_matches_full_render() {
        // Rendering the bottom-right quadrant as a region must produce the
        // same vignette as the same pixels in a full render.
        let n = 16;
        let src = SourceImage {
            width: n,
            height: n,
            data: vec![0.4; n * n * 3],
        };
        let mut p = EditParams::default();
        p.vignette = -80.0;
        let t = Tuning::default();
        let full = render_rgb(&src, &p, &t, RenderCtx::full(n, n));

        let quad = src.sample_region([0.5, 0.5, 0.5, 0.5], n / 2, n / 2);
        let ctx = RenderCtx {
            norm_rect: [0.5, 0.5, 0.5, 0.5],
            radius_dim: n as f32,
        };
        let region = render_rgb(&quad, &p, &t, ctx);

        // Compare corresponding pixels (region (0,0) == full (n/2, n/2)).
        for y in 0..n / 2 {
            for x in 0..n / 2 {
                let f = full[((y + n / 2) * n + (x + n / 2)) * 3];
                let r = region[(y * (n / 2) + x) * 3];
                assert!((f - r).abs() < 1e-3, "mismatch at {x},{y}: {f} vs {r}");
            }
        }
    }

    /// A gray n×n image plus one mask, rendered full-frame.
    fn render_with_mask(n: usize, gray: f32, mask: crate::engine::params::Mask) -> Vec<f32> {
        let src = SourceImage {
            width: n,
            height: n,
            data: vec![gray; n * n * 3],
        };
        let mut p = EditParams::default();
        p.masks.push(mask);
        render_rgb(&src, &p, &Tuning::default(), RenderCtx::full(n, n))
    }

    /// A radial mask over the middle of the frame, brightening what it covers.
    fn brightening_radial() -> crate::engine::params::Mask {
        use crate::engine::params::{LocalAdjust, Mask, MaskComponent, MaskKind, MaskOp};
        let mut adj = LocalAdjust::default();
        adj.exposure = 80.0;
        Mask {
            name: "m".into(),
            components: vec![MaskComponent::new(
                MaskKind::Radial {
                    center: [0.5, 0.5],
                    radius: [0.35, 0.35],
                    feather: 0.2,
                },
                MaskOp::Add,
            )],
            adjust: adj,
            ..Mask::default()
        }
    }

    #[test]
    fn local_mask_only_affects_covered_region() {
        let n = 32;
        let out = render_with_mask(n, 0.3, brightening_radial());
        let center = out[((n / 2) * n + n / 2) * 3];
        let corner = out[0];
        assert!(
            center > corner + 0.1,
            "mask center {center} vs corner {corner}"
        );
    }

    #[test]
    fn subtracting_a_component_cuts_a_hole_in_the_mask() {
        use crate::engine::params::{MaskComponent, MaskKind, MaskOp};
        let n = 32;
        let mut mask = brightening_radial();
        // Punch a smaller radial out of the middle of the first one.
        mask.components.push(MaskComponent::new(
            MaskKind::Radial {
                center: [0.5, 0.5],
                radius: [0.12, 0.12],
                feather: 0.1,
            },
            MaskOp::Subtract,
        ));
        let out = render_with_mask(n, 0.3, mask);
        let center = out[((n / 2) * n + n / 2) * 3];
        let mid_ring = out[((n / 2) * n + n / 2 + 8) * 3]; // inside the big radial
        let corner = out[0];
        // The hole is back to the untouched value; the ring around it is lifted.
        assert!(
            (center - corner).abs() < 1e-3,
            "hole {center} vs corner {corner}"
        );
        assert!(
            mid_ring > corner + 0.1,
            "ring {mid_ring} vs corner {corner}"
        );
    }

    #[test]
    fn intersecting_components_keep_only_the_overlap() {
        use crate::engine::params::{MaskComponent, MaskKind, MaskOp};
        let n = 32;
        let mut mask = brightening_radial();
        // Keep only the left half of the radial.
        mask.components.push(MaskComponent::new(
            MaskKind::Linear {
                p0: [0.55, 0.5],
                p1: [0.45, 0.5],
            },
            MaskOp::Intersect,
        ));
        let out = render_with_mask(n, 0.3, mask);
        let row = n / 2;
        let left = out[(row * n + n / 2 - 6) * 3];
        let right = out[(row * n + n / 2 + 6) * 3];
        let corner = out[0];
        assert!(left > corner + 0.1, "left {left} vs corner {corner}");
        assert!(
            (right - corner).abs() < 1e-3,
            "right {right} vs corner {corner}"
        );
    }

    #[test]
    fn a_range_mask_narrows_a_shape_by_pixel_color() {
        use crate::engine::params::{LocalAdjust, Mask};
        // Left half dark, right half bright, no shapes — just a luminance
        // range that admits only the bright half.
        let n = 32;
        let mut data = vec![0.0f32; n * n * 3];
        for y in 0..n {
            for x in 0..n {
                let v = if x < n / 2 { 0.05 } else { 0.6 };
                let i = (y * n + x) * 3;
                data[i] = v;
                data[i + 1] = v;
                data[i + 2] = v;
            }
        }
        let src = SourceImage {
            width: n,
            height: n,
            data,
        };
        let mut adj = LocalAdjust::default();
        adj.exposure = 60.0;
        let mut mask = Mask {
            name: "bright".into(),
            components: Vec::new(),
            adjust: adj,
            ..Mask::default()
        };
        mask.range.lum_enabled = true;
        mask.range.lum_lo = 0.6;
        mask.range.lum_hi = 1.0;
        mask.range.lum_feather = 0.1;

        let mut p = EditParams::default();
        p.masks.push(mask);
        let t = Tuning::default();
        let out = render_rgb(&src, &p, &t, RenderCtx::full(n, n));
        let plain = render_rgb(&src, &EditParams::default(), &t, RenderCtx::full(n, n));
        let row = n / 2;
        let dark = (row * n + 4) * 3;
        let bright = (row * n + n - 4) * 3;
        assert!(
            (out[dark] - plain[dark]).abs() < 1e-3,
            "dark half was touched"
        );
        assert!(
            out[bright] > plain[bright] + 0.05,
            "bright half was not lifted"
        );
    }

    #[test]
    fn local_mask_region_render_matches_the_full_render() {
        // The same guarantee the vignette has: a composed mask with a range
        // must resolve identically whether the whole frame is rendered or
        // just a zoomed-in quadrant of it.
        use crate::engine::params::{MaskComponent, MaskKind, MaskOp};
        let n = 32;
        let mut mask = brightening_radial();
        mask.components.push(MaskComponent::new(
            MaskKind::Linear {
                p0: [0.2, 0.5],
                p1: [0.8, 0.5],
            },
            MaskOp::Intersect,
        ));
        mask.range.lum_enabled = true;
        mask.range.lum_lo = 0.1;
        mask.range.lum_hi = 1.0;

        let src = SourceImage {
            width: n,
            height: n,
            data: vec![0.3; n * n * 3],
        };
        let mut p = EditParams::default();
        p.masks.push(mask);
        let t = Tuning::default();
        let full = render_rgb(&src, &p, &t, RenderCtx::full(n, n));

        let quad = src.sample_region([0.5, 0.5, 0.5, 0.5], n / 2, n / 2);
        let ctx = RenderCtx {
            norm_rect: [0.5, 0.5, 0.5, 0.5],
            radius_dim: n as f32,
        };
        let region = render_rgb(&quad, &p, &t, ctx);
        for y in 0..n / 2 {
            for x in 0..n / 2 {
                let f = full[((y + n / 2) * n + (x + n / 2)) * 3];
                let r = region[(y * (n / 2) + x) * 3];
                assert!((f - r).abs() < 1e-3, "mismatch at {x},{y}: {f} vs {r}");
            }
        }
    }

    #[test]
    fn mask_coverage_reports_what_the_render_adjusts() {
        // The overlay must agree with the pixels: wherever coverage is high
        // the render moved, and wherever it is zero the render did not.
        let n = 32;
        let mask = brightening_radial();
        let src = SourceImage {
            width: n,
            height: n,
            data: vec![0.3; n * n * 3],
        };
        let t = Tuning::default();
        let ctx = RenderCtx::full(n, n);
        let plain = render_rgb(&src, &EditParams::default(), &t, ctx);

        let mut p = EditParams::default();
        p.masks.push(mask.clone());
        let edited = render_rgb(&src, &p, &t, ctx);
        let cov = mask_coverage(&mask, &edited, n, n, ctx);

        assert_eq!(cov.len(), n * n);
        assert!(cov[(n / 2) * n + n / 2] > 0.99, "center should be covered");
        assert_eq!(cov[0], 0.0, "corner should be outside the radial");
        for i in 0..n * n {
            let moved = (edited[i * 3] - plain[i * 3]).abs() > 1e-4;
            assert_eq!(
                moved,
                cov[i] > 1e-4,
                "coverage {} disagrees with the render at pixel {i}",
                cov[i]
            );
        }
    }

    #[test]
    fn mask_coverage_of_an_empty_mask_is_empty() {
        use crate::engine::params::Mask;
        let n = 8;
        let buf = vec![0.5; n * n * 3];
        let ctx = RenderCtx::full(n, n);
        let mut m = Mask::default();
        m.components.clear();
        // No shapes and no range: selects nothing, so washes nothing.
        assert!(mask_coverage(&m, &buf, n, n, ctx).iter().all(|&w| w == 0.0));
        // Add a range and the whole frame becomes fair game again.
        m.range.lum_enabled = true;
        assert!(mask_coverage(&m, &buf, n, n, ctx).iter().any(|&w| w > 0.5));
    }

    #[test]
    fn mask_coverage_sees_inversion_and_subtraction() {
        use crate::engine::params::{MaskComponent, MaskKind, MaskOp};
        let n = 16;
        let buf = vec![0.5; n * n * 3];
        let ctx = RenderCtx::full(n, n);

        let mut mask = brightening_radial();
        let inside = (n / 2) * n + n / 2;
        assert!(mask_coverage(&mask, &buf, n, n, ctx)[inside] > 0.99);

        mask.inverted = true;
        assert!(mask_coverage(&mask, &buf, n, n, ctx)[inside] < 0.01);
        assert!(mask_coverage(&mask, &buf, n, n, ctx)[0] > 0.99);

        // Subtracting a shape that covers everything empties the mask.
        mask.inverted = false;
        mask.components.push(MaskComponent::new(
            MaskKind::Radial {
                center: [0.5, 0.5],
                radius: [5.0, 5.0],
                feather: 0.0,
            },
            MaskOp::Subtract,
        ));
        assert!(mask_coverage(&mask, &buf, n, n, ctx)
            .iter()
            .all(|&w| w < 1e-4));
    }

    #[test]
    fn auto_masked_brush_matches_between_full_and_region_renders() {
        // Auto-mask references are captured at paint time, so a stroke must
        // resolve the same way even when the dab's center lies outside the
        // region being rendered.
        use crate::engine::params::{Dab, LocalAdjust, Mask, MaskComponent, MaskKind, MaskOp};
        let n = 32;
        let mut data = vec![0.0f32; n * n * 3];
        for y in 0..n {
            for x in 0..n {
                let v = if y < n / 2 { 0.2 } else { 0.7 };
                let i = (y * n + x) * 3;
                data[i] = v;
                data[i + 1] = v;
                data[i + 2] = v;
            }
        }
        let src = SourceImage {
            width: n,
            height: n,
            data,
        };
        let mut adj = LocalAdjust::default();
        adj.exposure = 70.0;
        // Dab centered in the top-left quadrant, spilling into all four.
        let dabs = vec![Dab {
            p: [0.35, 0.35],
            radius: 0.5,
            hardness: 0.9,
            erase: false,
            auto: Some([0.48, 0.48, 0.48]), // gamma-space value of linear 0.2
        }];
        let mut p = EditParams::default();
        p.masks.push(Mask {
            name: "brush".into(),
            components: vec![MaskComponent::new(MaskKind::Brush { dabs }, MaskOp::Add)],
            adjust: adj,
            ..Mask::default()
        });
        let t = Tuning::default();
        let full = render_rgb(&src, &p, &t, RenderCtx::full(n, n));

        let quad = src.sample_region([0.5, 0.0, 0.5, 0.5], n / 2, n / 2);
        let ctx = RenderCtx {
            norm_rect: [0.5, 0.0, 0.5, 0.5],
            radius_dim: n as f32,
        };
        let region = render_rgb(&quad, &p, &t, ctx);
        for y in 0..n / 2 {
            for x in 0..n / 2 {
                let f = full[(y * n + (x + n / 2)) * 3];
                let r = region[(y * (n / 2) + x) * 3];
                assert!((f - r).abs() < 1e-3, "mismatch at {x},{y}: {f} vs {r}");
            }
        }
    }

    #[test]
    fn curve_in_pipeline_lifts_midtones() {
        let src = test_image();
        let mut p = EditParams::default();
        p.curve.master = vec![[0.0, 0.0], [0.5, 0.75], [1.0, 1.0]];
        let out = render_full(&src, &p);
        let identity = render_full(&src, &EditParams::default());
        // A midtone pixel should be brighter than without the curve.
        assert!(out[9] > identity[9]);
    }

    #[test]
    fn levels_in_pipeline_raise_blackpoint() {
        let src = test_image();
        let mut p = EditParams::default();
        p.lv_in_black = 0.3;
        let out = render_full(&src, &p);
        // The darkest pixel (linear 0 → gamma 0) must clip to 0 and stay 0;
        // mid pixels must be darker than identity.
        let identity = render_full(&src, &EditParams::default());
        assert!(out[0] <= identity[0]);
        assert!(out[3] < identity[3]);
    }
}
