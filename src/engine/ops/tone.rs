//! Tonal adjustments: contrast, levels, and the four luminance-range
//! sliders (highlights, shadows, whites, blacks). All operate on
//! gamma-encoded values in roughly 0..1.

/// Rec.709 luma from gamma-encoded RGB — good enough as a perceptual proxy.
#[inline]
pub fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Smooth Gaussian weight centered on a tonal range.
#[inline]
fn range_mask(l: f32, center: f32, width: f32) -> f32 {
    let d = (l - center) / width;
    (-0.5 * d * d).exp()
}

/// Combined luminance delta from the four range sliders (each -1..=1).
/// `strength` is the tuning value for highlights/shadows; whites/blacks
/// use 5/6 of it (they target narrower ranges).
#[inline]
pub fn range_delta(l: f32, highlights: f32, shadows: f32, whites: f32, blacks: f32, strength: f32) -> f32 {
    let end_strength = strength * (5.0 / 6.0);
    let mut d = 0.0;
    if highlights != 0.0 {
        d += highlights * strength * range_mask(l, 0.75, 0.22);
    }
    if shadows != 0.0 {
        d += shadows * strength * range_mask(l, 0.25, 0.22);
    }
    if whites != 0.0 {
        d += whites * end_strength * range_mask(l, 1.0, 0.18);
    }
    if blacks != 0.0 {
        d += blacks * end_strength * range_mask(l, 0.0, 0.18);
    }
    d
}

/// Contrast curve, c in -1..=1. Positive blends toward a smoothstep S-curve
/// around middle gray; negative compresses toward middle gray.
#[inline]
pub fn contrast(x: f32, c: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if c > 0.0 {
        let s = x * x * (3.0 - 2.0 * x);
        x + (s - x) * c
    } else {
        let flat = 0.5 + (x - 0.5) * 0.72;
        x + (flat - x) * (-c)
    }
}

/// Photoshop-style levels: remap [in_black, in_white] to [out_black, out_white]
/// with a midtone gamma. Identity at (0, 1, 1, 0, 1).
#[inline]
pub fn levels(x: f32, in_black: f32, in_white: f32, gamma: f32, out_black: f32, out_white: f32) -> f32 {
    let t = ((x - in_black) / (in_white - in_black).max(1e-4)).clamp(0.0, 1.0);
    let t = t.powf(1.0 / gamma.max(0.05));
    out_black + t * (out_white - out_black)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_identity_at_zero() {
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((contrast(x, 0.0) - x).abs() < 1e-6);
        }
    }

    #[test]
    fn positive_contrast_darkens_shadows_brightens_highlights() {
        assert!(contrast(0.25, 1.0) < 0.25);
        assert!(contrast(0.75, 1.0) > 0.75);
        // Midpoint fixed.
        assert!((contrast(0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn range_delta_targets_correct_zones() {
        // Positive highlights slider lifts bright tones more than dark ones.
        assert!(range_delta(0.8, 1.0, 0.0, 0.0, 0.0, 0.3) > range_delta(0.2, 1.0, 0.0, 0.0, 0.0, 0.3));
        // Positive shadows slider lifts dark tones more than bright ones.
        assert!(range_delta(0.2, 0.0, 1.0, 0.0, 0.0, 0.3) > range_delta(0.8, 0.0, 1.0, 0.0, 0.0, 0.3));
        // Strength scales the effect.
        assert!(range_delta(0.8, 1.0, 0.0, 0.0, 0.0, 0.6) > range_delta(0.8, 1.0, 0.0, 0.0, 0.0, 0.3));
    }

    #[test]
    fn levels_identity() {
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            assert!((levels(x, 0.0, 1.0, 1.0, 0.0, 1.0) - x).abs() < 1e-5);
        }
    }

    #[test]
    fn levels_remaps_endpoints_and_gamma() {
        // Raising input black clips shadows to 0.
        assert_eq!(levels(0.1, 0.2, 1.0, 1.0, 0.0, 1.0), 0.0);
        // Lowering input white clips highlights to 1.
        assert_eq!(levels(0.9, 0.0, 0.8, 1.0, 0.0, 1.0), 1.0);
        // Gamma > 1 brightens midtones.
        assert!(levels(0.5, 0.0, 1.0, 2.0, 0.0, 1.0) > 0.5);
        // Output range compresses.
        assert!((levels(1.0, 0.0, 1.0, 1.0, 0.1, 0.9) - 0.9).abs() < 1e-6);
        assert!((levels(0.0, 0.0, 1.0, 1.0, 0.1, 0.9) - 0.1).abs() < 1e-6);
    }
}
