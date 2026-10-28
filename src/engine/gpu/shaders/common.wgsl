// Shared helpers, prepended to every kernel. Each is a port of a CPU op in
// src/engine/ops — keep them in step with their Rust originals; the GPU
// parity tests render both paths and compare.

const SQRT_2: f32 = 1.41421356;

// Rust's f32::rem_euclid.
fn rem_euclid(a: f32, b: f32) -> f32 {
    let r = a % b;
    if (r < 0.0) {
        return r + abs(b);
    }
    return r;
}

// tone::luma
fn luma(p: vec3<f32>) -> f32 {
    return 0.2126 * p.x + 0.7152 * p.y + 0.0722 * p.z;
}

// pipeline::srgb_encode
fn srgb_encode(x: f32) -> f32 {
    if (x <= 0.0031308) {
        return 12.92 * x;
    }
    return 1.055 * pow(max(x, 0.0), 1.0 / 2.4) - 0.055;
}

fn smoothstep01(t: f32) -> f32 {
    let c = clamp(t, 0.0, 1.0);
    return c * c * (3.0 - 2.0 * c);
}

// tone::range_mask
fn range_mask(l: f32, center: f32, width: f32) -> f32 {
    let d = (l - center) / width;
    return exp(-0.5 * d * d);
}

// tone::range_delta. `s` = (highlights, shadows, whites, blacks).
fn range_delta(l: f32, s: vec4<f32>, strength: f32) -> f32 {
    let end_strength = strength * (5.0 / 6.0);
    var d = 0.0;
    if (s.x != 0.0) {
        d += s.x * strength * range_mask(l, 0.75, 0.22);
    }
    if (s.y != 0.0) {
        d += s.y * strength * range_mask(l, 0.25, 0.22);
    }
    if (s.z != 0.0) {
        d += s.z * end_strength * range_mask(l, 1.0, 0.18);
    }
    if (s.w != 0.0) {
        d += s.w * end_strength * range_mask(l, 0.0, 0.18);
    }
    return d;
}

// tone::contrast
fn contrast(x0: f32, c: f32) -> f32 {
    let x = clamp(x0, 0.0, 1.0);
    if (c > 0.0) {
        let s = x * x * (3.0 - 2.0 * x);
        return x + (s - x) * c;
    }
    let flat = 0.5 + (x - 0.5) * 0.72;
    return x + (flat - x) * (-c);
}

fn contrast3(p: vec3<f32>, c: f32) -> vec3<f32> {
    return vec3<f32>(contrast(p.x, c), contrast(p.y, c), contrast(p.z, c));
}

// Scale RGB so luma moves from `l` to `nl`, the way every luma-only stage
// does it on the CPU.
fn luma_ratio(p: vec3<f32>, l: f32, nl: f32) -> vec3<f32> {
    return p * (max(nl, 0.0) / max(l, 1e-5));
}

// color::rgb_to_hsl → (hue degrees, saturation, lightness)
fn rgb_to_hsl(p: vec3<f32>) -> vec3<f32> {
    let mx = max(max(p.x, p.y), p.z);
    let mn = min(min(p.x, p.y), p.z);
    let l = 0.5 * (mx + mn);
    let d = mx - mn;
    if (d < 1e-6) {
        return vec3<f32>(0.0, 0.0, l);
    }
    let s = d / max(1.0 - abs(2.0 * l - 1.0), 1e-6);
    var h: f32;
    if (mx == p.x) {
        h = 60.0 * rem_euclid((p.y - p.z) / d, 6.0);
    } else if (mx == p.y) {
        h = 60.0 * ((p.z - p.x) / d + 2.0);
    } else {
        h = 60.0 * ((p.x - p.y) / d + 4.0);
    }
    return vec3<f32>(h, min(s, 1.0), l);
}

// color::hsl_to_rgb
fn hsl_to_rgb(h0: f32, s: f32, l: f32) -> vec3<f32> {
    let h = rem_euclid(h0, 360.0);
    let c = (1.0 - abs(2.0 * l - 1.0)) * s;
    let x = c * (1.0 - abs(rem_euclid(h / 60.0, 2.0) - 1.0));
    let m = l - c / 2.0;
    var rgb: vec3<f32>;
    switch (u32(h / 60.0)) {
        case 0u: { rgb = vec3<f32>(c, x, 0.0); }
        case 1u: { rgb = vec3<f32>(x, c, 0.0); }
        case 2u: { rgb = vec3<f32>(0.0, c, x); }
        case 3u: { rgb = vec3<f32>(0.0, x, c); }
        case 4u: { rgb = vec3<f32>(x, 0.0, c); }
        default: { rgb = vec3<f32>(c, 0.0, x); }
    }
    return rgb + vec3<f32>(m);
}

// color::saturate
fn saturate_px(p: vec3<f32>, l: f32, amount: f32) -> vec3<f32> {
    let k = 1.0 + amount;
    return vec3<f32>(l) + (p - vec3<f32>(l)) * k;
}

// color::vibrance
fn vibrance(p: vec3<f32>, l: f32, amount: f32) -> vec3<f32> {
    let mx = max(max(p.x, p.y), p.z);
    let mn = min(min(p.x, p.y), p.z);
    var sat = 0.0;
    if (mx > 1e-5) {
        sat = (mx - mn) / mx;
    }
    var k = amount * (1.0 - sat);
    if (amount > 0.0) {
        // Skin protection: damp the boost for orange-ish hues.
        let hsl = rgb_to_hsl(p);
        if (hsl.y > 0.05 && hsl.x > 10.0 && hsl.x < 50.0) {
            k *= 0.35;
        }
    }
    return saturate_px(p, l, k);
}

// detail::dehaze_channel
fn dehaze_channel(x: f32, amount: f32, strength: f32) -> f32 {
    if (amount > 0.0) {
        let k = strength * amount;
        return max((x - k) / max(1.0 - k, 0.05), 0.0);
    }
    let veil = 0.30 + x * 0.55;
    return x + (veil - x) * (-amount);
}

fn dehaze3(p: vec3<f32>, amount: f32, strength: f32) -> vec3<f32> {
    return vec3<f32>(
        dehaze_channel(p.x, amount, strength),
        dehaze_channel(p.y, amount, strength),
        dehaze_channel(p.z, amount, strength),
    );
}

// --- Masks (ops/mask.rs) ---

fn linear_weight(p0: vec2<f32>, p1: vec2<f32>, n: vec2<f32>) -> f32 {
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let len2 = dx * dx + dy * dy;
    if (len2 < 1e-9) {
        return 1.0;
    }
    let t = ((n.x - p0.x) * dx + (n.y - p0.y) * dy) / len2;
    return smoothstep01(t);
}

fn radial_weight(center: vec2<f32>, radius: vec2<f32>, feather: f32, n: vec2<f32>) -> f32 {
    let rx = max(radius.x, 1e-4);
    let ry = max(radius.y, 1e-4);
    let dx = (n.x - center.x) / rx;
    let dy = (n.y - center.y) / ry;
    let d = sqrt(dx * dx + dy * dy);
    let inner = 1.0 - clamp(feather, 0.0, 1.0);
    if (d <= inner) {
        return 1.0;
    }
    if (d >= 1.0) {
        return 0.0;
    }
    return smoothstep01(1.0 - (d - inner) / max(1.0 - inner, 1e-4));
}

// MaskOp::combine — op 0 add, 1 subtract, 2 intersect.
fn combine(op: u32, w: f32, c: f32) -> f32 {
    switch (op) {
        case 1u: { return w * (1.0 - c); }
        case 2u: { return w * c; }
        default: { return w + c - w * c; }
    }
}

fn invert_if(w: f32, inverted: bool) -> f32 {
    return select(w, 1.0 - w, inverted);
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = rem_euclid(a - b, 360.0);
    return min(d, 360.0 - d);
}

fn band(x: f32, lo: f32, hi: f32, feather: f32) -> f32 {
    let f = max(feather, 1e-4);
    let up = smoothstep01((x - lo + f) / f);
    let down = smoothstep01((hi + f - x) / f);
    return min(up, down);
}

const SAT_GATE_RAMP: f32 = 0.08;

// mask::range_weight. r0 = (lum_lo, lum_hi, lum_feather, hue),
// r1 = (hue_width, hue_feather, sat_min, _).
fn range_weight(px: vec3<f32>, lum_on: bool, color_on: bool, r0: vec4<f32>, r1: vec4<f32>) -> f32 {
    var w = 1.0;
    if (lum_on) {
        w *= band(luma(px), r0.x, r0.y, r0.z);
    }
    if (color_on && w > 0.0) {
        let hsl = rgb_to_hsl(px);
        let sat_gate = smoothstep01((hsl.y - r1.z) / SAT_GATE_RAMP);
        let f = max(r1.y, 1e-4);
        let hue_gate = smoothstep01((r1.x + f - hue_distance(hsl.x, r0.w)) / f);
        w *= sat_gate * hue_gate;
    }
    return w;
}

// Mask flag bits shared by the mask kernels.
const M_INVERTED: u32 = 1u;
const M_RANGE: u32 = 2u;
const M_COMPONENTS: u32 = 4u;
const M_LUM: u32 = 8u;
const M_COLOR: u32 = 16u;

// PreparedMask::weight: composed shapes (or the whole frame when there are
// none), inverted if asked, then narrowed by the range mask.
fn mask_weight(px: vec3<f32>, shapes: f32, flags: u32, r0: vec4<f32>, r1: vec4<f32>) -> f32 {
    let base = select(1.0, shapes, (flags & M_COMPONENTS) != 0u);
    var weight = invert_if(base, (flags & M_INVERTED) != 0u);
    if ((flags & M_RANGE) != 0u && weight > 1e-4) {
        weight *= range_weight(px, (flags & M_LUM) != 0u, (flags & M_COLOR) != 0u, r0, r1);
    }
    return weight;
}
