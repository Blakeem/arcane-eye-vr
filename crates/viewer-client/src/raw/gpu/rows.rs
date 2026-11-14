//! The pixel rows a read back keeps of a mapped texture.

use std::num::NonZeroU32;
use std::slice;

use windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE;

use super::{BYTES_PER_PIXEL, GpuError};

/// The bytes of every `step`-th pixel of every `step`-th row of a mapped `size` texture of 4-byte
/// pixels, starting at the top left pixel, each kept row cut to its kept pixels.
pub(super) fn kept_rows(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    size: (u32, u32),
    step: NonZeroU32,
) -> Result<Vec<u8>, GpuError> {
    let (width, height) = size;
    let size_error = || GpuError::FrameSize { width, height, row_pitch: mapped.RowPitch };
    let row_pitch = mapped.RowPitch as usize;
    let pixel_bytes = BYTES_PER_PIXEL as usize;
    let step = step.get() as usize;
    let Some(row_bytes) = (width as usize).checked_mul(pixel_bytes) else {
        return Err(size_error());
    };
    let kept_row_bytes = (width as usize).div_ceil(step).saturating_mul(pixel_bytes);
    let Some(total) = kept_row_bytes.checked_mul((height as usize).div_ceil(step)) else {
        return Err(size_error());
    };
    if row_pitch < row_bytes {
        return Err(size_error());
    }
    let base = mapped.pData.cast::<u8>().cast_const();
    let mut pixels = Vec::with_capacity(total);
    for row in (0..height as usize).step_by(step) {
        let Some(offset) = row.checked_mul(row_pitch) else {
            return Err(size_error());
        };
        // SAFETY: `Map` gave `height` rows `row_pitch` bytes apart, each holding at least
        // `row_bytes` bytes, which stay mapped until the caller unmaps them.
        let line = unsafe { slice::from_raw_parts(base.wrapping_add(offset), row_bytes) };
        if step == 1 {
            pixels.extend_from_slice(line);
        } else {
            for pixel in line.chunks_exact(pixel_bytes).step_by(step) {
                pixels.extend_from_slice(pixel);
            }
        }
    }
    Ok(pixels)
}
