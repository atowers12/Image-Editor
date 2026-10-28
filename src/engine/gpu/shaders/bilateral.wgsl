// The smoothing half of sharpen::luminance_nr: a 5×5 bilateral filter over
// luma. Neighbours across a large luma jump get little weight, so edges hold.

struct Bilateral {
    dims: vec4<u32>,   // w, h
    params: vec4<f32>, // 1 / (2 σr²)
}

@group(0) @binding(0) var<uniform> u: Bilateral;
@group(0) @binding(1) var lum: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<r32float, write>;

const R: i32 = 2;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let p = vec2<i32>(id.xy);
    let hi = vec2<i32>(i32(u.dims.x) - 1, i32(u.dims.y) - 1);
    let center = textureLoad(lum, p, 0).x;
    var acc = 0.0;
    var wsum = 0.0;
    for (var dy = -R; dy <= R; dy++) {
        for (var dx = -R; dx <= R; dx++) {
            let s = textureLoad(lum, clamp(p + vec2<i32>(dx, dy), vec2<i32>(0), hi), 0).x;
            let dr = s - center;
            let fd = f32(dx * dx + dy * dy);
            let spatial = exp(-fd / (2.0 * 1.4 * 1.4));
            let wgt = spatial * exp(-dr * dr * u.params.x);
            acc += s * wgt;
            wsum += wgt;
        }
    }
    textureStore(dst, id.xy, vec4<f32>(acc / max(wsum, 1e-6), 0.0, 0.0, 1.0));
}
