//! Color adjustments: white balance gains, vibrance, saturation, and
//! RGB <-> HSL conversion helpers shared with the color mixer.

/// Per-channel linear-space gains for temperature/tint sliders (-1..=1 each).
/// Positive temp warms (more red, less blue); positive tint shifts magenta.
pub fn white_balance_gains(temp: f32, tint: f32) -> (f32, f32, f32) {
    let r = 2f32.powf(0.35 * temp + 0.15 * tint);
    let g = 2f32.powf(-0.20 * tint);
    let b = 2f32.powf(-0.35 * temp + 0.15 * tint);
    (r, g, b)
}

/// Given a sampled linear RGB that should be neutral gray, solve for the
/// (temp, tint) slider values in -100..=100 that neutralize it — the
/// white-balance eyedropper. Inverts `white_balance_gains`.
pub fn neutral_to_temp_tint(rgb: [f32; 3]) -> (f32, f32) {
    let eps = 1e-4;
    let lr = rgb[0].max(eps).log2();
    let lg = rgb[1].max(eps).log2();
    let lb = rgb[2].max(eps).log2();
    // From white_balance_gains, requiring r*gr = g*gg = b*gb:
    //   temp_norm = (B - R) / 0.70
    //   tint_norm = (2G - R - B) / 0.70
    let temp = ((lb - lr) / 0.70 * 100.0).clamp(-100.0, 100.0);
    let tint = ((2.0 * lg - lr - lb) / 0.70 * 100.0).clamp(-100.0, 100.0);
    (temp, tint)
}

/// Uniform saturation scale around luma. `amount` -1..=1.
#[inline]
pub fn saturate(px: &mut [f32], l: f32, amount: f32) {
    let k = 1.0 + amount;
    for c in px.iter_mut() {
        *c = l + (*c - l) * k;
    }
}

/// Vibrance: boosts muted colors more than already-saturated ones and
/// partially protects skin tones (orange hues). `amount` -1..=1.
#[inline]
pub fn vibrance(px: &mut [f32], l: f32, amount: f32) {
    let max = px[0].max(px[1]).max(px[2]);
    let min = px[0].min(px[1]).min(px[2]);
    let sat = if max > 1e-5 { (max - min) / max } else { 0.0 };
    let mut k = amount * (1.0 - sat);
    if amount > 0.0 {
        // Skin protection: damp the boost for orange-ish hues.
        let (h, s, _) = rgb_to_hsl(px[0], px[1], px[2]);
        if s > 0.05 && h > 10.0 && h < 50.0 {
            k *= 0.35;
        }
    }
    saturate(px, l, k);
}

/// RGB (0..1 gamma) -> (hue degrees 0..360, saturation 0..1, lightness 0..1).
#[inline]
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = 0.5 * (max + min);
    let d = max - min;
    if d < 1e-6 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s.min(1.0), l)
}

/// Inverse of `rgb_to_hsl`.
#[inline]
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (r + m, g + m, b + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wb_identity_at_zero() {
        let (r, g, b) = white_balance_gains(0.0, 0.0);
        assert!((r - 1.0).abs() < 1e-6 && (g - 1.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hsl_round_trip() {
        for &(r, g, b) in &[
            (0.8, 0.2, 0.1),
            (0.1, 0.9, 0.5),
            (0.3, 0.3, 0.9),
            (0.5, 0.5, 0.5),
        ] {
            let (h, s, l) = rgb_to_hsl(r, g, b);
            let (r2, g2, b2) = hsl_to_rgb(h, s, l);
            assert!((r - r2).abs() < 1e-4, "{r} vs {r2}");
            assert!((g - g2).abs() < 1e-4, "{g} vs {g2}");
            assert!((b - b2).abs() < 1e-4, "{b} vs {b2}");
        }
    }

    #[test]
    fn eyedropper_neutralizes_sampled_color() {
        // A bluish pixel: applying the solved gains should equalize channels.
        let sample = [0.3f32, 0.35, 0.5];
        let (temp, tint) = neutral_to_temp_tint(sample);
        let (gr, gg, gb) = white_balance_gains(temp / 100.0, tint / 100.0);
        let (r, g, b) = (sample[0] * gr, sample[1] * gg, sample[2] * gb);
        assert!((r - g).abs() < 0.02, "{r} vs {g}");
        assert!((g - b).abs() < 0.02, "{g} vs {b}");
        // Bluish input should push temp warm (positive).
        assert!(temp > 0.0);
    }

    #[test]
    fn neutral_gray_needs_no_correction() {
        let (temp, tint) = neutral_to_temp_tint([0.5, 0.5, 0.5]);
        assert!(temp.abs() < 1.0 && tint.abs() < 1.0);
    }

    #[test]
    fn saturation_zero_is_identity() {
        let mut px = [0.7, 0.4, 0.2];
        let l = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
        saturate(&mut px, l, 0.0);
        assert!((px[0] - 0.7).abs() < 1e-6);
    }
}
