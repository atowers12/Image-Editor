// Pass 1 of render_rgb: fetch the source pixel (texel-exact, or bilinear like
// SourceImage::sample_region for zoomed regions), then white balance and
// exposure in linear light, sRGB encode, tone ranges, contrast, levels, curve.

struct Develop {
    dims: vec4<u32>,   // out w, out h, src w, src h
    mode: vec4<u32>,   // direct read, any tone range, has levels, has curve
    rect: vec4<f32>,   // sampled source rect in source pixels: x0, y0, w, h
    gain: vec4<f32>,   // WB × exposure gains (r, g, b), contrast
    ranges: vec4<f32>, // highlights, shadows, whites, blacks
    levels: vec4<f32>, // in black, in white, gamma, out black
    extra: vec4<f32>,  // out white, tone range strength
}

@group(0) @binding(0) var<uniform> u: Develop;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> curve: array<vec4<f32>>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba32float, write>;

fn fetch(x: u32, y: u32) -> vec3<f32> {
    if (u.mode.x != 0u) {
        return textureLoad(src, vec2<u32>(x, y), 0).rgb;
    }
    let sw = u.dims.z;
    let sh = u.dims.w;
    let sy = u.rect.y + (f32(y) + 0.5) / f32(u.dims.y) * u.rect.w - 0.5;
    let iy0 = min(u32(max(floor(sy), 0.0)), sh - 1u);
    let iy1 = min(iy0 + 1u, sh - 1u);
    let fy = clamp(sy - f32(iy0), 0.0, 1.0);
    let sx = u.rect.x + (f32(x) + 0.5) / f32(u.dims.x) * u.rect.z - 0.5;
    let ix0 = min(u32(max(floor(sx), 0.0)), sw - 1u);
    let ix1 = min(ix0 + 1u, sw - 1u);
    let fx = clamp(sx - f32(ix0), 0.0, 1.0);
    let p00 = textureLoad(src, vec2<u32>(ix0, iy0), 0).rgb;
    let p10 = textureLoad(src, vec2<u32>(ix1, iy0), 0).rgb;
    let p01 = textureLoad(src, vec2<u32>(ix0, iy1), 0).rgb;
    let p11 = textureLoad(src, vec2<u32>(ix1, iy1), 0).rgb;
    let top = p00 + (p10 - p00) * fx;
    let bot = p01 + (p11 - p01) * fx;
    return top + (bot - top) * fy;
}

fn levels(x: f32) -> f32 {
    let t0 = clamp((x - u.levels.x) / max(u.levels.y - u.levels.x, 1e-4), 0.0, 1.0);
    let t = select(pow(t0, 1.0 / max(u.levels.z, 0.05)), 0.0, t0 <= 0.0);
    return u.levels.w + t * (u.extra.x - u.levels.w);
}

// curve::sample on one column of the packed LUT (0 master, 1-3 r/g/b).
fn curve_lut(v: f32, col: u32) -> f32 {
    let x = clamp(v, 0.0, 1.0) * 255.0;
    let i = u32(floor(x));
    if (i >= 255u) {
        return curve[255][col];
    }
    let f = x - f32(i);
    return curve[i][col] + (curve[i + 1u][col] - curve[i][col]) * f;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let lin = fetch(id.x, id.y) * u.gain.rgb;
    var px = vec3<f32>(srgb_encode(lin.x), srgb_encode(lin.y), srgb_encode(lin.z));
    if (u.mode.y != 0u) {
        let l = luma(px);
        let d = range_delta(l, u.ranges, u.extra.y);
        if (d != 0.0) {
            px = px * (max(l + d, 0.0) / max(l, 1e-5));
        }
    }
    let c = u.gain.w;
    if (c != 0.0) {
        px = contrast3(px, c);
    } else {
        px = clamp(px, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (u.mode.z != 0u) {
        px = vec3<f32>(levels(px.x), levels(px.y), levels(px.z));
    }
    if (u.mode.w != 0u) {
        px = vec3<f32>(
            curve_lut(curve_lut(px.x, 0u), 1u),
            curve_lut(curve_lut(px.y, 0u), 2u),
            curve_lut(curve_lut(px.z, 0u), 3u),
        );
    }
    textureStore(dst, id.xy, vec4<f32>(px, 1.0));
}
