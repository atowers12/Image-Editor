// Luma of an RGB buffer into a single-channel one.

struct Dims {
    dims: vec4<u32>, // w, h
}

@group(0) @binding(0) var<uniform> u: Dims;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<r32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let l = luma(textureLoad(src, id.xy, 0).rgb);
    textureStore(dst, id.xy, vec4<f32>(l, 0.0, 0.0, 1.0));
}
