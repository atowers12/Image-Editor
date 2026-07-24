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
use crate::engine::params::{EditParams, LocalAdjust, MaskKind};
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
    let lv = (p.lv_in_black, p.lv_in_white, p.lv_gamma, p.lv_out_black, p.lv_out_white);
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
    let local = LocalPass::prepare(p, &buf, w, h, ctx);

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

/// One prepared local-adjustment mask: its geometry, adjustment, inversion,
/// and (for brush masks) a rasterized coverage buffer.
struct PreparedMask {
    kind: MaskKind,
    adjust: LocalAdjust,
    inverted: bool,
    brush: Option<Vec<f32>>,
}

/// All local (masked) adjustments prepared for the position-aware pass:
/// brush coverage buffers plus the blurred-luma buffers local clarity and
/// sharpness need.
struct LocalPass {
    masks: Vec<PreparedMask>,
    blur_large: Vec<f32>,
    blur_small: Vec<f32>,
    have_large: bool,
    have_small: bool,
}

impl LocalPass {
    fn prepare(p: &EditParams, buf: &[f32], w: usize, h: usize, ctx: RenderCtx) -> Self {
        let masks: Vec<PreparedMask> = p
            .active_masks()
            .map(|m| {
                let brush = match &m.kind {
                    MaskKind::Brush { dabs } => {
                        Some(mask::rasterize_brush(dabs, w, h, ctx.norm_rect))
                    }
                    _ => None,
                };
                PreparedMask {
                    kind: m.kind.clone(),
                    adjust: m.adjust,
                    inverted: m.inverted,
                    brush,
                }
            })
            .collect();

        let need_large = masks.iter().any(|m| local::needs_large_blur(&m.adjust));
        let need_small = masks.iter().any(|m| local::needs_small_blur(&m.adjust));
        let (blur_large, blur_small) = if need_large || need_small {
            let lum: Vec<f32> = buf
                .par_chunks(3)
                .map(|px| tone::luma(px[0], px[1], px[2]))
                .collect();
            let dim = ctx.radius_dim;
            let bl = if need_large {
                blur::gaussian_approx(&lum, w, h, ((dim * 0.010) as usize).max(2))
            } else {
                Vec::new()
            };
            let bs = if need_small {
                blur::gaussian_approx(&lum, w, h, ((dim * 0.0015) as usize).max(1))
            } else {
                Vec::new()
            };
            (bl, bs)
        } else {
            (Vec::new(), Vec::new())
        };

        LocalPass {
            masks,
            have_large: need_large,
            have_small: need_small,
            blur_large,
            blur_small,
        }
    }

    #[inline]
    fn active(&self) -> bool {
        !self.masks.is_empty()
    }

    /// Apply every active mask to one pixel at buffer index `i` / normalized
    /// full-image coords (nx, ny).
    #[inline]
    fn apply(&self, px: &mut [f32], i: usize, nx: f32, ny: f32) {
        for m in &self.masks {
            let raw = match &m.brush {
                Some(cov) => cov[i],
                None => mask::weight_at(&m.kind, nx, ny),
            };
            let weight = if m.inverted { 1.0 - raw } else { raw };
            if weight <= 1e-4 {
                continue;
            }
            let l = tone::luma(px[0], px[1], px[2]);
            let blg = if self.have_large { self.blur_large[i] } else { l };
            let bs = if self.have_small { self.blur_small[i] } else { l };
            let target = local::target([px[0], px[1], px[2]], l, blg, bs, &m.adjust);
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
    let rgb = render_rgb(src, p, t, ctx);
    let mut out = vec![255u8; src.width * src.height * 4];
    out.par_chunks_mut(4)
        .zip(rgb.par_chunks(3))
        .for_each(|(d, s)| {
            d[0] = (s[0] * 255.0 + 0.5) as u8;
            d[1] = (s[1] * 255.0 + 0.5) as u8;
            d[2] = (s[2] * 255.0 + 0.5) as u8;
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
        render_rgb(src, p, &Tuning::default(), RenderCtx::full(src.width, src.height))
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
        assert!((region.data[0] - 4.0).abs() < 0.51, "got {}", region.data[0]);
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

    #[test]
    fn local_mask_only_affects_covered_region() {
        use crate::engine::params::{LocalAdjust, Mask, MaskKind};
        // Flat gray image; a radial mask brightening the center.
        let n = 32;
        let src = SourceImage {
            width: n,
            height: n,
            data: vec![0.3; n * n * 3],
        };
        let mut p = EditParams::default();
        let mut adj = LocalAdjust::default();
        adj.exposure = 80.0;
        p.masks.push(Mask {
            name: "m".into(),
            kind: MaskKind::Radial {
                center: [0.5, 0.5],
                radius: [0.2, 0.2],
                feather: 0.4,
            },
            adjust: adj,
            enabled: true,
            inverted: false,
        });
        let out = render_rgb(&src, &p, &Tuning::default(), RenderCtx::full(n, n));
        let center = out[((n / 2) * n + n / 2) * 3];
        let corner = out[0];
        assert!(center > corner + 0.1, "mask center {center} vs corner {corner}");
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
