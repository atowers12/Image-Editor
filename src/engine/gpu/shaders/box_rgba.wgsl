// One box-blur pass along one axis, edge-clamped — a single sweep of
// blur::box_blur_rows. blur::gaussian_approx is BLUR_PASSES × (x, y) of these.

struct Box {
    dims: vec4<u32>, // w, h, radius, axis (0 = x, 1 = y)
}

@group(0) @binding(0) var<uniform> u: Box;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    let r = i32(u.dims.z);
    let p = vec2<i32>(id.xy);
    let dir = select(vec2<i32>(0, 1), vec2<i32>(1, 0), u.dims.w == 0u);
    let hi = vec2<i32>(i32(u.dims.x) - 1, i32(u.dims.y) - 1);
    var acc: vec4<f32> = vec4<f32>(0.0);
    for (var i = -r; i <= r; i++) {
        acc += textureLoad(src, clamp(p + dir * i, vec2<i32>(0), hi), 0);
    }
    let norm = 1.0 / f32(2 * r + 1);
    textureStore(dst, id.xy, acc * norm);
}
