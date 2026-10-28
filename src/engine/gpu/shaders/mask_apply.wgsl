// One local-adjustment mask over the buffer (LocalPass::apply for a single
// mask): the mask's weight at each pixel, then a blend toward the fully
// adjusted pixel (local::target) by that weight.

struct MaskApply {
    dims: vec4<u32>,   // w, h, mask flags (M_*), neighbourhood flags (N_*)
    range0: vec4<f32>, // lum lo, lum hi, lum feather, hue
    range1: vec4<f32>, // hue width, hue feather, sat min
    gains: vec4<f32>,  // local WB gains (r, g, b), exposure gain
    adj0: vec4<f32>,   // noise, exposure, temp, tint
    adj1: vec4<f32>,   // highlights, shadows, whites, blacks
    adj2: vec4<f32>,   // contrast, dehaze, texture, clarity
    adj3: vec4<f32>,   // sharpness, vibrance, saturation
    k0: vec4<f32>,     // local strengths: tone range, texture, clarity, sharpen
    k1: vec4<f32>,     // local strengths: dehaze
}

@group(0) @binding(0) var<uniform> u: MaskApply;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var weight_tex: texture_2d<f32>;
@group(0) @binding(3) var blur_texture: texture_2d<f32>;
@group(0) @binding(4) var blur_clarity: texture_2d<f32>;
@group(0) @binding(5) var blur_sharpen: texture_2d<f32>;
@group(0) @binding(6) var denoised: texture_2d<f32>;
@group(0) @binding(7) var dst: texture_storage_2d<rgba32float, write>;

const N_TEXTURE: u32 = 1u;
const N_CLARITY: u32 = 2u;
const N_SHARPEN: u32 = 4u;
const N_DENOISED: u32 = 8u;

// local::target, with LocalPass::neighborhood inlined: a buffer that was not
// prepared falls back to the incoming pixel (its luma, or itself).
fn local_target(px: vec3<f32>, id: vec2<u32>) -> vec3<f32> {
    let nf = u.dims.w;
    let l0 = luma(px);
    var p = px;

    let nr = clamp(u.adj0.x / 100.0, 0.0, 1.0);
    if (nr > 0.0) {
        var den = px;
        if ((nf & N_DENOISED) != 0u) {
            den = textureLoad(denoised, id, 0).rgb;
        }
        p += (den - p) * nr;
    }

    if (u.adj0.y != 0.0) {
        p *= u.gains.w;
    }
    if (u.adj0.z != 0.0 || u.adj0.w != 0.0) {
        p *= u.gains.rgb;
    }

    let ranges = u.adj1 / 100.0;
    if (any(ranges != vec4<f32>(0.0))) {
        let ll = luma(p);
        let d = range_delta(ll, ranges, u.k0.x);
        if (d != 0.0) {
            p = p * (max(ll + d, 0.0) / max(ll, 1e-5));
        }
    }

    if (u.adj2.x != 0.0) {
        p = contrast3(p, u.adj2.x / 100.0);
    }

    if (u.adj2.y != 0.0) {
        p = dehaze3(p, u.adj2.y / 100.0, u.k1.x);
    }

    let tx = u.adj2.z / 100.0;
    let cl = u.adj2.w / 100.0;
    let sp = u.adj3.x / 100.0;
    if (tx != 0.0 || cl != 0.0 || sp != 0.0) {
        let ll = luma(p);
        var nl = ll;
        if (tx != 0.0) {
            let b = select(l0, textureLoad(blur_texture, id, 0).x, (nf & N_TEXTURE) != 0u);
            nl += tx * u.k0.y * (ll - b);
        }
        if (cl != 0.0) {
            let b = select(l0, textureLoad(blur_clarity, id, 0).x, (nf & N_CLARITY) != 0u);
            let wmid = max(1.0 - abs(2.0 * ll - 1.0), 0.0);
            nl += cl * u.k0.z * wmid * (ll - b);
        }
        if (sp != 0.0) {
            let b = select(l0, textureLoad(blur_sharpen, id, 0).x, (nf & N_SHARPEN) != 0u);
            nl += sp * u.k0.w * (ll - b);
        }
        if (nl != ll) {
            p = luma_ratio(p, ll, nl);
        }
    }

    if (u.adj3.y != 0.0 || u.adj3.z != 0.0) {
        let ll = luma(p);
        if (u.adj3.y != 0.0) {
            p = vibrance(p, ll, u.adj3.y / 100.0);
        }
        if (u.adj3.z != 0.0) {
            p = saturate_px(p, ll, u.adj3.z / 100.0);
        }
    }

    return clamp(p, vec3<f32>(0.0), vec3<f32>(1.0));
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    var px = textureLoad(src, id.xy, 0).rgb;
    let shapes = textureLoad(weight_tex, id.xy, 0).x;
    let weight = mask_weight(px, shapes, u.dims.z, u.range0, u.range1);
    if (weight > 1e-4) {
        px += (local_target(px, id.xy) - px) * weight;
    }
    textureStore(dst, id.xy, vec4<f32>(px, 1.0));
}
