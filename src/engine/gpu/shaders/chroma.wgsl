// The recombine half of sharpen::color_nr: take color from the blurred
// buffer, luma from the sharp one, and blend toward that by `amount`.

struct Chroma {
    dims: vec4<u32>, // w, h
    k: vec4<f32>,    // amount
}

@group(0) @binding(0) var<uniform> u: Chroma;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var blurred: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let px = textureLoad(src, id.xy, 0).rgb;
    let b = textureLoad(blurred, id.xy, 0).rgb;
    let l = luma(px);
    let bl = luma(b);
    let t = clamp(b - vec3<f32>(bl) + vec3<f32>(l), vec3<f32>(0.0), vec3<f32>(1.0));
    textureStore(dst, id.xy, vec4<f32>(px + (t - px) * u.k.x, 1.0));
}
