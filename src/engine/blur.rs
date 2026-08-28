//! Fast approximate Gaussian blur for single-channel f32 images:
//! three iterated box blurs, each done as a horizontal pass + transpose
//! so both directions are cache-friendly sliding-window sums.

use rayon::prelude::*;

/// Blur `src` (w*h, row-major) with an approximate Gaussian of the given radius.
pub fn gaussian_approx(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    let radius = radius.max(1);
    let mut a = src.to_vec();
    let mut b = vec![0.0; src.len()];
    for _ in 0..3 {
        box_blur_rows(&a, &mut b, w, h, radius);
        transpose(&b, &mut a, w, h);
        box_blur_rows(&a, &mut b, h, w, radius);
        transpose(&b, &mut a, h, w);
    }
    a
}

/// Sliding-window box blur along rows. Edges clamp.
fn box_blur_rows(src: &[f32], dst: &mut [f32], w: usize, h: usize, radius: usize) {
    let norm = 1.0 / (2 * radius + 1) as f32;
    dst.par_chunks_mut(w)
        .zip(src.par_chunks(w))
        .for_each(|(drow, srow)| {
            let clamp = |i: isize| srow[i.clamp(0, w as isize - 1) as usize];
            let mut acc = 0.0;
            for i in -(radius as isize)..=(radius as isize) {
                acc += clamp(i);
            }
            for x in 0..w {
                drow[x] = acc * norm;
                acc +=
                    clamp(x as isize + radius as isize + 1) - clamp(x as isize - radius as isize);
            }
        });
    let _ = h;
}

fn transpose(src: &[f32], dst: &mut [f32], w: usize, h: usize) {
    // Each output row (a former column) is built independently, in parallel.
    dst.par_chunks_mut(h).enumerate().for_each(|(x, out_row)| {
        for y in 0..h {
            out_row[y] = src[y * w + x];
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_preserves_constant_image() {
        let img = vec![0.5f32; 40 * 30];
        let out = gaussian_approx(&img, 40, 30, 5);
        for v in out {
            assert!((v - 0.5).abs() < 1e-4);
        }
    }

    #[test]
    fn blur_smooths_impulse() {
        let mut img = vec![0.0f32; 21 * 21];
        img[10 * 21 + 10] = 1.0;
        let out = gaussian_approx(&img, 21, 21, 2);
        // Energy spreads but total roughly conserved (edges clamp so approximate).
        let sum: f32 = out.iter().sum();
        assert!((sum - 1.0).abs() < 0.05, "sum was {sum}");
        assert!(out[10 * 21 + 10] < 0.5);
    }
}
