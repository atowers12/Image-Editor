//! GPU memory: grow-only scratch textures for the render graph, the
//! readback buffers, and source images uploaded from the CPU.

use rayon::prelude::*;

use crate::engine::gpu::context::Context;
use crate::engine::pipeline::SourceImage;

/// RGB working buffers. Four cover the busiest point of the graph: the
/// post-detail snapshot local masks read, the local denoised copy, and a
/// ping-pong pair.
pub const RGBA_SLOTS: usize = 4;
/// Single-channel buffers: luma, a blur temporary, three local blurs, and
/// a spare that is never written during the mask stage (bound wherever a
/// kernel needs a texture it won't read).
pub const R_SLOTS: usize = 6;

pub struct Texture {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl Texture {
    pub fn new(
        device: &wgpu::Device,
        (w, h): (u32, u32),
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Texture { texture, view }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}

/// The render graph's working set. Textures are sized to the largest render
/// seen so far; kernels take the logical size as a uniform, so a smaller
/// render (a different zoom, a narrower tile) reuses them as they are.
pub struct Scratch {
    pub rgba: Vec<Texture>,
    pub r: Vec<Texture>,
}

#[derive(Default)]
pub struct Pool {
    scratch: Option<Scratch>,
    /// Packed RGBA8 output (storage) and its CPU-mappable twin.
    out: Option<(wgpu::Buffer, wgpu::Buffer)>,
}

impl Pool {
    /// Scratch textures at least `size`, plus the packed-output buffer and its
    /// readback twin, at least `out_bytes` long.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        out_bytes: u64,
    ) -> (&Scratch, &wgpu::Buffer, &wgpu::Buffer) {
        self.ensure_scratch(device, size);
        self.ensure_out(device, out_bytes);
        let (packed, readback) = self.out.as_ref().expect("just allocated");
        (self.scratch.as_ref().expect("just allocated"), packed, readback)
    }

    fn ensure_scratch(&mut self, device: &wgpu::Device, (w, h): (u32, u32)) {
        let fits = self.scratch.as_ref().is_some_and(|s| {
            let (sw, sh) = s.rgba[0].size();
            sw >= w && sh >= h
        });
        if !fits {
            // Grow both axes to the larger of old and new, so alternating
            // landscape/portrait requests don't reallocate every time.
            let (ow, oh) = self.scratch.as_ref().map_or((0, 0), |s| s.rgba[0].size());
            let size = (w.max(ow), h.max(oh));
            self.scratch = None; // free before allocating the replacement
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING;
            let make = |format, n| {
                (0..n)
                    .map(|_| Texture::new(device, size, format, usage, "scratch"))
                    .collect()
            };
            self.scratch = Some(Scratch {
                rgba: make(wgpu::TextureFormat::Rgba32Float, RGBA_SLOTS),
                r: make(wgpu::TextureFormat::R32Float, R_SLOTS),
            });
        }
    }

    fn ensure_out(&mut self, device: &wgpu::Device, bytes: u64) {
        if !self.out.as_ref().is_some_and(|(b, _)| b.size() >= bytes) {
            self.out = None;
            let make = |usage, label| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes,
                    usage,
                    mapped_at_creation: false,
                })
            };
            self.out = Some((
                make(
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    "packed",
                ),
                make(
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    "readback",
                ),
            ));
        }
    }

    /// Release everything; the next render reallocates at its own size.
    pub fn trim(&mut self) {
        self.scratch = None;
        self.out = None;
    }
}

/// How a source image is stored on the GPU.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// Half floats: what resident sources use, at half the memory. Linear
    /// light in f16 keeps ~11 bits of precision, far below an 8-bit step.
    Half,
    /// Full floats: export tiles, so an export matches the CPU's input bit
    /// for bit.
    Full,
}

impl SourceFormat {
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            SourceFormat::Half => wgpu::TextureFormat::Rgba16Float,
            SourceFormat::Full => wgpu::TextureFormat::Rgba32Float,
        }
    }

    fn bytes_per_px(self) -> usize {
        match self {
            SourceFormat::Half => 8,
            SourceFormat::Full => 16,
        }
    }
}

/// A texture a source image can be uploaded into.
pub fn source_texture(device: &wgpu::Device, size: (u32, u32), format: SourceFormat) -> Texture {
    Texture::new(
        device,
        size,
        format.texture_format(),
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        "source",
    )
}

/// Upload the `rect` (x, y, w, h) of `img` into the top-left of `target`,
/// widening RGB to RGBA. Goes up in bands of rows, each flushed as it is
/// queued, so a full-resolution photo never needs a second full-size copy
/// staged in memory at once.
pub fn upload(gpu: &Context, img: &SourceImage, rect: [usize; 4], target: &Texture, format: SourceFormat) {
    const BAND_BYTES: usize = 16 << 20;
    let [x0, y0, w, h] = rect;
    let row_bytes = w * format.bytes_per_px();
    let band_rows = (BAND_BYTES / row_bytes.max(1)).max(1);
    let mut y = 0;
    while y < h {
        let rows = band_rows.min(h - y);
        let src_row = |r: usize| {
            let start = ((y0 + y + r) * img.width + x0) * 3;
            &img.data[start..start + w * 3]
        };
        let bytes: Vec<u8> = match format {
            SourceFormat::Half => {
                let mut v = vec![0u16; rows * w * 4];
                v.par_chunks_mut(w * 4).enumerate().for_each(|(r, out)| {
                    for (o, i) in out.chunks_exact_mut(4).zip(src_row(r).chunks_exact(3)) {
                        o[0] = half::f16::from_f32(i[0]).to_bits();
                        o[1] = half::f16::from_f32(i[1]).to_bits();
                        o[2] = half::f16::from_f32(i[2]).to_bits();
                        o[3] = half::f16::ONE.to_bits();
                    }
                });
                bytemuck::cast_slice(&v).to_vec()
            }
            SourceFormat::Full => {
                let mut v = vec![0f32; rows * w * 4];
                v.par_chunks_mut(w * 4).enumerate().for_each(|(r, out)| {
                    for (o, i) in out.chunks_exact_mut(4).zip(src_row(r).chunks_exact(3)) {
                        o[..3].copy_from_slice(i);
                        o[3] = 1.0;
                    }
                });
                bytemuck::cast_slice(&v).to_vec()
            }
        };
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: y as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes as u32),
                rows_per_image: Some(rows as u32),
            },
            wgpu::Extent3d {
                width: w as u32,
                height: rows as u32,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(std::iter::empty());
        y += rows;
    }
}
