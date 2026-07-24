//! Mask coverage: turning a `MaskKind` into a per-pixel weight in 0..1.
//!
//! Linear and radial masks are analytic — evaluated per pixel from
//! normalized coordinates, so they cost nothing to store and scale to any
//! render resolution. Brush masks are rasterized from their dabs into a
//! coverage buffer sized to the current render (dabs are stored in
//! normalized image space, so the same strokes render at any resolution).

use rayon::prelude::*;

use crate::engine::params::{Mask, MaskKind};

/// Analytic mask weight at a normalized point (nx, ny in 0..1 of the full
/// image). Brush masks return 0 here — use `rasterize_brush` for those.
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
fn linear_weight(p0: [f32; 2], p1: [f32; 2], nx: f32, ny: f32) -> f32 {
    let dx = p1[0] - p0[0];
    let dy = p1[1] - p0[1];
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-9 {
        return 1.0;
    }
    // Projection of (point - p0) onto the p0->p1 axis, normalized to 0..1.
    let t = ((nx - p0[0]) * dx + (ny - p0[1]) * dy) / len2;
    let t = t.clamp(0.0, 1.0);
    // Smoothstep for a soft gradient.
    t * t * (3.0 - 2.0 * t)
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
        let t = 1.0 - (d - inner) / (1.0 - inner).max(1e-4);
        t * t * (3.0 - 2.0 * t)
    }
}

/// Rasterize a brush mask's dabs into a `w`×`h` coverage buffer. `norm_rect`
/// is the region of the full image the buffer covers (x, y, w, h in 0..1).
pub fn rasterize_brush(dabs: &[super::super::params::Dab], w: usize, h: usize, norm_rect: [f32; 4]) -> Vec<f32> {
    let mut buf = vec![0.0f32; w * h];
    if dabs.is_empty() {
        return buf;
    }
    let (rx0, ry0, rw, rh) = (norm_rect[0], norm_rect[1], norm_rect[2], norm_rect[3]);
    // Paint dabs in order; erase dabs subtract. Coverage accumulates toward 1.
    buf.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
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
                let a = if d <= hard {
                    1.0
                } else {
                    let t = 1.0 - (d - hard) / (1.0 - hard);
                    t * t * (3.0 - 2.0 * t)
                };
                if dab.erase {
                    cov *= 1.0 - a;
                } else {
                    cov = cov + a * (1.0 - cov); // over-composite
                }
            }
            row[x] = cov.clamp(0.0, 1.0);
        }
    });
    buf
}

/// Apply a mask's `inverted` flag to a weight.
#[inline]
pub fn apply_inversion(weight: f32, mask: &Mask) -> f32 {
    if mask.inverted {
        1.0 - weight
    } else {
        weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::params::Dab;

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
        let dabs = vec![Dab {
            p: [0.5, 0.5],
            radius: 0.25,
            hardness: 0.5,
            erase: false,
        }];
        let buf = rasterize_brush(&dabs, 20, 20, [0.0, 0.0, 1.0, 1.0]);
        // Center pixel painted, corner not.
        assert!(buf[10 * 20 + 10] > 0.9);
        assert!(buf[0] < 0.01);
    }

    #[test]
    fn brush_erase_removes_coverage() {
        let dabs = vec![
            Dab { p: [0.5, 0.5], radius: 0.4, hardness: 0.9, erase: false },
            Dab { p: [0.5, 0.5], radius: 0.4, hardness: 0.9, erase: true },
        ];
        let buf = rasterize_brush(&dabs, 16, 16, [0.0, 0.0, 1.0, 1.0]);
        assert!(buf[8 * 16 + 8] < 0.1);
    }

    #[test]
    fn inversion_flips_weight() {
        let mut m = Mask::default();
        assert_eq!(apply_inversion(0.3, &m), 0.3);
        m.inverted = true;
        assert!((apply_inversion(0.3, &m) - 0.7).abs() < 1e-6);
    }
}
