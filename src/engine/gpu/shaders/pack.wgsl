// Pack a window of the finished buffer into RGBA8 words for readback,
// rounding exactly like pipeline::rgb_to_rgba. With a coverage map, its
// value rides in the alpha byte; otherwise alpha is opaque.

struct Pack {
    dims: vec4<u32>, // window w, window h, window x, window y
    mode: vec4<u32>, // has coverage
}

@group(0) @binding(0) var<uniform> u: Pack;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var coverage: texture_2d<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

fn to_byte(v: f32) -> u32 {
    return min(u32(clamp(v, 0.0, 1.0) * 255.0 + 0.5), 255u);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let p = id.xy + u.dims.zw;
    let px = textureLoad(src, p, 0).rgb;
    var a = 255u;
    if (u.mode.x != 0u) {
        a = to_byte(textureLoad(coverage, p, 0).x);
    }
    out[id.y * u.dims.x + id.x] =
        to_byte(px.x) | (to_byte(px.y) << 8u) | (to_byte(px.z) << 16u) | (a << 24u);
}
