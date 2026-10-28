//! Uniform blocks, one per kernel, mirroring the structs at the top of each
//! shader. Every field is a 16-byte `[u32; 4]` / `[f32; 4]` row, so the Rust
//! and WGSL layouts agree without any padding rules coming into play. The
//! comments on the WGSL side say what each lane holds.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Develop {
    pub dims: [u32; 4],
    pub mode: [u32; 4],
    pub rect: [f32; 4],
    pub gain: [f32; 4],
    pub ranges: [f32; 4],
    pub levels: [f32; 4],
    pub extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Dims {
    pub dims: [u32; 4],
}

/// Shared by `bilateral`, `detail` and `chroma`: dimensions (+ mode) and
/// four scalars.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct DimsK {
    pub dims: [u32; 4],
    pub k: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Color {
    pub dims: [u32; 4],
    pub k: [f32; 4],
    pub bands: [[f32; 4]; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Component {
    pub dims: [u32; 4],
    pub mode: [u32; 4],
    pub geo0: [f32; 4],
    pub geo1: [f32; 4],
    pub rect: [f32; 4],
}

/// One brush dab, as stored in the component kernel's dab buffer.
#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Dab {
    pub shape: [f32; 4],
    pub color: [f32; 4],
    pub flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct MaskApply {
    pub dims: [u32; 4],
    pub range0: [f32; 4],
    pub range1: [f32; 4],
    pub gains: [f32; 4],
    pub adj0: [f32; 4],
    pub adj1: [f32; 4],
    pub adj2: [f32; 4],
    pub adj3: [f32; 4],
    pub k0: [f32; 4],
    pub k1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct MaskWeight {
    pub dims: [u32; 4],
    pub range0: [f32; 4],
    pub range1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Finish {
    pub dims: [u32; 4],
    pub rect: [f32; 4],
    pub vig: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, Pod, Zeroable)]
pub struct Pack {
    pub dims: [u32; 4],
    pub mode: [u32; 4],
}

// WGSL sizes: 16 bytes per row.
const _: () = {
    use std::mem::size_of;
    assert!(size_of::<Develop>() == 7 * 16);
    assert!(size_of::<Color>() == 10 * 16);
    assert!(size_of::<Component>() == 5 * 16);
    assert!(size_of::<Dab>() == 3 * 16);
    assert!(size_of::<MaskApply>() == 10 * 16);
    assert!(size_of::<MaskWeight>() == 3 * 16);
    assert!(size_of::<Finish>() == 3 * 16);
};
