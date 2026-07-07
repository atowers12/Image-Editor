//! Post-crop-style vignette: radial luminance falloff. Negative darkens
//! corners (the classic look), positive brightens them.

/// Multiplicative gain for a pixel at (x, y). `amount` -1..=1.
#[inline]
pub fn gain(x: usize, y: usize, w: usize, h: usize, amount: f32) -> f32 {
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    let dx = (x as f32 + 0.5 - cx) / cx;
    let dy = (y as f32 + 0.5 - cy) / cy;
    // Normalized so r = 1 at the corners.
    let r = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
    // Smooth feathered mask starting mid-frame.
    let t = ((r - 0.35) / 0.65).clamp(0.0, 1.0);
    let mask = t * t * (3.0 - 2.0 * t);
    1.0 + amount * 0.85 * mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_unaffected_corners_darkened() {
        let g_center = gain(50, 50, 100, 100, -1.0);
        let g_corner = gain(0, 0, 100, 100, -1.0);
        assert!((g_center - 1.0).abs() < 1e-3);
        assert!(g_corner < 0.5);
    }

    #[test]
    fn zero_amount_is_identity() {
        assert_eq!(gain(0, 0, 100, 100, 0.0), 1.0);
    }
}
