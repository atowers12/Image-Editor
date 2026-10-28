// The luma-only stages: each computes a new luma from the pixel's luma `l`
// and blurred/smoothed lumas `a`, `b`, then scales RGB to match and clamps.
//   mode 0  luminance NR apply   (a = bilateral-smoothed luma)
//   mode 1  texture + clarity    (a = small blur, b = large blur)
//   mode 2  unsharp mask         (a = sharpen-radius blur)

struct Detail {
    dims: vec4<u32>, // w, h, mode
    k: vec4<f32>,    // mode 0: amount · 1: texture, clarity, their strengths · 2: amount
}

@group(0) @binding(0) var<uniform> u: Detail;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var lum: texture_2d<f32>;
@group(0) @binding(3) var lum_a: texture_2d<f32>;
@group(0) @binding(4) var lum_b: texture_2d<f32>;
@group(0) @binding(5) var dst: texture_storage_2d<rgba32float, write>;

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    var px = textureLoad(src, id.xy, 0).rgb;
    let l = textureLoad(lum, id.xy, 0).x;
    var nl = l;
    switch (u.dims.z) {
        case 0u: {
            nl = l + (textureLoad(lum_a, id.xy, 0).x - l) * u.k.x;
        }
        case 1u: {
            // detail::texture_clarity
            if (u.k.x != 0.0) {
                nl += u.k.x * u.k.z * (l - textureLoad(lum_a, id.xy, 0).x);
            }
            if (u.k.y != 0.0) {
                let wmid = max(1.0 - abs(2.0 * l - 1.0), 0.0);
                nl += u.k.y * u.k.w * wmid * (l - textureLoad(lum_b, id.xy, 0).x);
            }
        }
        default: {
            nl = l + u.k.x * (l - textureLoad(lum_a, id.xy, 0).x);
        }
    }
    if (nl != l) {
        px = clamp(luma_ratio(px, l, nl), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    textureStore(dst, id.xy, vec4<f32>(px, 1.0));
}
