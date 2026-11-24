//! The shared texture's description in the viewer header: the adapter that owns it, its size and
//! format, and each eye's rectangle in it.

use core::mem::offset_of;

/// The locally unique id of the adapter that owns the shared texture, as DXGI reports it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdapterLuid {
    pub low: u32,
    pub high: i32,
}

/// One eye's rectangle in the shared texture, in pixels.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EyeRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl EyeRect {
    /// The rectangle that covers an entire `width` by `height` image.
    pub const fn full(width: u32, height: u32) -> Self {
        Self { x: 0, y: 0, width, height }
    }
}

/// The shared texture the header describes. Every field is 0 before the core created a texture. A
/// width of 0 means no texture exists. A dropped texture keeps its other fields, so the adapter LUID
/// is never zeroed under a viewer that read the old width.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureInfo {
    pub adapter_luid: AdapterLuid,
    pub width: u32,
    pub height: u32,
    /// The raw `DXGI_FORMAT` value.
    pub format: u32,
    /// The left eye's rectangle, then the right eye's.
    pub eyes: [EyeRect; 2],
}

const _: () = assert!(offset_of!(TextureInfo, adapter_luid) == 0);
const _: () = assert!(offset_of!(AdapterLuid, low) == 0);
const _: () = assert!(offset_of!(AdapterLuid, high) == 4);
const _: () = assert!(offset_of!(TextureInfo, width) == 8);
const _: () = assert!(offset_of!(TextureInfo, height) == 12);
const _: () = assert!(offset_of!(TextureInfo, format) == 16);
const _: () = assert!(offset_of!(TextureInfo, eyes) == 20);
const _: () = assert!(size_of::<EyeRect>() == 16);
