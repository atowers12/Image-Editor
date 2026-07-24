//! Tone curve evaluation. Control points are interpolated with a
//! monotone cubic spline (Fritsch–Carlson) so the curve never overshoots
//! or wiggles between points, then baked into 256-entry lookup tables for
//! fast per-pixel application.

use crate::engine::params::{CurvePoint, ToneCurve};

pub const LUT_SIZE: usize = 256;

/// Four lookup tables (master applied to all channels, then per-channel).
pub struct CurveLuts {
    pub master: [f32; LUT_SIZE],
    pub r: [f32; LUT_SIZE],
    pub g: [f32; LUT_SIZE],
    pub b: [f32; LUT_SIZE],
    identity: bool,
}

impl CurveLuts {
    pub fn build(curve: &ToneCurve) -> Self {
        CurveLuts {
            master: build_lut(&curve.master),
            r: build_lut(&curve.r),
            g: build_lut(&curve.g),
            b: build_lut(&curve.b),
            identity: curve.is_identity(),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.identity
    }

    /// Apply master then the given channel LUT to a 0..1 value.
    #[inline]
    pub fn apply(&self, channel: usize, v: f32) -> f32 {
        let v = sample(&self.master, v);
        let ch = match channel {
            0 => &self.r,
            1 => &self.g,
            _ => &self.b,
        };
        sample(ch, v)
    }
}

#[inline]
fn sample(lut: &[f32; LUT_SIZE], v: f32) -> f32 {
    let x = (v.clamp(0.0, 1.0)) * (LUT_SIZE - 1) as f32;
    let i = x.floor() as usize;
    if i >= LUT_SIZE - 1 {
        return lut[LUT_SIZE - 1];
    }
    let f = x - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * f
}

/// Build a 256-entry LUT by sampling the monotone spline through `points`.
pub fn build_lut(points: &[CurvePoint]) -> [f32; LUT_SIZE] {
    let mut lut = [0.0f32; LUT_SIZE];
    // Sanitize: sort by x, need at least the two endpoints.
    let mut pts: Vec<CurvePoint> = points.to_vec();
    pts.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
    if pts.len() < 2 {
        for (i, v) in lut.iter_mut().enumerate() {
            *v = i as f32 / (LUT_SIZE - 1) as f32;
        }
        return lut;
    }

    let n = pts.len();
    let xs: Vec<f32> = pts.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = pts.iter().map(|p| p[1]).collect();

    // Secant slopes between successive points.
    let mut delta = vec![0.0f32; n - 1];
    for i in 0..n - 1 {
        let dx = (xs[i + 1] - xs[i]).max(1e-6);
        delta[i] = (ys[i + 1] - ys[i]) / dx;
    }

    // Tangents via Fritsch–Carlson to keep the spline monotone.
    let mut m = vec![0.0f32; n];
    m[0] = delta[0];
    m[n - 1] = delta[n - 2];
    for i in 1..n - 1 {
        if delta[i - 1] * delta[i] <= 0.0 {
            m[i] = 0.0; // local extremum: flatten to avoid overshoot
        } else {
            m[i] = (delta[i - 1] + delta[i]) / 2.0;
        }
    }
    for i in 0..n - 1 {
        if delta[i].abs() < 1e-9 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let a = m[i] / delta[i];
            let b = m[i + 1] / delta[i];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * delta[i];
                m[i + 1] = t * b * delta[i];
            }
        }
    }

    for (idx, out) in lut.iter_mut().enumerate() {
        let x = idx as f32 / (LUT_SIZE - 1) as f32;
        *out = eval(&xs, &ys, &m, x).clamp(0.0, 1.0);
    }
    lut
}

/// Evaluate the Hermite spline at x, clamping outside the point range.
fn eval(xs: &[f32], ys: &[f32], m: &[f32], x: f32) -> f32 {
    let n = xs.len();
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[n - 1] {
        return ys[n - 1];
    }
    // Find the segment (small n; linear scan is fine).
    let mut i = 0;
    while i < n - 1 && x > xs[i + 1] {
        i += 1;
    }
    let h = (xs[i + 1] - xs[i]).max(1e-6);
    let t = (x - xs[i]) / h;
    let t2 = t * t;
    let t3 = t2 * t;
    let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h10 = t3 - 2.0 * t2 + t;
    let h01 = -2.0 * t3 + 3.0 * t2;
    let h11 = t3 - t2;
    h00 * ys[i] + h10 * h * m[i] + h01 * ys[i + 1] + h11 * h * m[i + 1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_curve_is_flat() {
        let lut = build_lut(&[[0.0, 0.0], [1.0, 1.0]]);
        for i in 0..LUT_SIZE {
            let x = i as f32 / (LUT_SIZE - 1) as f32;
            assert!((lut[i] - x).abs() < 1e-3, "at {i}: {} vs {x}", lut[i]);
        }
    }

    #[test]
    fn passes_through_control_points() {
        let pts = [[0.0, 0.0], [0.5, 0.8], [1.0, 1.0]];
        let lut = build_lut(&pts);
        assert!((sample(&lut, 0.5) - 0.8).abs() < 5e-3);
        assert!((sample(&lut, 0.0) - 0.0).abs() < 1e-3);
        assert!((sample(&lut, 1.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn monotone_curve_stays_monotone() {
        // A steep S-curve must never decrease.
        let pts = [[0.0, 0.0], [0.25, 0.1], [0.75, 0.9], [1.0, 1.0]];
        let lut = build_lut(&pts);
        for i in 1..LUT_SIZE {
            assert!(lut[i] >= lut[i - 1] - 1e-4, "non-monotone at {i}");
        }
    }

    #[test]
    fn master_then_channel_compose() {
        let mut curve = ToneCurve::default();
        curve.master = vec![[0.0, 0.0], [1.0, 1.0]];
        curve.r = vec![[0.0, 0.2], [1.0, 1.0]]; // lift red blacks
        let luts = CurveLuts::build(&curve);
        assert!(luts.apply(0, 0.0) > 0.15); // red channel lifted
        assert!(luts.apply(1, 0.0) < 0.05); // green untouched
    }

    #[test]
    fn identity_flag_set() {
        assert!(CurveLuts::build(&ToneCurve::default()).is_identity());
        let mut c = ToneCurve::default();
        c.master.push([0.5, 0.7]);
        assert!(!CurveLuts::build(&c).is_identity());
    }
}
