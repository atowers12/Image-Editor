// pipeline::mask_coverage: one mask's weight over the finished buffer, for
// the "show mask coverage" wash.

struct MaskWeight {
    dims: vec4<u32>,   // w, h, mask flags (M_*)
    range0: vec4<f32>, // lum lo, lum hi, lum feather, hue
    range1: vec4<f32>, // hue width, hue feather, sat min
}

@group(0) @binding(0) var<uniform> u: MaskWeight;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var weight_tex: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let px = textureLoad(src, id.xy, 0).rgb;
    let shapes = textureLoad(weight_tex, id.xy, 0).x;
    let w = mask_weight(px, shapes, u.dims.z, u.range0, u.range1);
    textureStore(dst, id.xy, vec4<f32>(clamp(w, 0.0, 1.0), 0.0, 0.0, 1.0));
}
