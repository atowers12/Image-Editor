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

/// Dimensions after the 90° orientation step alone — the frame the straighten
/// angle rotates, and the frame the crop constraint tests against.
pub fn source_dims(w: usize, h: usize, p: &EditParams) -> (usize, usize) {
    if p.rotate90 % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// Dimensions after geometry, without materializing the image.
pub fn oriented_dims(w: usize, h: usize, p: &EditParams, include_crop: bool) -> (usize, usize) {
    let (w, h) = source_dims(w, h, p);
    // Straightening grows the canvas to the rotated bounding box.
    let (mut w, mut h) = straightened_dims(w, h, p.angle);
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

/// Canvas size after straightening a `w`×`h` frame by `deg`: the rotated
/// frame's bounding box.
///
/// Rotating inside the *source* size would push the rotated rectangle's four
/// corners outside the canvas and clip them away — straightening would throw
/// away real pixels along every edge on top of adding black corners. Growing
/// to the bounding box keeps every pixel; only the corner wedges are black.
pub fn straightened_dims(w: usize, h: usize, deg: f32) -> (usize, usize) {
    if deg == 0.0 {
        return (w, h);
    }
    let (sin, cos) = deg.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    let (wf, hf) = (w as f32, h as f32);
    // Round rather than ceil: an imperceptible angle should not grow the
    // canvas by a pixel, and half a pixel at a corner point is nothing.
    (
        ((wf * cos + hf * sin).round() as usize).max(1),
        ((wf * sin + hf * cos).round() as usize).max(1),
    )
}

/// Rotate by `deg` around the center (positive = clockwise on screen) into a
/// canvas grown to the rotated bounding box, bilinear sampling, black in the
/// corner wedges the source no longer reaches.
fn rotate_angle(src: &SourceImage, deg: f32) -> SourceImage {
    let (w, h) = (src.width, src.height);
    let (dw, dh) = straightened_dims(w, h, deg);
    let theta = deg.to_radians();
    let (sin, cos) = theta.sin_cos();
    // Map the destination's center onto the source's center.
    let (dcx, dcy) = ((dw as f32 - 1.0) * 0.5, (dh as f32 - 1.0) * 0.5);
    let (scx, scy) = ((w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5);
    let mut out = vec![0.0f32; dw * dh * 3];
    out.par_chunks_mut(dw * 3).enumerate().for_each(|(y, row)| {
        let dy = y as f32 - dcy;
        for x in 0..dw {
            let dx = x as f32 - dcx;
            let sx = scx + cos * dx - sin * dy;
            let sy = scy + sin * dx + cos * dy;
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
        width: dw,
        height: dh,
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

/// Is a normalized canvas point still covered by the straightened image?
///
/// `src` is the frame *before* the angle is applied (see `source_dims`); the
/// canvas the point is normalized against is that frame's rotated bounding
/// box. Mirrors `rotate_angle`'s inverse map — rotate the point back by `deg`
/// and ask whether it lands inside the source frame — in continuous pixel
/// coordinates, where pixel index `i` spans `i..i+1`.
pub fn point_in_frame(n: [f32; 2], deg: f32, src: (usize, usize)) -> bool {
    const EPS: f32 = 1e-4;
    // The canvas itself is always the outer bound.
    if n[0] < -EPS || n[0] > 1.0 + EPS || n[1] < -EPS || n[1] > 1.0 + EPS {
        return false;
    }
    if deg == 0.0 {
        return true;
    }
    let (w, h) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (dw, dh) = straightened_dims(src.0.max(1), src.1.max(1), deg);
    let (dw, dh) = (dw as f32, dh as f32);
    let (sin, cos) = deg.to_radians().sin_cos();
    let dx = n[0] * dw - dw * 0.5;
    let dy = n[1] * dh - dh * 0.5;
    let sx = w * 0.5 + cos * dx - sin * dy;
    let sy = h * 0.5 + sin * dx + cos * dy;
    let tol = EPS * w.max(h);
    sx >= -tol && sx <= w + tol && sy >= -tol && sy <= h + tol
}

/// Is a normalized crop rect entirely inside the straightened image?
///
/// The covered region is the source rectangle rotated about the canvas
/// centre — convex — so testing the four corners is exact, not a sample.
pub fn rect_in_frame(c: [f32; 4], deg: f32, src: (usize, usize)) -> bool {
    let (x0, y0) = (c[0], c[1]);
    let (x1, y1) = (c[0] + c[2], c[1] + c[3]);
    [[x0, y0], [x1, y0], [x0, y1], [x1, y1]]
        .into_iter()
        .all(|p| point_in_frame(p, deg, src))
}

/// The straightened image's four corners, as normalized canvas points, in
/// order. This is the boundary the crop constraint holds the frame inside —
/// the overlay draws it so the limit is visible rather than mysterious.
pub fn frame_corners(deg: f32, src: (usize, usize)) -> [[f32; 2]; 4] {
    let (w, h) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (dw, dh) = straightened_dims(src.0.max(1), src.1.max(1), deg);
    let (dw, dh) = (dw as f32, dh as f32);
    let (sin, cos) = deg.to_radians().sin_cos();
    // Forward map: a source corner, offset from its centre, rotated onto the
    // canvas. The inverse of the map `point_in_frame` uses.
    let put = |sx: f32, sy: f32| {
        let (ox, oy) = (sx - w * 0.5, sy - h * 0.5);
        let x = dw * 0.5 + cos * ox + sin * oy;
        let y = dh * 0.5 - sin * ox + cos * oy;
        [x / dw, y / dh]
    };
    [put(0.0, 0.0), put(w, 0.0), put(w, h), put(0.0, h)]
}

/// Pull a crop rect back inside the straightened image, keeping its
/// width:height ratio and staying as large as it can. Returns whether it
/// had to move.
pub fn fit_crop_in_frame(c: &mut [f32; 4], deg: f32, src: (usize, usize)) -> bool {
    if rect_in_frame(*c, deg, src) {
        return false;
    }
    // The far end of the search: the same rect shrunk almost to a point at
    // the canvas centre, which is covered at any angle. Both ends share a
    // width:height ratio and we interpolate linearly, so every candidate
    // along the way keeps that ratio — an aspect lock survives the fit.
    const SEED: f32 = 0.001;
    let target = [
        0.5 - c[2] * SEED * 0.5,
        0.5 - c[3] * SEED * 0.5,
        c[2] * SEED,
        c[3] * SEED,
    ];
    let start = *c;
    let lerp = |t: f32| {
        let mut r = [0.0f32; 4];
        for i in 0..4 {
            r[i] = start[i] + (target[i] - start[i]) * t;
        }
        r
    };
    // Smallest amount of shrinking that fits: `lo` never fits, `hi` always does.
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if rect_in_frame(lerp(mid), deg, src) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    *c = lerp(hi);
    true
}

/// Grow a crop rect to the largest one of the same width:height ratio that
/// fits the straightened frame, centred on it. The counterpart to
/// `fit_crop_in_frame`, which only ever shrinks — after straightening, "give
/// me back as much photo as this shape can hold" is the useful move.
pub fn fill_frame(c: &mut [f32; 4], deg: f32, src: (usize, usize)) {
    let ratio = if c[3] > 1e-6 { c[2] / c[3] } else { 1.0 };
    // Largest height (and so width, at a fixed ratio) that still fits.
    let candidate = |height: f32| {
        let (w, h) = (height * ratio, height);
        [0.5 - w * 0.5, 0.5 - h * 0.5, w, h]
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32); // `lo` fits, `hi` may not.
    if rect_in_frame(candidate(hi), deg, src) {
        *c = candidate(hi);
        return;
    }
    for _ in 0..24 {
        let mid = (lo + hi) * 0.5;
        if rect_in_frame(candidate(mid), deg, src) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    *c = candidate(lo);
}

/// Re-normalize a crop rect written against the un-grown straighten canvas.
///
/// Straightening used to rotate inside the source size, so a saved crop was a
/// fraction of `src`; it is now a fraction of `src`'s rotated bounding box,
/// which is larger and shares its centre. Same pixels, new denominator.
pub fn migrate_crop_to_grown_canvas(c: [f32; 4], deg: f32, src: (usize, usize)) -> [f32; 4] {
    if deg == 0.0 {
        return c;
    }
    let (w, h) = (src.0.max(1) as f32, src.1.max(1) as f32);
    let (dw, dh) = straightened_dims(src.0.max(1), src.1.max(1), deg);
    let (dw, dh) = (dw as f32, dh as f32);
    [
        ((dw - w) * 0.5 + c[0] * w) / dw,
        ((dh - h) * 0.5 + c[1] * h) / dh,
        c[2] * w / dw,
        c[3] * h / dh,
    ]
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
    fn straighten_grows_the_canvas_to_keep_every_pixel() {
        // A 45° turn of a square needs a canvas sqrt(2) wider.
        let (w, h) = straightened_dims(100, 100, 45.0);
        assert!((w as i32 - 142).abs() <= 1 && (h as i32 - 142).abs() <= 1);
        // Landscape at 90° worth of turn swaps, via the bounding box.
        assert_eq!(straightened_dims(400, 200, 0.0), (400, 200));
        let (w, h) = straightened_dims(400, 200, 30.0);
        assert!(w > 400 && h > 200);
        // And the rendered image really is that size — no clipped corners.
        let src = img(40, 30);
        let p = EditParams {
            angle: 30.0,
            ..EditParams::default()
        };
        let out = apply(&src, &p, false);
        assert_eq!((out.width, out.height), straightened_dims(40, 30, 30.0));
        assert_eq!(oriented_dims(40, 30, &p, false), (out.width, out.height));
    }

    #[test]
    fn straighten_keeps_every_source_pixel() {
        // Every source pixel is non-black, so anything black in the output is
        // wedge — and the non-black count tells us what survived the rotation.
        let (w, h) = (61usize, 43usize);
        let src = SourceImage {
            width: w,
            height: h,
            data: vec![0.5; w * h * 3],
        };
        let p = EditParams {
            angle: 24.0,
            ..EditParams::default()
        };
        let out = apply(&src, &p, false);
        let kept = out.data.chunks(3).filter(|px| px[0] > 0.01).count();
        // Rotating inside the source size used to clip the corners away; now
        // the canvas grows, so essentially the whole photo comes through.
        // (Resampling nibbles a fraction of a pixel around the border.)
        let area = w * h;
        assert!(
            kept as f32 > area as f32 * 0.98,
            "kept {kept} of {area} source pixels"
        );
        // And the canvas really did grow to make room.
        assert!(out.width > w && out.height > h);
    }

    #[test]
    fn full_frame_fits_only_when_straight() {
        let src = (4000, 3000);
        assert!(rect_in_frame([0.0, 0.0, 1.0, 1.0], 0.0, src));
        // The grown canvas has black wedges at its corners.
        assert!(!rect_in_frame([0.0, 0.0, 1.0, 1.0], 20.0, src));
        assert!(!rect_in_frame([0.0, 0.0, 1.0, 1.0], -20.0, src));
        // The centre is covered whatever the angle.
        assert!(point_in_frame([0.5, 0.5], 45.0, src));
        // ...and nothing outside the canvas ever is.
        assert!(!point_in_frame([1.2, 0.5], 0.0, src));
    }

    #[test]
    fn the_frame_corners_sit_on_the_boundary() {
        let src = (4000, 3000);
        for deg in [-28.0, -5.0, 12.0, 41.0] {
            let corners = frame_corners(deg, src);
            for c in corners {
                // On the boundary: inside (within tolerance)...
                assert!(point_in_frame(c, deg, src), "corner {c:?} at {deg}");
                // ...and nudged outward along both axes, outside.
                let out = [(c[0] - 0.5) * 1.02 + 0.5, (c[1] - 0.5) * 1.02 + 0.5];
                assert!(!point_in_frame(out, deg, src), "nudged {out:?} at {deg}");
            }
            // Each corner touches a canvas edge — the bounding box is tight.
            let touches = corners
                .iter()
                .filter(|c| c[0] < 1e-3 || c[0] > 1.0 - 1e-3 || c[1] < 1e-3 || c[1] > 1.0 - 1e-3)
                .count();
            assert_eq!(touches, 4, "at {deg}");
        }
    }

    #[test]
    fn fill_frame_maximizes_within_the_frame() {
        let src = (4000, 3000);
        let mut c = [0.45, 0.45, 0.1, 0.1];
        fill_frame(&mut c, 20.0, src);
        assert!(rect_in_frame(c, 20.0, src));
        assert!((c[2] / c[3] - 1.0).abs() < 1e-3); // square stayed square
        assert!(c[2] > 0.4); // and actually grew
                             // A touch bigger must not fit — it really is maximal.
        let bigger = [
            0.5 - c[2] * 0.52,
            0.5 - c[3] * 0.52,
            c[2] * 1.04,
            c[3] * 1.04,
        ];
        assert!(!rect_in_frame(bigger, 20.0, src));
        // Straight on, it fills the whole canvas.
        let mut c = [0.4, 0.4, 0.2, 0.15];
        fill_frame(&mut c, 0.0, src);
        assert!((c[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn migrating_a_crop_keeps_the_same_pixels() {
        let src = (4000, 3000);
        let deg = 15.0;
        // The old full-canvas crop covered exactly the un-grown frame, which
        // is centred in the new canvas.
        let m = migrate_crop_to_grown_canvas([0.0, 0.0, 1.0, 1.0], deg, src);
        let (dw, dh) = straightened_dims(src.0, src.1, deg);
        assert!((m[2] - src.0 as f32 / dw as f32).abs() < 1e-4);
        assert!((m[3] - src.1 as f32 / dh as f32).abs() < 1e-4);
        // Centred: equal margins either side.
        assert!((m[0] - (1.0 - m[2]) * 0.5).abs() < 1e-4);
        assert!((m[1] - (1.0 - m[3]) * 0.5).abs() < 1e-4);
        // A centred crop stays centred, and shrinks by the same factor.
        let m = migrate_crop_to_grown_canvas([0.25, 0.25, 0.5, 0.5], deg, src);
        assert!((m[0] + m[2] * 0.5 - 0.5).abs() < 1e-4);
        assert!((m[1] + m[3] * 0.5 - 0.5).abs() < 1e-4);
        // No angle, no change.
        assert_eq!(
            migrate_crop_to_grown_canvas([0.1, 0.2, 0.3, 0.4], 0.0, src),
            [0.1, 0.2, 0.3, 0.4]
        );
    }

    #[test]
    fn fit_shrinks_into_frame_and_keeps_ratio() {
        let src = (4000, 3000);
        let mut c = [0.0, 0.0, 1.0, 1.0];
        let before = c[2] / c[3];
        assert!(fit_crop_in_frame(&mut c, 30.0, src));
        assert!(rect_in_frame(c, 30.0, src));
        assert!((c[2] / c[3] - before).abs() < 1e-3);
        // It should land on the boundary, not shrink to nothing.
        assert!(c[2] > 0.2 && c[3] > 0.2);
    }

    #[test]
    fn fit_leaves_a_valid_rect_alone() {
        let src = (4000, 3000);
        let mut c = [0.4, 0.4, 0.2, 0.2];
        assert!(!fit_crop_in_frame(&mut c, 30.0, src));
        assert_eq!(c, [0.4, 0.4, 0.2, 0.2]);
    }

    #[test]
    fn fitted_crop_holds_a_locked_aspect() {
        let src = (4000, 3000);
        // 9:16 in pixels is 9/16 * 3000/4000 in normalized terms.
        let want = (9.0 / 16.0) * (3000.0 / 4000.0);
        let mut c = [0.0, 0.0, 0.4, 0.4 / want];
        fit_crop_in_frame(&mut c, -28.0, src);
        assert!(rect_in_frame(c, -28.0, src));
        assert!((c[2] / c[3] - want).abs() < 1e-3);
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
