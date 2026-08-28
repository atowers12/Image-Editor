//! Mask coverage: turning mask components into a per-pixel weight in 0..1.
//!
//! Linear and radial components are analytic — evaluated per pixel from
//! normalized coordinates, so they cost nothing to store and scale to any
//! render resolution. Brush components are rasterized from their dabs into a
//! coverage buffer sized to the current render (dabs are stored in
//! normalized image space, so the same strokes render at any resolution).
//!
//! A mask is a *list* of components folded together with add / subtract /
//! intersect, optionally narrowed further by a `RangeMask` that tests the
//! pixel's own luminance and hue.

use rayon::prelude::*;

use crate::engine::ops::{color, tone};
use crate::engine::params::{Dab, MaskKind, MaskOp, RangeMask};

/// Analytic mask weight at a normalized point (nx, ny in 0..1 of the full
/// image). Brush components return 0 here — use `rasterize_brush` for those.
#[inline]
pub fn weight_at(kind: &MaskKind, nx: f32, ny: f32) -> f32 {
    match kind {
        MaskKind::Linear { p0, p1 } => linear_weight(*p0, *p1, nx, ny),
        MaskKind::Radial {
            center,
            radius,
            feather,
        } => radial_weight(*center, *radius, *feather, nx, ny),
        MaskKind::Brush { .. } => 0.0,
    }
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn linear_weight(p0: [f32; 2], p1: [f32; 2], nx: f32, ny: f32) -> f32 {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-9 {
        return 1.0;
    }
    // Projection of (point - p0) onto the p0->p1 axis, normalized to 0..1.
    let t = ((nx - p0[0]) * dx + (ny - p0[1]) * dy) / len2;
    smoothstep(t)
}

#[inline]
fn radial_weight(center: [f32; 2], radius: [f32; 2], feather: f32, nx: f32, ny: f32) -> f32 {
    let rx = radius[0].max(1e-4);
    let ry = radius[1].max(1e-4);
    let dx = (nx - center[0]) / rx;
    let dy = (ny - center[1]) / ry;
    let d = (dx * dx + dy * dy).sqrt(); // 1.0 at the ellipse edge
                                        // Full effect inside `inner`, ramping to 0 at the edge (d = 1).
    let inner = 1.0 - feather.clamp(0.0, 1.0);
    if d <= inner {
        1.0
    } else if d >= 1.0 {
        0.0
    } else {
        smoothstep(1.0 - (d - inner) / (1.0 - inner).max(1e-4))
    }
}

/// Fold a mask's component coverages into a single weight. The first
/// component establishes the base — its operator is ignored, since there is
/// nothing above it to combine with — and each later one is folded in by its
/// own operator. An empty mask covers nothing.
pub fn fold(parts: impl IntoIterator<Item = (MaskOp, f32)>) -> f32 {
    let mut w = 0.0;
    let mut first = true;
    for (op, c) in parts {
        w = if first { c } else { op.combine(w, c) };
        first = false;
    }
    w
}

/// Flip a weight when a component or mask is inverted.
#[inline]
pub fn invert_if(weight: f32, inverted: bool) -> f32 {
    if inverted {
        1.0 - weight
    } else {
        weight
    }
}

/// How strongly a range mask admits one gamma-space pixel: 1 keeps it,
/// 0 excludes it, in between feathers. A range mask with neither half
/// enabled admits everything.
#[inline]
pub fn range_weight(px: [f32; 3], r: &RangeMask) -> f32 {
    let mut w = 1.0;
    if r.lum_enabled {
        let l = tone::luma(px[0], px[1], px[2]);
        w *= band(l, r.lum_lo, r.lum_hi, r.lum_feather);
    }
    if r.color_enabled && w > 0.0 {
        let (h, s, _) = color::rgb_to_hsl(px[0], px[1], px[2]);
        // Near-gray pixels have no meaningful hue, so gate on saturation
        // first, then on how far the hue is from the target.
        let sat_gate = smoothstep((s - r.sat_min) / SAT_GATE_RAMP);
        let f = r.hue_feather.max(1e-4);
        let hue_gate = smoothstep((r.hue_width + f - hue_distance(h, r.hue)) / f);
        w *= sat_gate * hue_gate;
    }
    w
}

/// Saturation over which a pixel goes from fully excluded (at `sat_min`) to
/// fully admitted by a color range mask.
const SAT_GATE_RAMP: f32 = 0.08;

/// Shortest distance between two hues, in degrees (0..=180).
#[inline]
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// 1 inside [lo, hi], falling to 0 across `feather` on each side.
#[inline]
fn band(x: f32, lo: f32, hi: f32, feather: f32) -> f32 {
    let f = feather.max(1e-4);
    let up = smoothstep((x - lo + f) / f);
    let down = smoothstep((hi + f - x) / f);
    up.min(down)
}

/// Rasterize a brush component's dabs into a `w`×`h` coverage buffer.
/// `norm_rect` is the region of the full image the buffer covers (x, y, w, h
/// in 0..1), and `buf` is the gamma-space RGB being rendered — needed by
/// auto-masked dabs, which only cover pixels close in color to the reference
/// captured when the dab was painted.
pub fn rasterize_brush(
    dabs: &[Dab],
    w: usize,
    h: usize,
    norm_rect: [f32; 4],
    buf: &[f32],
) -> Vec<f32> {
    let mut cov_buf = vec![0.0f32; w * h];
    if dabs.is_empty() {
        return cov_buf;
    }
    let (rx0, ry0, rw, rh) = (norm_rect[0], norm_rect[1], norm_rect[2], norm_rect[3]);
    // Paint dabs in order; erase dabs subtract. Coverage accumulates toward 1.
    cov_buf.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        // Normalized y at this pixel row (pixel centers).
        let ny = ry0 + (y as f32 + 0.5) / h as f32 * rh;
        for x in 0..w {
            let nx = rx0 + (x as f32 + 0.5) / w as f32 * rw;
            let mut cov = 0.0f32;
            for dab in dabs {
                let r = dab.radius.max(1e-4);
                let dx = (nx - dab.p[0]) / r;
                let dy = (ny - dab.p[1]) / r;
                let d = (dx * dx + dy * dy).sqrt();
                if d >= 1.0 {
                    continue;
                }
                // Hardness controls how quickly the dab falls off to its edge.
                let hard = dab.hardness.clamp(0.0, 0.99);
                let mut a = if d <= hard {
                    1.0
                } else {
                    smoothstep(1.0 - (d - hard) / (1.0 - hard))
                };
                if let Some(reference) = dab.auto {
                    let i = (y * w + x) * 3;
                    a *= auto_mask_factor([buf[i], buf[i + 1], buf[i + 2]], reference);
                    if a <= 0.0 {
                        continue;
                    }
                }
                if dab.erase {
                    cov *= 1.0 - a;
                } else {
                    cov += a * (1.0 - cov); // over-composite
                }
            }
            row[x] = cov.clamp(0.0, 1.0);
        }
    });
    cov_buf
}

/// Color distance at which an auto-masked dab starts to fade out, and the
/// distance at which it stops covering entirely.
const AUTO_NEAR: f32 = 0.05;
const AUTO_FAR: f32 = 0.22;

/// How much an auto-masked dab covers a pixel, from how far the pixel's color
/// is from the dab's reference color. Luma-weighted, so a stroke stops at a
/// brightness edge (a roof against sky) as readily as a hue one.
#[inline]
fn auto_mask_factor(px: [f32; 3], reference: [f32; 3]) -> f32 {
    let dl = tone::luma(px[0], px[1], px[2]) - tone::luma(reference[0], reference[1], reference[2]);
    let dr = px[0] - reference[0];
    let dg = px[1] - reference[1];
    let db = px[2] - reference[2];
    // Chroma difference plus a doubled luma term: edges are usually both.
    let dist = (dl * dl * 2.0 + (dr * dr + dg * dg + db * db) / 3.0).sqrt();
    smoothstep((AUTO_FAR - dist) / (AUTO_FAR - AUTO_NEAR))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::params::Dab;

    fn dab(p: [f32; 2], radius: f32, hardness: f32, erase: bool) -> Dab {
        Dab {
            p,
            radius,
            hardness,
            erase,
            auto: None,
        }
    }

    /// A flat mid-gray buffer, for brush tests that don't exercise auto-mask.
    fn flat(w: usize, h: usize) -> Vec<f32> {
        vec![0.5; w * h * 3]
    }

    #[test]
    fn linear_ramps_from_p0_to_p1() {
        let k = MaskKind::Linear {
            p0: [0.0, 0.5],
            p1: [1.0, 0.5],
        };
        assert!(weight_at(&k, 0.0, 0.5) < 0.01);
        assert!(weight_at(&k, 1.0, 0.5) > 0.99);
        assert!((weight_at(&k, 0.5, 0.5) - 0.5).abs() < 0.05);
    }

    #[test]
    fn radial_full_center_zero_outside() {
        let k = MaskKind::Radial {
            center: [0.5, 0.5],
            radius: [0.2, 0.2],
            feather: 0.5,
        };
        assert!((weight_at(&k, 0.5, 0.5) - 1.0).abs() < 1e-4);
        assert!(weight_at(&k, 0.9, 0.9) < 1e-4); // well outside
    }

    #[test]
    fn brush_covers_painted_area() {
        let dabs = vec![dab([0.5, 0.5], 0.25, 0.5, false)];
        let buf = rasterize_brush(&dabs, 20, 20, [0.0, 0.0, 1.0, 1.0], &flat(20, 20));
        // Center pixel painted, corner not.
        assert!(buf[10 * 20 + 10] > 0.9);
        assert!(buf[0] < 0.01);
    }

    #[test]
    fn brush_erase_removes_coverage() {
        let dabs = vec![
            dab([0.5, 0.5], 0.4, 0.9, false),
            dab([0.5, 0.5], 0.4, 0.9, true),
        ];
        let buf = rasterize_brush(&dabs, 16, 16, [0.0, 0.0, 1.0, 1.0], &flat(16, 16));
        assert!(buf[8 * 16 + 8] < 0.1);
    }

    #[test]
    fn inversion_flips_weight() {
        assert_eq!(invert_if(0.3, false), 0.3);
        assert!((invert_if(0.3, true) - 0.7).abs() < 1e-6);
    }

    #[test]
    fn fold_uses_first_component_as_the_base() {
        // The leading operator is ignored — Subtract first still starts at 0.6.
        assert!((fold([(MaskOp::Subtract, 0.6)]) - 0.6).abs() < 1e-6);
        assert!(fold(std::iter::empty()) == 0.0);
    }

    #[test]
    fn fold_composes_in_order() {
        // Radial 1.0, minus a brush at 0.5, intersected with a linear at 0.8.
        let w = fold([
            (MaskOp::Add, 1.0),
            (MaskOp::Subtract, 0.5),
            (MaskOp::Intersect, 0.8),
        ]);
        assert!((w - 0.4).abs() < 1e-6, "got {w}");
    }

    #[test]
    fn default_range_mask_admits_everything() {
        let r = RangeMask::default();
        assert!(!r.is_active());
        for px in [[0.0, 0.0, 0.0], [0.5, 0.2, 0.9], [1.0, 1.0, 1.0]] {
            assert_eq!(range_weight(px, &r), 1.0);
        }
    }

    #[test]
    fn luminance_range_selects_a_band() {
        let mut r = RangeMask::default();
        r.lum_enabled = true;
        r.lum_lo = 0.6;
        r.lum_hi = 1.0;
        r.lum_feather = 0.1;
        // A bright pixel is kept, a dark one excluded.
        assert!(range_weight([0.9, 0.9, 0.9], &r) > 0.99);
        assert!(range_weight([0.2, 0.2, 0.2], &r) < 0.01);
        // And the edge of the band feathers rather than jumping.
        let edge = range_weight([0.55, 0.55, 0.55], &r);
        assert!(edge > 0.0 && edge < 1.0, "got {edge}");
    }

    #[test]
    fn color_range_selects_a_hue() {
        let mut r = RangeMask::default();
        r.color_enabled = true;
        r.hue = 120.0; // green
        r.hue_width = 25.0;
        r.hue_feather = 20.0;
        assert!(range_weight([0.1, 0.8, 0.1], &r) > 0.99); // green
        assert!(range_weight([0.8, 0.1, 0.1], &r) < 0.01); // red
                                                           // Gray has no usable hue, so it is excluded whatever the target.
        assert!(range_weight([0.5, 0.5, 0.5], &r) < 0.01);
    }

    #[test]
    fn hue_distance_wraps_around_the_circle() {
        assert!((hue_distance(350.0, 10.0) - 20.0).abs() < 1e-4);
        assert!((hue_distance(10.0, 350.0) - 20.0).abs() < 1e-4);
        assert!((hue_distance(0.0, 180.0) - 180.0).abs() < 1e-4);
    }

    #[test]
    fn auto_mask_dab_stops_at_a_color_edge() {
        // Left half dark, right half bright; the dab is painted on the left.
        let (w, h) = (16, 16);
        let mut buf = vec![0.0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let v = if x < w / 2 { 0.2 } else { 0.9 };
                let i = (y * w + x) * 3;
                buf[i] = v;
                buf[i + 1] = v;
                buf[i + 2] = v;
            }
        }
        let dabs = vec![Dab {
            p: [0.5, 0.5],
            radius: 0.5,
            hardness: 0.95,
            erase: false,
            auto: Some([0.2, 0.2, 0.2]),
        }];
        let cov = rasterize_brush(&dabs, w, h, [0.0, 0.0, 1.0, 1.0], &buf);
        let y = h / 2;
        // Well inside the dark half: covered. Inside the bright half, and
        // equally within the dab's radius: rejected by the reference color.
        assert!(cov[y * w + 4] > 0.9, "dark side {}", cov[y * w + 4]);
        assert!(cov[y * w + 11] < 0.05, "bright side {}", cov[y * w + 11]);
    }

    #[test]
    fn auto_mask_without_reference_covers_regardless_of_color() {
        let (w, h) = (16, 16);
        let mut buf = vec![0.0f32; w * h * 3];
        for x in 0..w * h {
            let v = if x % w < w / 2 { 0.2 } else { 0.9 };
            buf[x * 3] = v;
            buf[x * 3 + 1] = v;
            buf[x * 3 + 2] = v;
        }
        let dabs = vec![dab([0.5, 0.5], 0.5, 0.95, false)];
        let cov = rasterize_brush(&dabs, w, h, [0.0, 0.0, 1.0, 1.0], &buf);
        let y = h / 2;
        assert!(cov[y * w + 4] > 0.9);
        assert!(cov[y * w + 11] > 0.9);
    }
}
