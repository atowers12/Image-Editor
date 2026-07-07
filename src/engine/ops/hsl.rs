//! The 8-band color mixer: per-hue-range hue shift, saturation, and
//! luminance adjustments, blended smoothly across neighboring bands.

use crate::engine::ops::color::{hsl_to_rgb, rgb_to_hsl};
use crate::engine::params::{EditParams, HSL_BAND_HUES};

/// Angular distance in degrees, wrap-aware, 0..=180.
#[inline]
fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// Smooth membership weight of a hue in a band centered at `center`.
#[inline]
fn band_weight(h: f32, center: f32) -> f32 {
    const WIDTH: f32 = 30.0;
    let d = hue_dist(h, center) / WIDTH;
    (-0.5 * d * d).exp()
}

/// Apply the color mixer to one gamma-encoded RGB pixel.
#[inline]
pub fn apply(px: &mut [f32], p: &EditParams) {
    let (h, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
    if s < 0.02 {
        return; // neutral pixel: no defined hue, leave untouched
    }
    // Fade the whole effect in as saturation rises so near-grays don't pop.
    let sat_gate = ((s - 0.02) / 0.13).clamp(0.0, 1.0);

    let mut hue_shift = 0.0;
    let mut sat_scale = 0.0;
    let mut lum_scale = 0.0;
    for (i, band) in p.hsl.iter().enumerate() {
        if band.is_zero() {
            continue;
        }
        let w = band_weight(h, HSL_BAND_HUES[i]);
        if w < 1e-3 {
            continue;
        }
        hue_shift += w * band.hue * 0.30; // full slider = +/-30 degrees
        sat_scale += w * band.sat / 100.0;
        lum_scale += w * band.lum / 100.0;
    }
    if hue_shift == 0.0 && sat_scale == 0.0 && lum_scale == 0.0 {
        return;
    }

    let nh = h + hue_shift * sat_gate;
    let ns = (s * (1.0 + sat_scale * sat_gate)).clamp(0.0, 1.0);
    // Luminance: positive brightens the color, negative deepens it.
    let nl = (l * (1.0 + 0.6 * lum_scale * sat_gate)).clamp(0.0, 1.0);
    let (r, g, b) = hsl_to_rgb(nh, ns, nl);
    px[0] = r;
    px[1] = g;
    px[2] = b;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_params_leave_pixel_unchanged() {
        let p = EditParams::default();
        let mut px = [0.8, 0.3, 0.2];
        let orig = px;
        apply(&mut px, &p);
        assert_eq!(px, orig);
    }

    #[test]
    fn red_band_desaturation_affects_red_not_blue() {
        let mut p = EditParams::default();
        p.hsl[0].sat = -100.0; // kill reds
        let mut red = [0.9, 0.1, 0.1];
        let mut blue = [0.1, 0.1, 0.9];
        apply(&mut red, &p);
        apply(&mut blue, &p);
        // Red pixel becomes much less saturated.
        let spread_red = red[0] - red[1];
        assert!(spread_red < 0.3, "red spread {spread_red}");
        // Blue pixel essentially untouched.
        assert!((blue[2] - 0.9).abs() < 0.02);
    }
}
