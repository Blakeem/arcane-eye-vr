//! Pixel fixtures for the core's tests: a texture with given pixels, a distance texture, the flush
//! of a test swapchain's context, and the pixels written into and read from its back buffer.

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32_FLOAT, DXGI_SAMPLE_DESC};

use super::test_support::{FORMAT, TestSwapchain};

/// One pixel of the test format: red, green, blue and alpha.
pub(crate) type Pixel = [u8; 4];

const BYTES_PER_PIXEL: usize = size_of::<Pixel>();

fn desc(width: u32, height: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned(),
        ..Default::default()
    }
}

fn row_len(width: u32) -> usize {
    (width as usize).checked_mul(BYTES_PER_PIXEL).expect("a test row length")
}

/// A shader resource texture of `test`'s device, `width` pixels wide, that holds `pixels` in row
/// order.
pub(crate) fn texture_of(test: &TestSwapchain, width: u32, pixels: &[Pixel]) -> ID3D11Texture2D {
    let device = super::device(&test.swapchain).expect("the swapchain's device");
    let rows = pixels.len().checked_div(width as usize).expect("a test width above 0");
    let bytes = pixels.concat();
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr().cast(),
        SysMemPitch: u32::try_from(row_len(width)).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let desc = desc(width, u32::try_from(rows).expect("a test height"));
    let mut texture = None;
    // SAFETY: `data` points at `rows` rows of one pitch each, which outlive the call.
    unsafe {
        device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the test texture");
    texture.expect("the test texture")
}

/// An `R32_FLOAT` shader resource texture of `test`'s device, `width` by `height` texels, that
/// holds `value` in every texel, as the game's distance texture holds a share of the far plane.
pub(crate) fn distances_of(
    test: &TestSwapchain,
    width: u32,
    height: u32,
    value: f32,
) -> ID3D11Texture2D {
    distance_rows(test, width, &vec![value; height as usize])
}

/// An `R32_FLOAT` shader resource texture of `test`'s device, `width` texels wide, with one row per
/// value of `rows` that holds that value in every texel.
pub(crate) fn distance_rows(test: &TestSwapchain, width: u32, rows: &[f32]) -> ID3D11Texture2D {
    let device = super::device(&test.swapchain).expect("the swapchain's device");
    let values: Vec<f32> =
        rows.iter().flat_map(|value| std::iter::repeat_n(*value, width as usize)).collect();
    let height = u32::try_from(rows.len()).expect("a test height");
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: values.as_ptr().cast(),
        SysMemPitch: u32::try_from(row_len(width)).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let desc = D3D11_TEXTURE2D_DESC { Format: DXGI_FORMAT_R32_FLOAT, ..desc(width, height) };
    let mut texture = None;
    // SAFETY: `data` points at `height` rows of one pitch of four-byte values each, which outlive
    // the call.
    unsafe {
        device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the test distance texture");
    texture.expect("the test distance texture")
}

/// Sends the commands of `test`'s immediate context to its device, so a read the core started can
/// finish.
pub(crate) fn flush(test: &TestSwapchain) {
    let device = super::device(&test.swapchain).expect("the swapchain's device");
    let context = super::immediate_context(&device).expect("the immediate context");
    // SAFETY: the context is live.
    unsafe { context.Flush() };
}

/// Copies `pixels`, in row order, into `test`'s back buffer, as the engine fills it before the
/// Present call site.
pub(crate) fn fill_back_buffer(test: &TestSwapchain, pixels: &[Pixel]) {
    let (size, _) = super::swapchain_desc(&test.swapchain).expect("the swapchain's description");
    let back_buffer = super::back_buffer_texture(&test.swapchain).expect("the back buffer");
    fill_texture(test, &back_buffer, size.width, pixels);
}

/// Copies `pixels`, in row order, into `texture`, a texture of `test`'s device in the test format
/// `width` pixels wide that the pixels fill.
pub(crate) fn fill_texture(
    test: &TestSwapchain,
    texture: &ID3D11Texture2D,
    width: u32,
    pixels: &[Pixel],
) {
    let device = super::device(&test.swapchain).expect("the swapchain's device");
    let context = super::immediate_context(&device).expect("the immediate context");
    let picture = texture_of(test, width, pixels);
    // SAFETY: both textures belong to this device and have the same size and format.
    unsafe { context.CopyResource(texture, &picture) };
}

/// The pixels of `test`'s back buffer in row order.
pub(crate) fn back_buffer_pixels(test: &TestSwapchain) -> Vec<Pixel> {
    let device = super::device(&test.swapchain).expect("the swapchain's device");
    let context = super::immediate_context(&device).expect("the immediate context");
    let (size, _) = super::swapchain_desc(&test.swapchain).expect("the swapchain's description");
    let back_buffer = super::back_buffer_texture(&test.swapchain).expect("the back buffer");
    let mut staging_desc = desc(size.width, size.height);
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
    let mut staging = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { device.CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging)) }
        .expect("create the staging texture");
    let staging = staging.expect("the staging texture");
    // SAFETY: both textures belong to this device and have the same size and format.
    unsafe { context.CopyResource(&staging, &back_buffer) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the staging texture has CPU read access, and `mapped` is writable.
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }.expect("map");
    let mut pixels = Vec::new();
    for y in 0..size.height as usize {
        let start = y.checked_mul(mapped.RowPitch as usize).expect("a row offset");
        let line = mapped.pData.cast::<u8>().wrapping_add(start);
        // SAFETY: the mapped subresource holds `height` rows of `RowPitch` bytes each, and a row
        // holds `width` pixels of four bytes.
        let row = unsafe { std::slice::from_raw_parts(line, row_len(size.width)) };
        pixels.extend_from_slice(row.as_chunks::<BYTES_PER_PIXEL>().0);
    }
    // SAFETY: the staging texture was mapped above.
    unsafe { context.Unmap(&staging, 0) };
    pixels
}
