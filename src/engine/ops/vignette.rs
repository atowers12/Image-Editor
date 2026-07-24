//! Post-crop-style vignette: radial luminance falloff. Negative darkens
//! corners (the classic look), positive brightens them. Takes normalized
//! image coordinates so it stays correct when rendering a zoomed-in region
//! of the full image. Shape (midpoint/feather/strength) comes from Tuning.

/// Multiplicative gain at normalized position (nx, ny) in 0..1 of the whole
/// image. `amount` -1..=1.
#[inline]
pub fn gain(nx: f32, ny: f32, amount: f32, strength: f32, midpoint: f32, feather: f32) -> f32 {
    let dx = nx * 2.0 - 1.0;
    let dy = ny * 2.0 - 1.0;
    // Normalized so r = 1 at the corners.
    let r = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
    let t = ((r - midpoint) / feather.max(0.01)).clamp(0.0, 1.0);
    let mask = t * t * (3.0 - 2.0 * t);
    1.0 + amount * strength * mask
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: f32 = 0.85;
    const M: f32 = 0.35;
    const F: f32 = 0.65;

    #[test]
    fn center_unaffected_corners_darkened() {
        let g_center = gain(0.5, 0.5, -1.0, S, M, F);
        let g_corner = gain(0.0, 0.0, -1.0, S, M, F);
        assert!((g_center - 1.0).abs() < 1e-3);
        assert!(g_corner < 0.5);
    }

    #[test]
    fn zero_amount_is_identity() {
        assert_eq!(gain(0.0, 0.0, 0.0, S, M, F), 1.0);
    }

    #[test]
    fn midpoint_moves_falloff_inward() {
        // Smaller midpoint = vignette reaches farther toward center.
        let wide = gain(0.3, 0.5, -1.0, S, 0.1, F);
        let tight = gain(0.3, 0.5, -1.0, S, 0.7, F);
        assert!(wide < tight);
    }
}
