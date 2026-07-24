//! Geometry: 90° orientation, horizontal/vertical flips, arbitrary-angle
//! straighten (bilinear, black outside the frame), and cropping. All
//! non-destructive — applied at render time from EditParams.

use rayon::prelude::*;

use crate::engine::params::EditParams;
use crate::engine::pipeline::SourceImage;

/// Apply orientation, straighten angle, and (optionally) crop.
/// Call only when `p.has_geometry(include_crop)` — otherwise use the source as-is.
pub fn apply(src: &SourceImage, p: &EditParams, include_crop: bool) -> SourceImage {
    let mut img = if p.rotate90 % 4 != 0 || p.flip_h || p.flip_v {
        orient(src, p.rotate90 % 4, p.flip_h, p.flip_v)
    } else {
        SourceImage {
            width: src.width,
            height: src.height,
            data: src.data.clone(),
        }
    };
    if p.angle != 0.0 {
        img = rotate_angle(&img, p.angle);
    }
    if include_crop && p.has_crop() {
        img = crop(&img, p.crop);
    }
    img
}

/// Dimensions after geometry, without materializing the image.
pub fn oriented_dims(w: usize, h: usize, p: &EditParams, include_crop: bool) -> (usize, usize) {
    let (mut w, mut h) = if p.rotate90 % 2 == 1 { (h, w) } else { (w, h) };
    if include_crop && p.has_crop() {
        w = ((w as f32 * p.crop[2]).round() as usize).max(1);
        h = ((h as f32 * p.crop[3]).round() as usize).max(1);
    }
    (w, h)
}

/// 90°-step rotation plus flips (flips act on the already-rotated frame).
fn orient(src: &SourceImage, k: u8, flip_h: bool, flip_v: bool) -> SourceImage {
    let (sw, sh) = (src.width, src.height);
    let (dw, dh) = if k % 2 == 1 { (sh, sw) } else { (sw, sh) };
    let mut out = vec![0.0f32; dw * dh * 3];
    out.par_chunks_mut(dw * 3).enumerate().for_each(|(y, row)| {
        for x in 0..dw {
            // Undo flips first (they're the last op applied), then rotation.
            let ux = if flip_h { dw - 1 - x } else { x };
            let uy = if flip_v { dh - 1 - y } else { y };
            let (sx, sy) = match k {
                1 => (uy, sh - 1 - ux),          // 90° clockwise
                2 => (sw - 1 - ux, sh - 1 - uy), // 180°
                3 => (sw - 1 - uy, ux),          // 90° counter-clockwise
                _ => (ux, uy),
            };
            let s = (sy * sw + sx) * 3;
            row[x * 3..x * 3 + 3].copy_from_slice(&src.data[s..s + 3]);
        }
    });
    SourceImage {
        width: dw,
        height: dh,
        data: out,
    }
}

/// Rotate by `deg` around the center (positive = clockwise on screen),
/// same canvas size, bilinear sampling, black outside the source frame.
fn rotate_angle(src: &SourceImage, deg: f32) -> SourceImage {
    let (w, h) = (src.width, src.height);
    let theta = deg.to_radians();
    let (sin, cos) = theta.sin_cos();
    let cx = (w as f32 - 1.0) * 0.5;
    let cy = (h as f32 - 1.0) * 0.5;
    let mut out = vec![0.0f32; w * h * 3];
    out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        let dy = y as f32 - cy;
        for x in 0..w {
            let dx = x as f32 - cx;
            let sx = cx + cos * dx - sin * dy;
            let sy = cy + sin * dx + cos * dy;
            if sx < -0.5 || sy < -0.5 || sx > w as f32 - 0.5 || sy > h as f32 - 0.5 {
                continue; // outside: stays black
            }
            let x0 = (sx.floor().max(0.0) as usize).min(w - 1);
            let y0 = (sy.floor().max(0.0) as usize).min(h - 1);
            let x1 = (x0 + 1).min(w - 1);
            let y1 = (y0 + 1).min(h - 1);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let fy = (sy - y0 as f32).clamp(0.0, 1.0);
            for c in 0..3 {
                let p00 = src.data[(y0 * w + x0) * 3 + c];
                let p10 = src.data[(y0 * w + x1) * 3 + c];
                let p01 = src.data[(y1 * w + x0) * 3 + c];
                let p11 = src.data[(y1 * w + x1) * 3 + c];
                let top = p00 + (p10 - p00) * fx;
                let bot = p01 + (p11 - p01) * fx;
                row[x * 3 + c] = top + (bot - top) * fy;
            }
        }
    });
    SourceImage {
        width: w,
        height: h,
        data: out,
    }
}

/// Extract the normalized crop rect (x, y, w, h in 0..1).
fn crop(src: &SourceImage, rect: [f32; 4]) -> SourceImage {
    let x0 = ((rect[0] * src.width as f32).round() as usize).min(src.width - 1);
    let y0 = ((rect[1] * src.height as f32).round() as usize).min(src.height - 1);
    let w = ((rect[2] * src.width as f32).round() as usize)
        .max(1)
        .min(src.width - x0);
    let h = ((rect[3] * src.height as f32).round() as usize)
        .max(1)
        .min(src.height - y0);
    let mut out = vec![0.0f32; w * h * 3];
    out.par_chunks_mut(w * 3).enumerate().for_each(|(y, row)| {
        let s = ((y0 + y) * src.width + x0) * 3;
        row.copy_from_slice(&src.data[s..s + w * 3]);
    });
    SourceImage {
        width: w,
        height: h,
        data: out,
    }
}

/// Transform a normalized crop rect when the image is rotated 90° clockwise,
/// so the crop follows the pixels it covered.
pub fn crop_rotated_cw(c: [f32; 4]) -> [f32; 4] {
    [1.0 - (c[1] + c[3]), c[0], c[3], c[2]]
}

/// Same for a 90° counter-clockwise rotation.
pub fn crop_rotated_ccw(c: [f32; 4]) -> [f32; 4] {
    [c[1], 1.0 - (c[0] + c[2]), c[3], c[2]]
}

/// Mirror a crop rect horizontally / vertically.
pub fn crop_flipped(c: [f32; 4], horizontal: bool) -> [f32; 4] {
    if horizontal {
        [1.0 - (c[0] + c[2]), c[1], c[2], c[3]]
    } else {
        [c[0], 1.0 - (c[1] + c[3]), c[2], c[3]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize) -> SourceImage {
        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&[x as f32, y as f32, 0.0]);
            }
        }
        SourceImage {
            width: w,
            height: h,
            data,
        }
    }

    fn px(s: &SourceImage, x: usize, y: usize) -> [f32; 3] {
        let i = (y * s.width + x) * 3;
        [s.data[i], s.data[i + 1], s.data[i + 2]]
    }

    #[test]
    fn rotate90_cw_moves_topleft_to_topright() {
        let src = img(4, 3);
        let mut p = EditParams::default();
        p.rotate90 = 1;
        let out = apply(&src, &p, true);
        assert_eq!((out.width, out.height), (3, 4));
        // src (0,0) should land at dest (h_src-1 - 0, 0) = (2, 0)
        assert_eq!(px(&out, 2, 0), [0.0, 0.0, 0.0]);
        // src (3,2) → dest (0, 3)
        assert_eq!(px(&out, 0, 3), [3.0, 2.0, 0.0]);
    }

    #[test]
    fn four_quarter_turns_is_identity() {
        let src = img(5, 4);
        let mut p = EditParams::default();
        p.rotate90 = 4;
        assert!(!p.has_geometry(true)); // 4 % 4 == 0
    }

    #[test]
    fn flip_h_mirrors() {
        let src = img(4, 2);
        let mut p = EditParams::default();
        p.flip_h = true;
        let out = apply(&src, &p, true);
        assert_eq!(px(&out, 0, 0), [3.0, 0.0, 0.0]);
        assert_eq!(px(&out, 3, 0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn crop_quarter() {
        let src = img(8, 8);
        let mut p = EditParams::default();
        p.crop = [0.5, 0.5, 0.5, 0.5];
        let out = apply(&src, &p, true);
        assert_eq!((out.width, out.height), (4, 4));
        assert_eq!(px(&out, 0, 0), [4.0, 4.0, 0.0]);
    }

    #[test]
    fn crop_ignored_when_not_included() {
        let src = img(8, 8);
        let mut p = EditParams::default();
        p.crop = [0.5, 0.5, 0.5, 0.5];
        p.flip_h = true; // keep has_geometry true
        let out = apply(&src, &p, false);
        assert_eq!((out.width, out.height), (8, 8));
    }

    #[test]
    fn zero_angle_rotation_identity_dims_and_center() {
        let src = img(9, 7);
        let mut p = EditParams::default();
        p.angle = 0.0001; // tiny nonzero to force the rotation path
        let out = apply(&src, &p, true);
        assert_eq!((out.width, out.height), (9, 7));
        // Center pixel unchanged by a near-zero rotation.
        let c = px(&out, 4, 3);
        assert!((c[0] - 4.0).abs() < 0.01 && (c[1] - 3.0).abs() < 0.01);
    }

    #[test]
    fn crop_rect_follows_rotation() {
        let c = [0.0, 0.0, 0.5, 0.25]; // top-left strip
        let cw = crop_rotated_cw(c);
        // After CW rotation the top-left goes to the top-right.
        assert_eq!(cw, [0.75, 0.0, 0.25, 0.5]);
        // CW then CCW round-trips.
        assert_eq!(crop_rotated_ccw(cw), c);
    }

    #[test]
    fn oriented_dims_swaps_and_crops() {
        let mut p = EditParams::default();
        p.rotate90 = 1;
        p.crop = [0.0, 0.0, 0.5, 0.5];
        assert_eq!(oriented_dims(400, 200, &p, false), (200, 400));
        assert_eq!(oriented_dims(400, 200, &p, true), (100, 200));
    }
}
