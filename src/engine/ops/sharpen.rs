//! Output sharpening and noise reduction, operating on the gamma-encoded
//! RGB buffer.
//!
//! - Sharpening is an unsharp mask on luminance (uses the shared blur).
//! - Luminance NR is a small bilateral filter on luma: it smooths noise
//!   while preserving edges (neighbors across a big luma jump are ignored).
//! - Color NR blurs chroma while keeping luminance crisp — the usual way to
//!   kill the colored speckle in high-ISO shots without softening detail.

use rayon::prelude::*;

use crate::engine::blur;
use crate::engine::ops::tone;

/// Sharpen `buf` in place using an unsharp mask against `blur_small`
/// (blurred luma at the sharpen radius). `amount` is 0..~2.
pub fn unsharp(buf: &mut [f32], lum: &[f32], blur_small: &[f32], amount: f32) {
    if amount <= 0.0 {
        return;
    }
    buf.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let l = lum[i];
        let nl = l + amount * (l - blur_small[i]);
        if nl != l {
            let ratio = nl.max(0.0) / l.max(1e-5);
            px[0] = (px[0] * ratio).clamp(0.0, 1.0);
            px[1] = (px[1] * ratio).clamp(0.0, 1.0);
            px[2] = (px[2] * ratio).clamp(0.0, 1.0);
        }
    });
}

/// Bilateral luminance noise reduction. `amount` 0..1 controls both blend
/// strength and the range sigma (stronger = more aggressive smoothing).
pub fn luminance_nr(buf: &mut [f32], w: usize, h: usize, lum: &[f32], amount: f32) {
    if amount <= 0.0 {
        return;
    }
    const R: isize = 2;
    let range_sigma = 0.04 + 0.16 * amount; // wider = smooths across more contrast
    let inv_2s2 = 1.0 / (2.0 * range_sigma * range_sigma);
    // Precompute spatial weights (5x5 gaussian, sigma ~1.4).
    let mut spatial = [[0.0f32; 5]; 5];
    for (dy, row) in spatial.iter_mut().enumerate() {
        for (dx, wgt) in row.iter_mut().enumerate() {
            let fx = dx as f32 - 2.0;
            let fy = dy as f32 - 2.0;
            *wgt = (-(fx * fx + fy * fy) / (2.0 * 1.4 * 1.4)).exp();
        }
    }

    let smoothed: Vec<f32> = (0..w * h)
        .into_par_iter()
        .map(|idx| {
            let x = (idx % w) as isize;
            let y = (idx / w) as isize;
            let center = lum[idx];
            let mut acc = 0.0;
            let mut wsum = 0.0;
            for dy in -R..=R {
                let yy = (y + dy).clamp(0, h as isize - 1);
                for dx in -R..=R {
                    let xx = (x + dx).clamp(0, w as isize - 1);
                    let s = lum[(yy * w as isize + xx) as usize];
                    let dr = s - center;
                    let wgt =
                        spatial[(dy + R) as usize][(dx + R) as usize] * (-dr * dr * inv_2s2).exp();
                    acc += s * wgt;
                    wsum += wgt;
                }
            }
            acc / wsum.max(1e-6)
        })
        .collect();

    buf.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let l = lum[i];
        let nl = l + (smoothed[i] - l) * amount;
        if nl != l {
            let ratio = nl.max(0.0) / l.max(1e-5);
            px[0] = (px[0] * ratio).clamp(0.0, 1.0);
            px[1] = (px[1] * ratio).clamp(0.0, 1.0);
            px[2] = (px[2] * ratio).clamp(0.0, 1.0);
        }
    });
}

/// Color noise reduction: blur chroma, keep luminance. `amount` 0..1,
/// `dim` the long-edge-equivalent for scaling the blur radius.
pub fn color_nr(buf: &mut [f32], w: usize, h: usize, amount: f32, dim: f32) {
    if amount <= 0.0 {
        return;
    }
    let radius = ((dim * 0.004 * amount) as usize).max(1);
    // Blur each channel, then recombine blurred chroma with sharp luma.
    let (mut cr, mut cg, mut cb) = (
        vec![0.0f32; w * h],
        vec![0.0f32; w * h],
        vec![0.0f32; w * h],
    );
    buf.par_chunks(3)
        .zip(
            cr.par_iter_mut()
                .zip(cg.par_iter_mut().zip(cb.par_iter_mut())),
        )
        .for_each(|(px, (r, (g, b)))| {
            *r = px[0];
            *g = px[1];
            *b = px[2];
        });
    let br = blur::gaussian_approx(&cr, w, h, radius);
    let bg = blur::gaussian_approx(&cg, w, h, radius);
    let bb = blur::gaussian_approx(&cb, w, h, radius);

    buf.par_chunks_mut(3).enumerate().for_each(|(i, px)| {
        let l = tone::luma(px[0], px[1], px[2]);
        let bl = tone::luma(br[i], bg[i], bb[i]);
        // Blurred color = blurred RGB shifted to preserve original luma.
        let tr = (br[i] - bl + l).clamp(0.0, 1.0);
        let tg = (bg[i] - bl + l).clamp(0.0, 1.0);
        let tb = (bb[i] - bl + l).clamp(0.0, 1.0);
        px[0] += (tr - px[0]) * amount;
        px[1] += (tg - px[1]) * amount;
        px[2] += (tb - px[2]) * amount;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsharp_zero_is_noop() {
        let mut buf = vec![0.5, 0.5, 0.5];
        unsharp(&mut buf, &[0.5], &[0.4], 0.0);
        assert_eq!(buf, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn unsharp_boosts_edge() {
        let mut buf = vec![0.6, 0.6, 0.6];
        unsharp(&mut buf, &[0.6], &[0.5], 1.0);
        assert!(buf[0] > 0.6);
    }

    #[test]
    fn luminance_nr_smooths_noise_keeps_flat() {
        // A noisy 8x8 gray patch should come out closer to flat.
        let w = 8;
        let h = 8;
        let mut buf = Vec::new();
        let mut lum = Vec::new();
        for i in 0..w * h {
            let v = if i % 2 == 0 { 0.45 } else { 0.55 };
            buf.extend_from_slice(&[v, v, v]);
            lum.push(v);
        }
        let before_var = variance(&lum);
        luminance_nr(&mut buf, w, h, &lum, 1.0);
        let after: Vec<f32> = buf.chunks(3).map(|p| p[0]).collect();
        assert!(variance(&after) < before_var, "NR should reduce variance");
    }

    #[test]
    fn color_nr_reduces_chroma_variance() {
        // Alternating red/green pixels, same luma-ish; color NR should
        // pull the colors together.
        let w = 8;
        let h = 8;
        let mut buf = Vec::new();
        for i in 0..w * h {
            if i % 2 == 0 {
                buf.extend_from_slice(&[0.6, 0.4, 0.4]);
            } else {
                buf.extend_from_slice(&[0.4, 0.6, 0.4]);
            }
        }
        let before: Vec<f32> = buf.chunks(3).map(|p| p[0] - p[1]).collect();
        color_nr(&mut buf, w, h, 1.0, w as f32 * 4.0);
        let after: Vec<f32> = buf.chunks(3).map(|p| p[0] - p[1]).collect();
        assert!(variance(&after) < variance(&before));
    }

    fn variance(v: &[f32]) -> f32 {
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32
    }
}
