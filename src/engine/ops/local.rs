//! Applying a mask's `LocalAdjust` to a single gamma-space pixel. The
//! caller computes the "fully adjusted" pixel here, then blends between the
//! original and this result by the mask's per-pixel weight — so a mask at
//! weight 0.5 gives half the effect, and weight 0 leaves the pixel untouched.

use crate::engine::ops::{color, detail, tone};
use crate::engine::params::LocalAdjust;

/// Fixed strengths for local effects. Global equivalents read the user's
/// Tuning; local masks use these so a mask behaves the same whatever the
/// global sliders are set to.
const LOCAL_RANGE_STRENGTH: f32 = 0.30;
const LOCAL_TEXTURE_STRENGTH: f32 = 0.55;
const LOCAL_CLARITY_STRENGTH: f32 = 0.45;
const LOCAL_SHARPEN_STRENGTH: f32 = 0.70;
const LOCAL_DEHAZE_STRENGTH: f32 = 0.30;

/// The spatial inputs a local adjustment needs beyond the pixel itself:
/// blurred luminance at three radii, and the pixel as it would look fully
/// denoised. The pipeline computes these once per render for the whole
/// buffer and hands out one entry per pixel.
#[derive(Clone, Copy)]
pub struct Neighborhood {
    /// Blurred luma at the texture, clarity, and sharpen radii.
    pub blur_texture: f32,
    pub blur_clarity: f32,
    pub blur_sharpen: f32,
    /// This pixel after full-strength luminance noise reduction.
    pub denoised: [f32; 3],
}

impl Neighborhood {
    /// A neighborhood carrying no spatial information: every blurred value is
    /// the pixel's own luma and the denoised pixel is the pixel itself, so
    /// texture, clarity, sharpness and noise all become no-ops.
    #[inline]
    pub fn flat(px: [f32; 3], l: f32) -> Self {
        Self {
            blur_texture: l,
            blur_clarity: l,
            blur_sharpen: l,
            denoised: px,
        }
    }
}

/// Compute the fully-adjusted pixel for a local mask, in the same stage order
/// as the global pipeline: noise, then light, then color, then detail.
#[inline]
pub fn target(px: [f32; 3], n: &Neighborhood, adj: &LocalAdjust) -> [f32; 3] {
    let mut p = px;

    // Noise reduction first, so later detail work isn't sharpening grain.
    let nr = (adj.noise / 100.0).clamp(0.0, 1.0);
    if nr > 0.0 {
        for c in 0..3 {
            p[c] += (n.denoised[c] - p[c]) * nr;
        }
    }

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

    if adj.dehaze != 0.0 {
        let d = adj.dehaze / 100.0;
        p[0] = detail::dehaze_channel(p[0], d, LOCAL_DEHAZE_STRENGTH);
        p[1] = detail::dehaze_channel(p[1], d, LOCAL_DEHAZE_STRENGTH);
        p[2] = detail::dehaze_channel(p[2], d, LOCAL_DEHAZE_STRENGTH);
    }

    // Texture / clarity / sharpness all modulate luma against blurred luma,
    // each at its own radius.
    let tx = adj.texture / 100.0;
    let cl = adj.clarity / 100.0;
    let sp = adj.sharpness / 100.0;
    if tx != 0.0 || cl != 0.0 || sp != 0.0 {
        let ll = tone::luma(p[0], p[1], p[2]);
        let mut nl = ll;
        if tx != 0.0 {
            nl += tx * LOCAL_TEXTURE_STRENGTH * (ll - n.blur_texture);
        }
        if cl != 0.0 {
            // Weight toward midtones so sky and shadows don't halo as hard.
            let wmid = (1.0 - (2.0 * ll - 1.0).abs()).max(0.0);
            nl += cl * LOCAL_CLARITY_STRENGTH * wmid * (ll - n.blur_clarity);
        }
        if sp != 0.0 {
            nl += sp * LOCAL_SHARPEN_STRENGTH * (ll - n.blur_sharpen);
        }
        if nl != ll {
            let ratio = nl.max(0.0) / ll.max(1e-5);
            p[0] *= ratio;
            p[1] *= ratio;
            p[2] *= ratio;
        }
    }

    if adj.vibrance != 0.0 || adj.saturation != 0.0 {
        let ll = tone::luma(p[0], p[1], p[2]);
        if adj.vibrance != 0.0 {
            color::vibrance(&mut p, ll, adj.vibrance / 100.0);
        }
        if adj.saturation != 0.0 {
            color::saturate(&mut p, ll, adj.saturation / 100.0);
        }
    }

    [
        p[0].clamp(0.0, 1.0),
        p[1].clamp(0.0, 1.0),
        p[2].clamp(0.0, 1.0),
    ]
}

/// Whether a local adjustment needs the texture-radius blur buffer.
pub fn needs_texture_blur(adj: &LocalAdjust) -> bool {
    adj.texture != 0.0
}

/// Whether a local adjustment needs the clarity-radius blur buffer.
pub fn needs_clarity_blur(adj: &LocalAdjust) -> bool {
    adj.clarity != 0.0
}

/// Whether a local adjustment needs the sharpen-radius blur buffer.
pub fn needs_sharpen_blur(adj: &LocalAdjust) -> bool {
    adj.sharpness != 0.0
}

/// Whether a local adjustment needs the denoised copy of the buffer.
pub fn needs_denoise(adj: &LocalAdjust) -> bool {
    adj.noise > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(px: [f32; 3]) -> Neighborhood {
        let l = tone::luma(px[0], px[1], px[2]);
        Neighborhood::flat(px, l)
    }

    #[test]
    fn zero_adjust_is_identity() {
        let px = [0.4, 0.5, 0.6];
        let out = target(px, &flat(px), &LocalAdjust::default());
        assert!((out[0] - px[0]).abs() < 1e-6);
        assert!((out[2] - px[2]).abs() < 1e-6);
    }

    #[test]
    fn positive_exposure_brightens() {
        let px = [0.3, 0.3, 0.3];
        let mut adj = LocalAdjust::default();
        adj.exposure = 50.0;
        let out = target(px, &flat(px), &adj);
        assert!(out[0] > 0.3);
    }

    #[test]
    fn saturation_pulls_from_gray() {
        let px = [0.6, 0.4, 0.4];
        let mut adj = LocalAdjust::default();
        adj.saturation = 100.0;
        let out = target(px, &flat(px), &adj);
        // Red channel moves further from the others.
        assert!(out[0] - out[1] > px[0] - px[1]);
    }

    #[test]
    fn vibrance_boosts_muted_colors_more() {
        let muted = [0.52, 0.48, 0.48];
        let vivid = [0.9, 0.1, 0.1];
        let mut adj = LocalAdjust::default();
        adj.vibrance = 100.0;
        let m_out = target(muted, &flat(muted), &adj);
        let v_out = target(vivid, &flat(vivid), &adj);
        let m_gain = (m_out[0] - m_out[1]) / (muted[0] - muted[1]);
        let v_gain = (v_out[0] - v_out[1]) / (vivid[0] - vivid[1]);
        assert!(
            m_gain > v_gain,
            "muted {m_gain} should gain more than {v_gain}"
        );
    }

    #[test]
    fn noise_blends_toward_the_denoised_pixel() {
        let px = [0.6, 0.6, 0.6];
        let n = Neighborhood {
            denoised: [0.4, 0.4, 0.4],
            ..flat(px)
        };
        let mut adj = LocalAdjust::default();
        adj.noise = 50.0;
        let out = target(px, &n, &adj);
        assert!((out[0] - 0.5).abs() < 1e-5, "got {}", out[0]);
        // At full strength the pixel lands on the denoised value.
        adj.noise = 100.0;
        assert!((target(px, &n, &adj)[0] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn texture_and_sharpness_use_their_own_radii() {
        let px = [0.5, 0.5, 0.5];
        // Fine detail only: the small-radius blur differs, the large one doesn't.
        let n = Neighborhood {
            blur_texture: 0.4,
            blur_clarity: 0.5,
            blur_sharpen: 0.5,
            denoised: px,
        };
        let mut adj = LocalAdjust::default();
        adj.texture = 100.0;
        assert!(target(px, &n, &adj)[0] > 0.5);
        // Clarity sees no difference at its own radius, so it does nothing.
        let mut adj = LocalAdjust::default();
        adj.clarity = 100.0;
        assert!((target(px, &n, &adj)[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn dehaze_lowers_local_blackpoint() {
        let px = [0.3, 0.3, 0.35];
        let mut adj = LocalAdjust::default();
        adj.dehaze = 100.0;
        assert!(target(px, &flat(px), &adj)[0] < 0.3);
    }

    #[test]
    fn blur_needs_flags() {
        let mut adj = LocalAdjust::default();
        assert!(!needs_texture_blur(&adj) && !needs_clarity_blur(&adj));
        assert!(!needs_sharpen_blur(&adj) && !needs_denoise(&adj));
        adj.texture = 10.0;
        adj.clarity = 20.0;
        adj.sharpness = 10.0;
        adj.noise = 30.0;
        assert!(needs_texture_blur(&adj) && needs_clarity_blur(&adj));
        assert!(needs_sharpen_blur(&adj) && needs_denoise(&adj));
    }
}
