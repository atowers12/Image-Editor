//! The compute pipelines — one per shader in `shaders/` — and the helper
//! that binds one kernel's resources and dispatches it over an image.

use wgpu::util::DeviceExt;

/// Every shader is `common.wgsl` followed by its own file (WGSL has no
/// `#include`).
const COMMON: &str = include_str!("shaders/common.wgsl");

/// Workgroup edge, matching `@workgroup_size(16, 16)` in every shader.
const WG: u32 = 16;

/// What a binding holds. Binding 0 is always the kernel's uniform block;
/// these describe bindings 1.. in order.
#[derive(Clone, Copy)]
enum Bind {
    /// `texture_2d<f32>`, read with `textureLoad`.
    Texture,
    /// `texture_storage_2d<rgba32float, write>`.
    StoreRgba,
    /// `texture_storage_2d<r32float, write>`.
    StoreR,
    /// `var<storage, read>` buffer.
    Storage,
    /// `var<storage, read_write>` buffer.
    StorageRw,
}

/// A resource bound to one of a kernel's bindings.
#[derive(Clone, Copy)]
pub enum Res<'a> {
    View(&'a wgpu::TextureView),
    Buffer(&'a wgpu::Buffer),
}

pub struct Kernel {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl Kernel {
    fn new(device: &wgpu::Device, name: &str, body: &str, binds: &[Bind]) -> Self {
        let uniform = wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let storage_texture = |format| wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D2,
        };
        let storage_buffer = |read_only| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let entries: Vec<_> = std::iter::once(uniform)
            .chain(binds.iter().map(|b| match b {
                Bind::Texture => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                Bind::StoreRgba => storage_texture(wgpu::TextureFormat::Rgba32Float),
                Bind::StoreR => storage_texture(wgpu::TextureFormat::R32Float),
                Bind::Storage => storage_buffer(true),
                Bind::StorageRw => storage_buffer(false),
            }))
            .enumerate()
            .map(|(i, ty)| wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(name),
            entries: &entries,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(format!("{COMMON}\n{body}").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(name),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Kernel { pipeline, layout }
    }

    /// Bind `uniform` and `res` (bindings 1.. in order) and run the kernel
    /// once per pixel of a `size` image.
    pub fn run<U: bytemuck::Pod>(
        &self,
        device: &wgpu::Device,
        pass: &mut wgpu::ComputePass<'_>,
        uniform: &U,
        res: &[Res],
        size: (u32, u32),
    ) {
        let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let entries: Vec<_> = std::iter::once(ubuf.as_entire_binding())
            .chain(res.iter().map(|r| match r {
                Res::View(v) => wgpu::BindingResource::TextureView(v),
                Res::Buffer(b) => b.as_entire_binding(),
            }))
            .enumerate()
            .map(|(i, resource)| wgpu::BindGroupEntry {
                binding: i as u32,
                resource,
            })
            .collect();
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(size.0.div_ceil(WG), size.1.div_ceil(WG), 1);
    }
}

pub struct Kernels {
    pub develop: Kernel,
    pub luma: Kernel,
    pub box_r: Kernel,
    pub box_rgba: Kernel,
    pub bilateral: Kernel,
    pub detail: Kernel,
    pub chroma: Kernel,
    pub color: Kernel,
    pub mask_component: Kernel,
    pub mask_apply: Kernel,
    pub mask_weight: Kernel,
    pub finish: Kernel,
    pub pack: Kernel,
}

impl Kernels {
    pub fn new(device: &wgpu::Device) -> Self {
        use Bind::*;
        macro_rules! kernel {
            ($name:literal, [$($b:expr),*]) => {
                Kernel::new(
                    device,
                    $name,
                    include_str!(concat!("shaders/", $name, ".wgsl")),
                    &[$($b),*],
                )
            };
        }
        Kernels {
            develop: kernel!("develop", [Texture, Storage, StoreRgba]),
            luma: kernel!("luma", [Texture, StoreR]),
            box_r: kernel!("box_r", [Texture, StoreR]),
            box_rgba: kernel!("box_rgba", [Texture, StoreRgba]),
            bilateral: kernel!("bilateral", [Texture, StoreR]),
            detail: kernel!("detail", [Texture, Texture, Texture, Texture, StoreRgba]),
            chroma: kernel!("chroma", [Texture, Texture, StoreRgba]),
            color: kernel!("color", [Texture, StoreRgba]),
            mask_component: kernel!("mask_component", [Texture, Texture, Storage, StoreR]),
            mask_apply: kernel!(
                "mask_apply",
                [Texture, Texture, Texture, Texture, Texture, Texture, StoreRgba]
            ),
            mask_weight: kernel!("mask_weight", [Texture, Texture, StoreR]),
            finish: kernel!("finish", [Texture, StoreRgba]),
            pack: kernel!("pack", [Texture, Texture, StorageRw]),
        }
    }
}
