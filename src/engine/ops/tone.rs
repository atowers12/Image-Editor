//! Tonal adjustments: contrast and the four luminance-range sliders
//! (highlights, shadows, whites, blacks). All operate on gamma-encoded
//! values in roughly 0..1.

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
#[inline]
pub fn range_delta(l: f32, highlights: f32, shadows: f32, whites: f32, blacks: f32) -> f32 {
    let mut d = 0.0;
    if highlights != 0.0 {
        d += highlights * 0.30 * range_mask(l, 0.75, 0.22);
    }
    if shadows != 0.0 {
        d += shadows * 0.30 * range_mask(l, 0.25, 0.22);
    }
    if whites != 0.0 {
        d += whites * 0.25 * range_mask(l, 1.0, 0.18);
    }
    if blacks != 0.0 {
        d += blacks * 0.25 * range_mask(l, 0.0, 0.18);
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
        assert!(range_delta(0.8, 1.0, 0.0, 0.0, 0.0) > range_delta(0.2, 1.0, 0.0, 0.0, 0.0));
        // Positive shadows slider lifts dark tones more than bright ones.
        assert!(range_delta(0.2, 0.0, 1.0, 0.0, 0.0) > range_delta(0.8, 0.0, 1.0, 0.0, 0.0));
    }
}
