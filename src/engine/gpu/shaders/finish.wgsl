// The end of render_rgb's pass 3: vignette (position-aware, in normalized
// full-image coordinates), then the final clamp.

struct Finish {
    dims: vec4<u32>, // w, h
    rect: vec4<f32>, // normalized rect of the full image this buffer covers
    vig: vec4<f32>,  // amount, strength, midpoint, feather
}

@group(0) @binding(0) var<uniform> u: Finish;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba32float, write>;

// vignette::gain
fn vignette_gain(nx: f32, ny: f32) -> f32 {
    let dx = nx * 2.0 - 1.0;
    let dy = ny * 2.0 - 1.0;
    let r = sqrt(dx * dx + dy * dy) / SQRT_2;
    let t = clamp((r - u.vig.z) / max(u.vig.w, 0.01), 0.0, 1.0);
    let m = t * t * (3.0 - 2.0 * t);
    return 1.0 + u.vig.x * u.vig.y * m;
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    var px = textureLoad(src, id.xy, 0).rgb;
    if (u.vig.x != 0.0) {
        let nx = u.rect.x + (f32(id.x) + 0.5) * (u.rect.z / f32(u.dims.x));
        let ny = u.rect.y + (f32(id.y) + 0.5) * (u.rect.w / f32(u.dims.y));
        px *= vignette_gain(nx, ny);
    }
    textureStore(dst, id.xy, vec4<f32>(clamp(px, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0));
}
