//! One render on the GPU: `pipeline::render_rgb` stage for stage — same
//! order, same skips — then the finished pixels packed to RGBA8 and read
//! back. Everything derivable from the params (gains, LUTs, radii) is worked
//! out here on the CPU with the same code the CPU pipeline uses; the
//! shaders only consume it.

use anyhow::{anyhow, Result};
use wgpu::util::DeviceExt;

use crate::engine::blur::BLUR_PASSES;
use crate::engine::gpu::context::Context;
use crate::engine::gpu::kernels::{Kernel, Kernels, Res};
use crate::engine::gpu::pool::{Pool, Scratch, RGBA_SLOTS, R_SLOTS};
use crate::engine::gpu::uniforms as u;
use crate::engine::ops::{color, curve, detail, local};
use crate::engine::params::{EditParams, Mask, MaskKind, MaskOp, HSL_BAND_HUES};
use crate::engine::pipeline::{RenderCtx, StageRadii, SHARPEN_GAIN};
use crate::engine::tuning::Tuning;

/// What to render.
pub struct Job<'a> {
    /// The source texture, and the part of it (from the origin) that holds
    /// the image.
    pub src: &'a wgpu::TextureView,
    pub src_size: (u32, u32),
    /// Source rect to bilinear-sample, in source pixels (x, y, w, h), the way
    /// `SourceImage::sample_region` does. `None` reads texels 1:1.
    pub sample: Option<[f32; 4]>,
    /// Size of the rendered buffer.
    pub size: (u32, u32),
    pub ctx: RenderCtx,
    /// A mask whose coverage of the finished pixels rides back in alpha.
    pub overlay: Option<&'a Mask>,
    /// The part of the buffer to read back (x, y, w, h).
    pub window: [u32; 4],
}

/// Render `job` and read back its window as RGBA8 bytes.
pub fn render(
    gpu: &Context,
    kernels: &Kernels,
    pool: &mut Pool,
    job: &Job,
    p: &EditParams,
    t: &Tuning,
) -> Result<Vec<u8>> {
    let dev = &gpu.device;
    let out_bytes = job.window[2] as u64 * job.window[3] as u64 * 4;
    let (scratch, packed, readback) = pool.get(dev, job.size, out_bytes.max(4));

    let luts = curve::CurveLuts::build(&p.curve);
    let lut: Vec<[f32; 4]> = (0..curve::LUT_SIZE)
        .map(|i| [luts.master[i], luts.r[i], luts.g[i], luts.b[i]])
        .collect();
    let curve_buf = dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("curve"),
        contents: bytemuck::cast_slice(&lut),
        usage: wgpu::BufferUsages::STORAGE,
    });

    let mut encoder = dev.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        let mut graph = Graph {
            dev,
            k: kernels,
            s: scratch,
            pass: &mut pass,
            size: job.size,
        };
        graph.encode(job, p, t, &curve_buf, packed);
    }
    encoder.copy_buffer_to_buffer(packed, 0, readback, 0, out_bytes);
    gpu.queue.submit([encoder.finish()]);
    read_back(gpu, readback, out_bytes)
}

fn read_back(gpu: &Context, buffer: &wgpu::Buffer, bytes: u64) -> Result<Vec<u8>> {
    if bytes == 0 {
        return Ok(Vec::new());
    }
    let slice = buffer.slice(..bytes);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .map_err(|_| anyhow!("GPU readback never completed"))??;
    let data = slice.get_mapped_range().to_vec();
    buffer.unmap();
    Ok(data)
}

// Mask flags (M_* in common.wgsl) and neighbourhood flags (N_* in
// mask_apply.wgsl).
const M_INVERTED: u32 = 1;
const M_RANGE: u32 = 2;
const M_COMPONENTS: u32 = 4;
const M_LUM: u32 = 8;
const M_COLOR: u32 = 16;
const N_TEXTURE: u32 = 1;
const N_CLARITY: u32 = 2;
const N_SHARPEN: u32 = 4;
const N_DENOISED: u32 = 8;

/// The single-channel slot nothing writes during the mask stage; bound
/// wherever a kernel needs a texture it will not read.
const SPARE_R: usize = R_SLOTS - 1;

/// The first RGB slot not in `busy`.
fn free(busy: &[usize]) -> usize {
    (0..RGBA_SLOTS)
        .find(|i| !busy.contains(i))
        .expect("render graph ran out of RGB slots")
}

struct Graph<'a, 'p> {
    dev: &'a wgpu::Device,
    k: &'a Kernels,
    s: &'a Scratch,
    pass: &'a mut wgpu::ComputePass<'p>,
    size: (u32, u32),
}

impl<'a> Graph<'a, '_> {
    /// The kernels, borrowed for `'a` rather than through `self`, so one can
    /// be passed to `run`, which borrows `self` mutably.
    fn k(&self) -> &'a Kernels {
        self.k
    }

    fn rgba(&self, i: usize) -> Res<'a> {
        Res::View(&self.s.rgba[i].view)
    }

    fn r(&self, i: usize) -> Res<'a> {
        Res::View(&self.s.r[i].view)
    }

    fn dims(&self, z: u32, w: u32) -> [u32; 4] {
        [self.size.0, self.size.1, z, w]
    }

    fn run<U: bytemuck::Pod>(&mut self, kernel: &Kernel, uniform: &U, res: &[Res]) {
        kernel.run(self.dev, self.pass, uniform, res, self.size);
    }

    /// RGB slot `src` → luma in single-channel slot `dst`.
    fn luma(&mut self, src: usize, dst: usize) {
        let uniform = u::Dims { dims: self.dims(0, 0) };
        let res = [self.rgba(src), self.r(dst)];
        self.run(&self.k().luma, &uniform, &res);
    }

    /// `blur::gaussian_approx`: BLUR_PASSES × (x pass, y pass), `src` →
    /// `dst` via `tmp`.
    fn blur(&mut self, rgb: bool, src: Res<'a>, tmp: Res<'a>, dst: Res<'a>, radius: usize) {
        let kernel = if rgb { &self.k().box_rgba } else { &self.k().box_r };
        for i in 0..BLUR_PASSES {
            let from = if i == 0 { src } else { dst };
            let x = u::Dims { dims: self.dims(radius as u32, 0) };
            self.run(kernel, &x, &[from, tmp]);
            let y = u::Dims { dims: self.dims(radius as u32, 1) };
            self.run(kernel, &y, &[tmp, dst]);
        }
    }

    fn blur_r(&mut self, src: usize, tmp: usize, dst: usize, radius: usize) {
        self.blur(false, self.r(src), self.r(tmp), self.r(dst), radius);
    }

    /// The smoothing half of `sharpen::luminance_nr`: luma slot `src` →
    /// bilateral-filtered luma slot `dst`.
    fn bilateral(&mut self, src: usize, dst: usize, amount: f32) {
        let range_sigma = 0.04 + 0.16 * amount;
        let uniform = u::DimsK {
            dims: self.dims(0, 0),
            k: [1.0 / (2.0 * range_sigma * range_sigma), 0.0, 0.0, 0.0],
        };
        let res = [self.r(src), self.r(dst)];
        self.run(&self.k().bilateral, &uniform, &res);
    }

    /// A luma-only stage (see detail.wgsl): RGB `src` with luma slots
    /// `[l, a, b]` → RGB `dst`.
    fn detail(&mut self, mode: u32, src: usize, lum: [usize; 3], k: [f32; 4], dst: usize) {
        let uniform = u::DimsK {
            dims: self.dims(mode, 0),
            k,
        };
        let res = [
            self.rgba(src),
            self.r(lum[0]),
            self.r(lum[1]),
            self.r(lum[2]),
            self.rgba(dst),
        ];
        self.run(&self.k().detail, &uniform, &res);
    }

    /// Compose a mask's shapes into one weight (mask::fold) and return the
    /// single-channel slot holding it — slot 0 or 1, or `SPARE_R` when the
    /// mask has no shapes and covers the whole frame. Auto-masked brush dabs
    /// test their colors against RGB slot `reference`.
    fn compose(&mut self, m: &Mask, reference: usize, ctx: RenderCtx) -> usize {
        if m.components.is_empty() {
            return SPARE_R;
        }
        let (mut inp, mut out) = (1, 0);
        for (i, c) in m.components.iter().enumerate() {
            let (kind, geo0, geo1, dabs) = match &c.kind {
                MaskKind::Linear { p0, p1 } => (0, [p0[0], p0[1], p1[0], p1[1]], [0.0; 4], &[][..]),
                MaskKind::Radial {
                    center,
                    radius,
                    feather,
                } => (
                    1,
                    [center[0], center[1], radius[0], radius[1]],
                    [*feather, 0.0, 0.0, 0.0],
                    &[][..],
                ),
                MaskKind::Brush { dabs } => (2, [0.0; 4], [0.0; 4], &dabs[..]),
            };
            let mut packed: Vec<u::Dab> = dabs
                .iter()
                .map(|d| u::Dab {
                    shape: [d.p[0], d.p[1], d.radius, d.hardness],
                    color: d.auto.map_or([0.0; 4], |a| [a[0], a[1], a[2], 0.0]),
                    flags: [d.erase as u32, d.auto.is_some() as u32, 0, 0],
                })
                .collect();
            let count = packed.len() as u32;
            if packed.is_empty() {
                packed.push(u::Dab::default()); // bindings can't be empty
            }
            let dab_buf = self.dev.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("dabs"),
                contents: bytemuck::cast_slice(&packed),
                usage: wgpu::BufferUsages::STORAGE,
            });
            let op = match c.op {
                MaskOp::Add => 0,
                MaskOp::Subtract => 1,
                MaskOp::Intersect => 2,
            };
            let uniform = u::Component {
                dims: self.dims(kind, count),
                mode: [(i == 0) as u32, c.inverted as u32, op, 0],
                geo0,
                geo1,
                rect: ctx.norm_rect,
            };
            let res = [
                self.r(inp),
                self.rgba(reference),
                Res::Buffer(&dab_buf),
                self.r(out),
            ];
            self.run(&self.k().mask_component, &uniform, &res);
            std::mem::swap(&mut inp, &mut out);
        }
        inp
    }

    fn encode(&mut self, job: &Job, p: &EditParams, t: &Tuning, curve: &wgpu::Buffer, packed: &wgpu::Buffer) {
        let radii = StageRadii::new(p, t, job.ctx.radius_dim);

        // --- Pass 1: WB + exposure, encode, tone ranges, contrast, levels, curve.
        let exp = 2f32.powf(p.exposure);
        let (wr, wg, wb) = color::white_balance_gains(p.temp / 100.0, p.tint / 100.0);
        let ranges = [
            p.highlights / 100.0,
            p.shadows / 100.0,
            p.whites / 100.0,
            p.blacks / 100.0,
        ];
        let develop = u::Develop {
            dims: [self.size.0, self.size.1, job.src_size.0, job.src_size.1],
            mode: [
                job.sample.is_none() as u32,
                ranges.iter().any(|&r| r != 0.0) as u32,
                p.has_levels() as u32,
                p.has_curve() as u32,
            ],
            rect: job.sample.unwrap_or_default(),
            gain: [wr * exp, wg * exp, wb * exp, p.contrast / 100.0],
            ranges,
            levels: [p.lv_in_black, p.lv_in_white, p.lv_gamma, p.lv_out_black],
            extra: [p.lv_out_white, t.tone_range_strength, 0.0, 0.0],
        };
        let res = [Res::View(job.src), Res::Buffer(curve), self.rgba(0)];
        self.run(&self.k().develop, &develop, &res);
        let mut cur = 0;

        // --- Pass 2a: noise reduction.
        if radii.lum_nr {
            let amount = p.luminance_nr / 100.0;
            self.luma(cur, 0);
            self.bilateral(0, 1, amount);
            let next = free(&[cur]);
            self.detail(0, cur, [0, 1, 1], [amount, 0.0, 0.0, 0.0], next);
            cur = next;
        }
        if let Some(radius) = radii.color_nr {
            let tmp = free(&[cur]);
            let blurred = free(&[cur, tmp]);
            self.blur(true, self.rgba(cur), self.rgba(tmp), self.rgba(blurred), radius);
            // The blur's temporary is free again; recombine into it.
            let uniform = u::DimsK {
                dims: self.dims(0, 0),
                k: [p.color_nr / 100.0, 0.0, 0.0, 0.0],
            };
            let res = [self.rgba(cur), self.rgba(blurred), self.rgba(tmp)];
            self.run(&self.k().chroma, &uniform, &res);
            cur = tmp;
        }

        // --- Pass 2b: texture + clarity, then sharpen.
        if radii.any_detail() {
            let (tx, cl) = (p.texture / 100.0, p.clarity / 100.0);
            self.luma(cur, 0);
            if let Some(r) = radii.texture {
                self.blur_r(0, 1, 2, r);
            }
            if let Some(r) = radii.clarity {
                self.blur_r(0, 1, 3, r);
            }
            if tx != 0.0 || cl != 0.0 {
                let next = free(&[cur]);
                let k = [tx, cl, t.texture_strength, t.clarity_strength];
                self.detail(1, cur, [0, 2, 3], k, next);
                cur = next;
            }
            if let Some(r) = radii.sharpen {
                self.luma(cur, 0);
                self.blur_r(0, 1, 2, r);
                let next = free(&[cur]);
                let amount = p.sharpen / 100.0 * SHARPEN_GAIN;
                self.detail(2, cur, [0, 2, 2], [amount, 0.0, 0.0, 0.0], next);
                cur = next;
            }
        }

        // --- Local mask inputs, all taken from the buffer as it is now.
        let snap = cur;
        let mut neighbourhood = 0;
        let (mut blur_tx, mut blur_cl, mut blur_sp, mut denoised) = (SPARE_R, SPARE_R, SPARE_R, snap);
        if radii.any_local() {
            self.luma(snap, 0);
            if let Some(r) = radii.local_texture {
                self.blur_r(0, 1, 2, r);
                blur_tx = 2;
                neighbourhood |= N_TEXTURE;
            }
            if let Some(r) = radii.local_clarity {
                self.blur_r(0, 1, 3, r);
                blur_cl = 3;
                neighbourhood |= N_CLARITY;
            }
            if let Some(r) = radii.local_sharpen {
                self.blur_r(0, 1, 4, r);
                blur_sp = 4;
                neighbourhood |= N_SHARPEN;
            }
            if radii.local_denoise {
                // Full strength; each mask's Noise slider picks how far to go.
                self.bilateral(0, 1, 1.0);
                denoised = free(&[snap]);
                self.detail(0, snap, [0, 1, 1], [1.0, 0.0, 0.0, 0.0], denoised);
                neighbourhood |= N_DENOISED;
            }
        }

        // --- Pass 3a: dehaze, color mixer, vibrance / saturation.
        let dh = p.dehaze / 100.0;
        let vib = p.vibrance / 100.0;
        let sat = p.saturation / 100.0 + detail::dehaze_sat_boost(dh, t.dehaze_sat);
        let any_hsl = p.any_hsl();
        if dh != 0.0 || any_hsl || vib != 0.0 || sat != 0.0 {
            let mut bands = [[0.0f32; 4]; 8];
            for (i, b) in p.hsl.iter().enumerate() {
                bands[i] = [b.hue, b.sat, b.lum, HSL_BAND_HUES[i]];
            }
            let uniform = u::Color {
                dims: self.dims(any_hsl as u32, 0),
                k: [dh, t.dehaze_strength, vib, sat],
                bands,
            };
            let next = free(&[snap, denoised]);
            let res = [self.rgba(cur), self.rgba(next)];
            self.run(&self.k().color, &uniform, &res);
            cur = next;
        }

        // --- Pass 3b: local masks, one after another.
        for m in p.active_masks() {
            let shapes = self.compose(m, snap, job.ctx);
            let a = &m.adjust;
            let (gr, gg, gb) = color::white_balance_gains(a.temp / 100.0, a.tint / 100.0);
            let uniform = u::MaskApply {
                dims: self.dims(mask_flags(m), neighbourhood),
                range0: range0(m),
                range1: range1(m),
                gains: [gr, gg, gb, 2f32.powf(a.exposure / 100.0 * 2.0)],
                adj0: [a.noise, a.exposure, a.temp, a.tint],
                adj1: [a.highlights, a.shadows, a.whites, a.blacks],
                adj2: [a.contrast, a.dehaze, a.texture, a.clarity],
                adj3: [a.sharpness, a.vibrance, a.saturation, 0.0],
                k0: [
                    local::LOCAL_RANGE_STRENGTH,
                    local::LOCAL_TEXTURE_STRENGTH,
                    local::LOCAL_CLARITY_STRENGTH,
                    local::LOCAL_SHARPEN_STRENGTH,
                ],
                k1: [local::LOCAL_DEHAZE_STRENGTH, 0.0, 0.0, 0.0],
            };
            let next = free(&[cur, snap, denoised]);
            let res = [
                self.rgba(cur),
                self.r(shapes),
                self.r(blur_tx),
                self.r(blur_cl),
                self.r(blur_sp),
                self.rgba(denoised),
                self.rgba(next),
            ];
            self.run(&self.k().mask_apply, &uniform, &res);
            cur = next;
        }

        // --- Pass 3c: vignette + final clamp.
        let fin = free(&[cur]);
        let uniform = u::Finish {
            dims: self.dims(0, 0),
            rect: job.ctx.norm_rect,
            vig: [
                p.vignette / 100.0,
                t.vignette_strength,
                t.vignette_midpoint,
                t.vignette_feather,
            ],
        };
        let res = [self.rgba(cur), self.rgba(fin)];
        self.run(&self.k().finish, &uniform, &res);

        // --- Coverage wash, measured against the finished pixels.
        let coverage = job.overlay.map(|m| {
            let shapes = self.compose(m, fin, job.ctx);
            let uniform = u::MaskWeight {
                dims: self.dims(mask_flags(m), 0),
                range0: range0(m),
                range1: range1(m),
            };
            let res = [self.rgba(fin), self.r(shapes), self.r(2)];
            self.run(&self.k().mask_weight, &uniform, &res);
            2
        });

        // --- Pack the window for readback.
        let [x, y, w, h] = job.window;
        let uniform = u::Pack {
            dims: [w, h, x, y],
            mode: [coverage.is_some() as u32, 0, 0, 0],
        };
        let res = [
            self.rgba(fin),
            self.r(coverage.unwrap_or(SPARE_R)),
            Res::Buffer(packed),
        ];
        self.k.pack.run(self.dev, self.pass, &uniform, &res, (w, h));
    }
}

fn mask_flags(m: &Mask) -> u32 {
    let mut f = 0;
    if m.inverted {
        f |= M_INVERTED;
    }
    if m.range.is_active() {
        f |= M_RANGE;
    }
    if !m.components.is_empty() {
        f |= M_COMPONENTS;
    }
    if m.range.lum_enabled {
        f |= M_LUM;
    }
    if m.range.color_enabled {
        f |= M_COLOR;
    }
    f
}

fn range0(m: &Mask) -> [f32; 4] {
    let r = &m.range;
    [r.lum_lo, r.lum_hi, r.lum_feather, r.hue]
}

fn range1(m: &Mask) -> [f32; 4] {
    let r = &m.range;
    [r.hue_width, r.hue_feather, r.sat_min, 0.0]
}
