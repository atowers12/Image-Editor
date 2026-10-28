//! Splitting a full-resolution render into tiles small enough for the GPU's
//! working set, without changing a single output pixel.
//!
//! Every spatial stage (blurs, bilateral NR) clamps at the edge of the buffer
//! it runs on. A tile is therefore rendered grown by a *halo* — the furthest
//! any finished pixel reaches back into the source (`StageRadii::halo`) — and
//! only its middle is kept: the clamping contaminates the halo but never
//! reaches the kept pixels. Where a tile meets the image's own edge it isn't
//! grown, so it clamps exactly where the full render does.

use crate::engine::pipeline::RenderCtx;

/// One tile of a tiled render. Rects are (x, y, w, h) in image pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    /// Pixels rendered: `inner` grown by the halo, clipped to the image.
    pub outer: [usize; 4],
    /// Pixels kept. The inner rects of a plan partition the image.
    pub inner: [usize; 4],
}

impl Tile {
    /// Render context placing this tile's outer rect inside a `w`×`h` image,
    /// with radii scaled to the whole image so blurs match the full render.
    pub fn ctx(&self, w: usize, h: usize) -> RenderCtx {
        let [x, y, tw, th] = self.outer;
        let (fw, fh) = (w as f32, h as f32);
        RenderCtx {
            norm_rect: [x as f32 / fw, y as f32 / fh, tw as f32 / fw, th as f32 / fh],
            radius_dim: w.max(h) as f32,
        }
    }

    /// Where the inner rect sits inside the outer buffer: (x, y, w, h).
    pub fn inner_in_outer(&self) -> [usize; 4] {
        [
            self.inner[0] - self.outer[0],
            self.inner[1] - self.outer[1],
            self.inner[2],
            self.inner[3],
        ]
    }
}

/// Tile a `w`×`h` image so no tile's outer edge exceeds `max_outer_edge`.
///
/// Returns `None` when the halo is too wide for that edge to be worth it —
/// tiles must keep at least half their edge, so a tiled render never costs
/// more than 4× the pixels it produces. An axis that already fits is never
/// split, so an image within `max_outer_edge` is one tile with no halo.
pub fn plan_tiles(w: usize, h: usize, halo: usize, max_outer_edge: usize) -> Option<Vec<Tile>> {
    let xs = axis_spans(w, halo, max_outer_edge)?;
    let ys = axis_spans(h, halo, max_outer_edge)?;
    let mut tiles = Vec::with_capacity(xs.len() * ys.len());
    for &(iy, ih, oy, oh) in &ys {
        for &(ix, iw, ox, ow) in &xs {
            tiles.push(Tile {
                outer: [ox, oy, ow, oh],
                inner: [ix, iy, iw, ih],
            });
        }
    }
    Some(tiles)
}

/// Split one axis of length `len` into (inner start, inner len, outer start,
/// outer len) spans.
fn axis_spans(len: usize, halo: usize, max_outer: usize) -> Option<Vec<(usize, usize, usize, usize)>> {
    if len <= max_outer {
        return Some(vec![(0, len, 0, len)]);
    }
    let max_inner = max_outer.checked_sub(2 * halo)?;
    if max_inner == 0 || max_inner * 2 < max_outer {
        return None;
    }
    let n = len.div_ceil(max_inner);
    Some(
        (0..n)
            .map(|i| {
                let a = len * i / n;
                let b = len * (i + 1) / n;
                let oa = a.saturating_sub(halo);
                let ob = (b + halo).min(len);
                (a, b - a, oa, ob - oa)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_plan(w: usize, h: usize, halo: usize, edge: usize) -> Vec<Tile> {
        let tiles = plan_tiles(w, h, halo, edge).expect("plan should fit");
        let mut hits = vec![0u8; w * h];
        for t in &tiles {
            let [ix, iy, iw, ih] = t.inner;
            let [ox, oy, ow, oh] = t.outer;
            assert!(ow <= edge && oh <= edge, "outer {:?} exceeds {edge}", t.outer);
            // The outer rect contains the inner one...
            assert!(ox <= ix && oy <= iy && ix + iw <= ox + ow && iy + ih <= oy + oh);
            // ...with the full halo on every side that isn't the image edge.
            assert!(ix - ox == halo.min(ix), "left halo {:?}", t);
            assert!(iy - oy == halo.min(iy), "top halo {:?}", t);
            assert!(ox + ow == (ix + iw + halo).min(w), "right halo {:?}", t);
            assert!(oy + oh == (iy + ih + halo).min(h), "bottom halo {:?}", t);
            for y in iy..iy + ih {
                for x in ix..ix + iw {
                    hits[y * w + x] += 1;
                }
            }
        }
        assert!(hits.iter().all(|&n| n == 1), "inner rects must partition the image");
        tiles
    }

    #[test]
    fn small_image_is_one_untouched_tile() {
        let tiles = check_plan(300, 200, 50, 512);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].outer, [0, 0, 300, 200]);
        assert_eq!(tiles[0].inner, [0, 0, 300, 200]);
    }

    #[test]
    fn large_image_tiles_partition_with_halos() {
        let tiles = check_plan(1000, 700, 40, 256);
        assert!(tiles.len() > 4);
        // Only the long axis that overflows is split in a wide, short image.
        let wide = check_plan(1000, 200, 40, 256);
        assert!(wide.iter().all(|t| t.outer[1] == 0 && t.outer[3] == 200));
    }

    #[test]
    fn halo_too_wide_for_the_edge_is_refused() {
        // Keeping under half the edge isn't worth tiling for.
        assert!(plan_tiles(1000, 1000, 70, 256).is_none());
        assert!(plan_tiles(1000, 1000, 200, 256).is_none());
        assert!(plan_tiles(1000, 1000, 64, 256).is_some());
    }

    #[test]
    fn tile_ctx_maps_back_to_the_full_image() {
        let tiles = check_plan(400, 300, 10, 128);
        let t = tiles[5];
        let ctx = t.ctx(400, 300);
        assert_eq!(ctx.radius_dim, 400.0);
        assert!((ctx.norm_rect[0] * 400.0 - t.outer[0] as f32).abs() < 1e-3);
        assert!((ctx.norm_rect[3] * 300.0 - t.outer[3] as f32).abs() < 1e-3);
        let [x, y, w, h] = t.inner_in_outer();
        assert_eq!((w, h), (t.inner[2], t.inner[3]));
        assert_eq!((x + t.outer[0], y + t.outer[1]), (t.inner[0], t.inner[1]));
    }
}
