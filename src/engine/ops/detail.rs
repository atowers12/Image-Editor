//! Local-contrast effects: texture (fine detail), clarity (midtone punch),
//! and dehaze. Texture and clarity are unsharp-mask style operations against
//! blurred luminance at two different radii; dehaze is a global veil removal.

/// New luminance after texture + clarity, given the pixel's luminance and the
/// small/large-radius blurred luminances. Amounts are -1..=1.
#[inline]
pub fn texture_clarity(
    l: f32,
    blur_small: f32,
    blur_large: f32,
    texture: f32,
    clarity: f32,
) -> f32 {
    let mut nl = l;
    if texture != 0.0 {
        nl += texture * 0.55 * (l - blur_small);
    }
    if clarity != 0.0 {
        // Weight toward midtones so sky/shadows don't halo as hard.
        let wmid = (1.0 - (2.0 * l - 1.0).abs()).max(0.0);
        nl += clarity * 0.45 * wmid * (l - blur_large);
    }
    nl
}

/// Dehaze one channel value. Positive removes veil (lowers black point,
/// steepens response), negative adds a hazy lift. `amount` -1..=1.
#[inline]
pub fn dehaze_channel(x: f32, amount: f32) -> f32 {
    if amount > 0.0 {
        let k = 0.30 * amount;
        ((x - k) / (1.0 - k)).max(0.0)
    } else {
        // Blend toward a flat, lifted "veil".
        let veil = 0.30 + x * 0.55;
        x + (veil - x) * (-amount)
    }
}

/// Saturation compensation factor to pair with dehaze (haze removal
/// naturally wants a bit more color).
#[inline]
pub fn dehaze_sat_boost(amount: f32) -> f32 {
    if amount > 0.0 {
        amount * 0.20
    } else {
        amount * 0.15
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_zero() {
        assert_eq!(texture_clarity(0.5, 0.4, 0.6, 0.0, 0.0), 0.5);
        assert_eq!(dehaze_channel(0.5, 0.0), 0.5);
    }

    #[test]
    fn texture_amplifies_local_difference() {
        // Pixel brighter than its neighborhood gets brighter with +texture.
        assert!(texture_clarity(0.6, 0.5, 0.5, 1.0, 0.0) > 0.6);
        // And darker with -texture (smoothing).
        assert!(texture_clarity(0.6, 0.5, 0.5, -1.0, 0.0) < 0.6);
    }

    #[test]
    fn dehaze_lowers_blackpoint() {
        assert!(dehaze_channel(0.3, 1.0) < 0.3);
        assert_eq!(dehaze_channel(0.0, 1.0), 0.0);
    }
}
