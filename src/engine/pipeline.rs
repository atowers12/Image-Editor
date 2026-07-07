//! The render pipeline: takes a decoded source image (linear RGB f32) and
//! EditParams, produces display/export-ready pixels.
//!
//! Stage order (Lightroom-like):
//!   linear:  white balance -> exposure
//!   encode:  linear -> sRGB gamma
//!   gamma:   tone ranges -> contrast -> texture/clarity ->
//!            color mixer -> vibrance/saturation/dehaze -> vignette

use rayon::prelude::*;

use crate::engine::blur;
use crate::engine::ops::{color, detail, hsl, tone, vignette};
use crate::engine::params::EditParams;

/// A decoded photo held as linear RGB, 3 floats per pixel.
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
            return SourceImage {
                width: self.width,
                height: self.height,
                data: self.data.clone(),
            };
        }
        let scale = max_edge as f32 / long as f32;
        let nw = ((self.width as f32 * scale) as usize).max(1);
        let nh = ((self.height as f32 * scale) as usize).max(1);
        let mut out = vec![0.0f32; nw * nh * 3];
        out.par_chunks_mut(nw * 3).enumerate().for_each(|(y, row)| {
            let sy = (y as f32 + 0.5) / scale - 0.5;
            let y0 = (sy.floor().max(0.0) as usize).min(self.height - 1);
            let y1 = (y0 + 1).min(self.height - 1);
            let fy = (sy - y0 as f32).clamp(0.0, 1.0);
            for x in 0..nw {
                let sx = (x as f32 + 0.5) / scale - 0.5;
                let x0 = (sx.floor().max(0.0) as usize).min(self.width - 1);
                let x1 = (x0 + 1).min(self.width - 1);
                let fx = (sx - x0 as f32).clamp(0.0, 1.0);
                for c in 0..3 {
                    let p00 = self.data[(y0 * self.width + x0) * 3 + c];
                    let p10 = self.data[(y0 * self.width + x1) * 3 + c];
                    let p01 = self.data[(y1 * self.width + x0) * 3 + c];
                    let p11 = self.data[(y1 * self.width + x1) * 3 + c];
                    let top = p00 + (p10 - p00) * fx;
                    let bot = p01 + (p11 - p01) * fx;
                    row[x * 3 + c] = top + (bot - top) * fy;
                }
            }
        });
        SourceImage {
            width: nw,
            height: nh,
            data: out,
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
pub fn render_rgb(src: &SourceImage, p: &EditParams) -> Vec<f32> {
    let w = src.width;
    let h = src.height;
    let mut buf = src.data.clone();

    // --- Pass 1 (per pixel): WB + exposure in linear, encode, tone ---
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

    buf.par_chunks_mut(3).for_each(|px| {
        px[0] = srgb_encode(px[0] * gain_r);
        px[1] = srgb_encode(px[1] * gain_g);
        px[2] = srgb_encode(px[2] * gain_b);
        if any_range {
            let l = tone::luma(px[0], px[1], px[2]);
            let d = tone::range_delta(l, hi, sh, wh, bl);
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
    });

    // --- Pass 2 (spatial): texture + clarity via blurred luminance ---
    let tx = p.texture / 100.0;
    let cl = p.clarity / 100.0;
    if tx != 0.0 || cl != 0.0 {
        let lum: Vec<f32> = buf
            .par_chunks(3)
            .map(|px| tone::luma(px[0], px[1], px[2]))
            .collect();
        let dim = w.max(h) as f32;
        let blur_small = if tx != 0.0 {
            blur::gaussian_approx(&lum, w, h, ((dim * 0.0015) as usize).max(1))
        } else {
            Vec::new()
        };
        let blur_large = if cl != 0.0 {
            blur::gaussian_approx(&lum, w, h, ((dim * 0.010) as usize).max(2))
        } else {
            Vec::new()
        };
        buf.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
            let l = lum[i];
            let bs = if tx != 0.0 { blur_small[i] } else { l };
            let blg = if cl != 0.0 { blur_large[i] } else { l };
            let nl = detail::texture_clarity(l, bs, blg, tx, cl);
            if nl != l {
                let ratio = (nl.max(0.0)) / l.max(1e-5);
                px[0] = (px[0] * ratio).clamp(0.0, 1.0);
                px[1] = (px[1] * ratio).clamp(0.0, 1.0);
                px[2] = (px[2] * ratio).clamp(0.0, 1.0);
            }
        });
    }

    // --- Pass 3 (per pixel, position-aware): mixer, color, dehaze, vignette ---
    let any_hsl = p.any_hsl();
    let vib = p.vibrance / 100.0;
    let dh = p.dehaze / 100.0;
    let sat = p.saturation / 100.0 + detail::dehaze_sat_boost(dh);
    let vig = p.vignette / 100.0;
    if any_hsl || vib != 0.0 || sat != 0.0 || dh != 0.0 || vig != 0.0 {
        buf.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
            for x in 0..w {
                let px = &mut row[x * 3..x * 3 + 3];
                if dh != 0.0 {
                    px[0] = detail::dehaze_channel(px[0], dh);
                    px[1] = detail::dehaze_channel(px[1], dh);
                    px[2] = detail::dehaze_channel(px[2], dh);
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
                if vig != 0.0 {
                    let g = vignette::gain(x, y, w, h, vig);
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

/// Render straight to RGBA bytes for display in egui.
pub fn render_rgba(src: &SourceImage, p: &EditParams) -> Vec<u8> {
    let rgb = render_rgb(src, p);
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

    #[test]
    fn identity_params_only_apply_gamma() {
        let src = test_image();
        let out = render_rgb(&src, &EditParams::default());
        for (o, s) in out.iter().zip(src.data.iter()) {
            assert!((o - srgb_encode(*s)).abs() < 1e-4);
        }
    }

    #[test]
    fn exposure_plus_one_ev_doubles_linear() {
        let src = test_image();
        let mut p = EditParams::default();
        p.exposure = 1.0;
        let out = render_rgb(&src, &p);
        // Check an un-clipped pixel: decode back to linear, expect 2x.
        let lin_out = srgb_decode(out[3]); // pixel 1, red = 1/7 linear
        let expected = 2.0 * (1.0 / 7.0);
        assert!(
            (lin_out - expected).abs() < 1e-3,
            "{lin_out} vs {expected}"
        );
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
}
