// The color half of render_rgb's pass 3: dehaze, the 8-band color mixer,
// then vibrance and saturation. Like the CPU pass, it does not clamp; the
// finish kernel does.

struct Color {
    dims: vec4<u32>,            // w, h, any mixer band set
    k: vec4<f32>,               // dehaze, dehaze strength, vibrance, saturation
    bands: array<vec4<f32>, 8>, // per band: hue, sat, lum sliders, center hue
}

@group(0) @binding(0) var<uniform> u: Color;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;

// hsl::band_weight
fn band_weight(h: f32, center: f32) -> f32 {
    let d = hue_distance(h, center) / 30.0;
    return exp(-0.5 * d * d);
}

// hsl::apply
fn mixer(p: vec3<f32>) -> vec3<f32> {
    let hsl = rgb_to_hsl(p);
    let h = hsl.x;
    let s = hsl.y;
    let l = hsl.z;
    if (s < 0.02) {
        return p;
    }
    let sat_gate = clamp((s - 0.02) / 0.13, 0.0, 1.0);
    var hue_shift = 0.0;
    var sat_scale = 0.0;
    var lum_scale = 0.0;
    for (var i = 0u; i < 8u; i++) {
        let b = u.bands[i];
        if (b.x == 0.0 && b.y == 0.0 && b.z == 0.0) {
            continue;
        }
        let w = band_weight(h, b.w);
        if (w < 1e-3) {
            continue;
        }
        hue_shift += w * b.x * 0.30;
        sat_scale += w * b.y / 100.0;
        lum_scale += w * b.z / 100.0;
    }
    if (hue_shift == 0.0 && sat_scale == 0.0 && lum_scale == 0.0) {
        return p;
    }
    let nh = h + hue_shift * sat_gate;
    let ns = clamp(s * (1.0 + sat_scale * sat_gate), 0.0, 1.0);
    let nl = clamp(l * (1.0 + 0.6 * lum_scale * sat_gate), 0.0, 1.0);
    return hsl_to_rgb(nh, ns, nl);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    var px = textureLoad(src, id.xy, 0).rgb;
    if (u.k.x != 0.0) {
        px = dehaze3(px, u.k.x, u.k.y);
    }
    if (u.dims.z != 0u) {
        px = mixer(px);
    }
    if (u.k.z != 0.0 || u.k.w != 0.0) {
        let l = luma(px);
        if (u.k.z != 0.0) {
            px = vibrance(px, l, u.k.z);
        }
        if (u.k.w != 0.0) {
            px = saturate_px(px, l, u.k.w);
        }
    }
    textureStore(dst, id.xy, vec4<f32>(px, 1.0));
}
