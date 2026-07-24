//! RGB histogram of a rendered image, plus shadow/highlight clipping
//! overlays. Both operate on the final display RGBA buffer so they show
//! exactly what the current edit produces.

use rayon::prelude::*;

pub const BINS: usize = 256;

#[derive(Clone, Default)]
pub struct Histogram {
    pub r: Vec<u32>,
    pub g: Vec<u32>,
    pub b: Vec<u32>,
}

impl Histogram {
    /// Max bin count ignoring the extreme endpoint bins (which spike on any
    /// clipped image and would flatten the rest of the plot).
    pub fn display_max(&self) -> u32 {
        let mid = |v: &[u32]| v[2..BINS - 2].iter().copied().max().unwrap_or(0);
        mid(&self.r).max(mid(&self.g)).max(mid(&self.b)).max(1)
    }
}

/// Count each channel of an RGBA buffer into 256 bins.
pub fn compute(rgba: &[u8]) -> Histogram {
    let zero = || (vec![0u32; BINS], vec![0u32; BINS], vec![0u32; BINS]);
    let (r, g, b) = rgba
        .par_chunks(4 * 16384)
        .fold(zero, |(mut r, mut g, mut b), chunk| {
            for px in chunk.chunks_exact(4) {
                r[px[0] as usize] += 1;
                g[px[1] as usize] += 1;
                b[px[2] as usize] += 1;
            }
            (r, g, b)
        })
        .reduce(zero, |(mut ar, mut ag, mut ab), (br, bg, bb)| {
            for i in 0..BINS {
                ar[i] += br[i];
                ag[i] += bg[i];
                ab[i] += bb[i];
            }
            (ar, ag, ab)
        });
    Histogram { r, g, b }
}

/// Paint clipping indicators into the display buffer, Lightroom-style:
/// blue where any channel hits 0 (crushed shadows), red where any channel
/// hits 255 (blown highlights). Highlights win where both apply.
pub fn mark_clipping(rgba: &mut [u8], shadows: bool, highlights: bool) {
    if !shadows && !highlights {
        return;
    }
    rgba.par_chunks_mut(4).for_each(|px| {
        let hi = px[0] == 255 || px[1] == 255 || px[2] == 255;
        let lo = px[0] == 0 || px[1] == 0 || px[2] == 0;
        if highlights && hi {
            px[0] = 235;
            px[1] = 60;
            px[2] = 60;
        } else if shadows && lo {
            px[0] = 70;
            px[1] = 120;
            px[2] = 235;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pixels_per_bin() {
        // Two pixels: one black, one mid-gray.
        let rgba = [0u8, 0, 0, 255, 128, 128, 128, 255];
        let h = compute(&rgba);
        assert_eq!(h.r[0], 1);
        assert_eq!(h.r[128], 1);
        assert_eq!(h.g[128], 1);
        assert_eq!(h.r.iter().sum::<u32>(), 2);
    }

    #[test]
    fn clipping_marks_only_requested_ends() {
        let mut rgba = [255u8, 255, 255, 255, 0, 0, 0, 255, 128, 128, 128, 255];
        mark_clipping(&mut rgba, true, false);
        // Highlight pixel untouched (highlights off), shadow pixel blue.
        assert_eq!(rgba[0], 255);
        assert_eq!(&rgba[4..7], &[70, 120, 235]);
        // Midtone untouched.
        assert_eq!(rgba[8], 128);

        let mut rgba2 = [255u8, 128, 128, 255];
        mark_clipping(&mut rgba2, false, true);
        assert_eq!(&rgba2[0..3], &[235, 60, 60]);
    }
}
