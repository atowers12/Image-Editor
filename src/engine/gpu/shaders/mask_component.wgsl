// One shape of a mask: its coverage at each pixel (PreparedComponent::
// coverage), folded into the weight composed so far (mask::fold). The first
// shape sets the base; later ones combine by their operator.

struct Component {
    dims: vec4<u32>, // w, h, kind (0 linear, 1 radial, 2 brush), dab count
    mode: vec4<u32>, // is first, inverted, operator
    geo0: vec4<f32>, // linear: p0, p1 · radial: center, radius
    geo1: vec4<f32>, // radial: feather
    rect: vec4<f32>, // normalized rect of the full image this buffer covers
}

struct Dab {
    shape: vec4<f32>, // x, y, radius, hardness
    color: vec4<f32>, // auto-mask reference color
    flags: vec4<u32>, // erase, has auto-mask reference
}

@group(0) @binding(0) var<uniform> u: Component;
@group(0) @binding(1) var weight_in: texture_2d<f32>;
@group(0) @binding(2) var reference: texture_2d<f32>;
@group(0) @binding(3) var<storage, read> dabs: array<Dab>;
@group(0) @binding(4) var dst: texture_storage_2d<r32float, write>;

const AUTO_NEAR: f32 = 0.05;
const AUTO_FAR: f32 = 0.22;

// mask::auto_mask_factor
fn auto_mask_factor(px: vec3<f32>, r: vec3<f32>) -> f32 {
    let dl = luma(px) - luma(r);
    let dc = px - r;
    let dist = sqrt(dl * dl * 2.0 + (dc.x * dc.x + dc.y * dc.y + dc.z * dc.z) / 3.0);
    return smoothstep01((AUTO_FAR - dist) / (AUTO_FAR - AUTO_NEAR));
}

// The per-pixel body of mask::rasterize_brush.
fn brush(x: u32, y: u32) -> f32 {
    let ny = u.rect.y + (f32(y) + 0.5) / f32(u.dims.y) * u.rect.w;
    let nx = u.rect.x + (f32(x) + 0.5) / f32(u.dims.x) * u.rect.z;
    let px = textureLoad(reference, vec2<u32>(x, y), 0).rgb;
    var cov = 0.0;
    for (var i = 0u; i < u.dims.w; i++) {
        let dab = dabs[i];
        let r = max(dab.shape.z, 1e-4);
        let dx = (nx - dab.shape.x) / r;
        let dy = (ny - dab.shape.y) / r;
        let d = sqrt(dx * dx + dy * dy);
        if (d >= 1.0) {
            continue;
        }
        let hard = clamp(dab.shape.w, 0.0, 0.99);
        var a = 1.0;
        if (d > hard) {
            a = smoothstep01(1.0 - (d - hard) / (1.0 - hard));
        }
        if (dab.flags.y != 0u) {
            a *= auto_mask_factor(px, dab.color.rgb);
            if (a <= 0.0) {
                continue;
            }
        }
        if (dab.flags.x != 0u) {
            cov *= 1.0 - a;
        } else {
            cov += a * (1.0 - cov);
        }
    }
    return clamp(cov, 0.0, 1.0);
}

@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= u.dims.x || id.y >= u.dims.y) {
        return;
    }
    var raw: f32;
    if (u.dims.z == 2u) {
        raw = brush(id.x, id.y);
    } else {
        // Analytic shapes use render_rgb's pass-3 coordinate mapping.
        let nxs = u.rect.z / f32(u.dims.x);
        let nys = u.rect.w / f32(u.dims.y);
        let n = vec2<f32>(u.rect.x + (f32(id.x) + 0.5) * nxs, u.rect.y + (f32(id.y) + 0.5) * nys);
        if (u.dims.z == 0u) {
            raw = linear_weight(u.geo0.xy, u.geo0.zw, n);
        } else {
            raw = radial_weight(u.geo0.xy, u.geo0.zw, u.geo1.x, n);
        }
    }
    let c = invert_if(raw, u.mode.y != 0u);
    var w = c;
    if (u.mode.x == 0u) {
        w = combine(u.mode.z, textureLoad(weight_in, id.xy, 0).x, c);
    }
    textureStore(dst, id.xy, vec4<f32>(w, 0.0, 0.0, 1.0));
}
