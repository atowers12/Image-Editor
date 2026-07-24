//! Applying a mask's `LocalAdjust` to a single gamma-space pixel. The
//! caller computes the "fully adjusted" pixel here, then blends between the
//! original and this result by the mask's per-pixel weight — so a mask at
//! weight 0.5 gives half the effect, and weight 0 leaves the pixel untouched.

use crate::engine::ops::{color, tone};
use crate::engine::params::LocalAdjust;

/// Fixed strength for local highlight/shadow range work (mirrors the global
/// default; local masks don't read the user Tuning).
const LOCAL_RANGE_STRENGTH: f32 = 0.30;

/// Compute the fully-adjusted pixel for a local mask. `l` is the pixel's
/// luma; `blur_large`/`blur_small` are blurred luma at clarity/sharpen radii
/// (pass `l` when the mask doesn't use those, they'll be no-ops).
#[inline]
pub fn target(px: [f32; 3], l: f32, blur_large: f32, blur_small: f32, adj: &LocalAdjust) -> [f32; 3] {
    let mut p = px;

    if adj.exposure != 0.0 {
        let gain = 2f32.powf(adj.exposure / 100.0 * 2.0); // ±2 EV at full slider
        p[0] *= gain;
        p[1] *= gain;
        p[2] *= gain;
    }
    if adj.temp != 0.0 || adj.tint != 0.0 {
        let (gr, gg, gb) = color::white_balance_gains(adj.temp / 100.0, adj.tint / 100.0);
        p[0] *= gr;
        p[1] *= gg;
        p[2] *= gb;
    }

    let (hi, sh, wh, bl) = (
        adj.highlights / 100.0,
        adj.shadows / 100.0,
        adj.whites / 100.0,
        adj.blacks / 100.0,
    );
    if hi != 0.0 || sh != 0.0 || wh != 0.0 || bl != 0.0 {
        let ll = tone::luma(p[0], p[1], p[2]);
        let d = tone::range_delta(ll, hi, sh, wh, bl, LOCAL_RANGE_STRENGTH);
        if d != 0.0 {
            let ratio = (ll + d).max(0.0) / ll.max(1e-5);
            p[0] *= ratio;
            p[1] *= ratio;
            p[2] *= ratio;
        }
    }

    if adj.contrast != 0.0 {
        let c = adj.contrast / 100.0;
        p[0] = tone::contrast(p[0], c);
        p[1] = tone::contrast(p[1], c);
        p[2] = tone::contrast(p[2], c);
    }

    // Local clarity / sharpness modulate luma against blurred luma.
    let cl = adj.clarity / 100.0;
    let sp = adj.sharpness / 100.0;
    if cl != 0.0 || sp != 0.0 {
        let ll = tone::luma(p[0], p[1], p[2]);
        let mut nl = ll;
        if cl != 0.0 {
            let wmid = (1.0 - (2.0 * ll - 1.0).abs()).max(0.0);
            nl += cl * 0.45 * wmid * (ll - blur_large);
        }
        if sp != 0.0 {
            nl += sp * 0.7 * (ll - blur_small);
        }
        if nl != ll {
            let ratio = nl.max(0.0) / ll.max(1e-5);
            p[0] *= ratio;
            p[1] *= ratio;
            p[2] *= ratio;
        }
    }

    if adj.saturation != 0.0 {
        let ll = tone::luma(p[0], p[1], p[2]);
        color::saturate(&mut p, ll, adj.saturation / 100.0);
    }

    let _ = l;
    [
        p[0].clamp(0.0, 1.0),
        p[1].clamp(0.0, 1.0),
        p[2].clamp(0.0, 1.0),
    ]
}

/// Whether a local adjustment needs a large-radius (clarity) blur buffer.
pub fn needs_large_blur(adj: &LocalAdjust) -> bool {
    adj.clarity != 0.0
}

/// Whether a local adjustment needs a small-radius (sharpen) blur buffer.
pub fn needs_small_blur(adj: &LocalAdjust) -> bool {
    adj.sharpness != 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_adjust_is_identity() {
        let px = [0.4, 0.5, 0.6];
        let out = target(px, 0.5, 0.5, 0.5, &LocalAdjust::default());
        assert!((out[0] - px[0]).abs() < 1e-6);
        assert!((out[2] - px[2]).abs() < 1e-6);
    }

    #[test]
    fn positive_exposure_brightens() {
        let px = [0.3, 0.3, 0.3];
        let mut adj = LocalAdjust::default();
        adj.exposure = 50.0;
        let out = target(px, 0.3, 0.3, 0.3, &adj);
        assert!(out[0] > 0.3);
    }

    #[test]
    fn saturation_pulls_from_gray() {
        let px = [0.6, 0.4, 0.4];
        let mut adj = LocalAdjust::default();
        adj.saturation = 100.0;
        let out = target(px, 0.45, 0.45, 0.45, &adj);
        // Red channel moves further from the others.
        assert!(out[0] - out[1] > px[0] - px[1]);
    }

    #[test]
    fn blur_needs_flags() {
        let mut adj = LocalAdjust::default();
        assert!(!needs_large_blur(&adj) && !needs_small_blur(&adj));
        adj.clarity = 20.0;
        adj.sharpness = 10.0;
        assert!(needs_large_blur(&adj) && needs_small_blur(&adj));
    }
}
