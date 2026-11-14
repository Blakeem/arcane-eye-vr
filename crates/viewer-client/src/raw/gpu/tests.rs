//! The device, the copy's texture checks and the keyed mutex decisions, and the row copy against
//! mapped bytes written by hand.

use std::num::NonZeroU32;

use windows::Win32::Foundation::S_OK;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_MAPPED_SUBRESOURCE, ID3D11Device, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_DEVICE_REMOVED;
use windows::core::{HRESULT, HSTRING};

use super::rows::kept_rows;
use super::{Copied, Device, GpuError, acquired};
use crate::fixture::{CoreTexture, MISSING_LUID, share, test_name, texture_desc};
use crate::region::Region;

#[test]
fn device_on_an_unknown_adapter_is_no_adapter_and_names_the_failed_step() {
    let error = Device::create(MISSING_LUID).map(|_device| ()).expect_err("no such adapter");
    assert!(matches!(error, GpuError::NoAdapter(_)), "{error:?}");
    assert!(error.to_string().starts_with("finding the core's adapter failed: "), "{error}");
}

#[test]
fn each_acquire_code_maps_to_its_copy_decision() {
    assert!(matches!(acquired(S_OK), Ok(true)));
    assert!(matches!(acquired(HRESULT(0x102)), Ok(false)), "WAIT_TIMEOUT");
    assert!(matches!(acquired(HRESULT(0x80)), Err(GpuError::Abandoned)), "WAIT_ABANDONED");
    let removed = acquired(DXGI_ERROR_DEVICE_REMOVED).map(|_held| ()).expect_err("removed");
    let GpuError::Call { step, source } = removed else {
        panic!("a failed call, not {removed:?}");
    };
    assert_eq!((step, source.code()), ("acquiring the keyed mutex", DXGI_ERROR_DEVICE_REMOVED));
}

/// Shares a 2x1 RGBA texture under the name [`test_name`] gives `test`, and returns the core's
/// texture, which must outlive every use of the name, and the name.
fn share_pair(test: &str) -> (CoreTexture, HSTRING) {
    let name = HSTRING::from(test_name(test));
    let core = share(&name, 2, DXGI_FORMAT_R8G8B8A8_UNORM, &[[0; 4]; 2]).expect("share a texture");
    (core, name)
}

/// An empty one-row texture of `d3d` with `width` pixels of `format`.
fn empty_texture(d3d: &ID3D11Device, width: u32, format: DXGI_FORMAT) -> ID3D11Texture2D {
    let desc = texture_desc(width, 1, format, 0, 0);
    let mut texture = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { d3d.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) }
        .expect("create a texture");
    texture.expect("the texture")
}

#[test]
fn a_copy_into_a_texture_of_another_size_or_format_is_a_target_mismatch() {
    let (core, name) = share_pair("copy-shape");
    let device = Device::create(core.luid).expect("a device on the core's adapter");
    let shared = device.open(&name).expect("open the shared texture by name");
    for (width, format) in [(1, DXGI_FORMAT_R8G8B8A8_UNORM), (2, DXGI_FORMAT_B8G8R8A8_UNORM)] {
        let target = empty_texture(device.d3d(), width, format);
        let error = device.copy(&shared, &target).map(|_copied| ()).expect_err("no copy");
        assert!(matches!(error, GpuError::TargetMismatch), "{width} {format:?}: {error:?}");
    }
    let target = empty_texture(device.d3d(), 2, DXGI_FORMAT_R8G8B8A8_UNORM);
    let copied = device.copy(&shared, &target).expect("a target of the same size and format");
    assert_eq!(copied, Copied::NewFrame);
}

#[test]
fn a_copy_with_a_texture_of_another_device_is_a_target_mismatch() {
    let (core, name) = share_pair("copy-device");
    let device = Device::create(core.luid).expect("a device on the core's adapter");
    let other = Device::create(core.luid).expect("a second device on the core's adapter");
    let shared = device.open(&name).expect("open the shared texture on the device");
    let other_target = empty_texture(other.d3d(), 2, DXGI_FORMAT_R8G8B8A8_UNORM);
    let target_error = device.copy(&shared, &other_target).map(|_copied| ());
    let target_error = target_error.expect_err("no copy into another device's texture");
    assert!(matches!(target_error, GpuError::TargetMismatch), "{target_error:?}");
    let other_shared = other.open(&name).expect("open the shared texture on the other device");
    let target = empty_texture(device.d3d(), 2, DXGI_FORMAT_R8G8B8A8_UNORM);
    let shared_error = device.copy(&other_shared, &target).map(|_copied| ());
    let shared_error = shared_error.expect_err("no copy out of another device's texture");
    assert!(matches!(shared_error, GpuError::TargetMismatch), "{shared_error:?}");
}

/// A mapping of `bytes` with rows `row_pitch` bytes apart.
fn mapped(bytes: &mut [u8], row_pitch: u32) -> D3D11_MAPPED_SUBRESOURCE {
    D3D11_MAPPED_SUBRESOURCE {
        pData: bytes.as_mut_ptr().cast(),
        RowPitch: row_pitch,
        DepthPitch: 0,
    }
}

fn step(step: u32) -> NonZeroU32 {
    NonZeroU32::new(step).expect("a nonzero step")
}

#[test]
fn rows_lose_the_padding_after_their_pixels() {
    let mut bytes = [1, 2, 3, 4, 9, 9, 9, 9, 5, 6, 7, 8, 9, 9, 9, 9];
    let pixels = kept_rows(&mapped(&mut bytes, 8), (1, 2), step(1)).expect("two rows of one pixel");
    assert_eq!(pixels, [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn a_step_keeps_every_step_th_pixel_of_every_step_th_row_from_the_first() {
    // Three rows of three pixels with 4 bytes of padding, each pixel's bytes its row and column.
    let mut bytes: Vec<u8> = (0..3_u8)
        .flat_map(|row| (0..3_u8).flat_map(move |column| [row, column, 0, 0]).chain([9; 4]))
        .collect();
    let halved = kept_rows(&mapped(&mut bytes, 16), (3, 3), step(2)).expect("a 3x3 texture");
    assert_eq!(halved, [0, 0, 0, 0, 0, 2, 0, 0, 2, 0, 0, 0, 2, 2, 0, 0]);
    let corner = kept_rows(&mapped(&mut bytes, 16), (3, 3), step(3)).expect("a 3x3 texture");
    assert_eq!(corner, [0, 0, 0, 0]);
}

#[test]
fn a_region_copy_outside_the_shared_texture_or_into_another_size_is_a_target_mismatch() {
    let (core, name) = share_pair("copy-region");
    let device = Device::create(core.luid).expect("a device on the core's adapter");
    let shared = device.open(&name).expect("open the shared texture by name");
    let target = empty_texture(device.d3d(), 1, DXGI_FORMAT_R8G8B8A8_UNORM);
    for region in [
        Region { x: 2, y: 0, width: 1, height: 1 },
        Region { x: 0, y: 1, width: 1, height: 1 },
        Region { x: 0, y: 0, width: 2, height: 1 },
    ] {
        let copied = device.copy_region(&shared, &target, region, || ()).map(|_held| ());
        let error = copied.expect_err("no copy");
        assert!(matches!(error, GpuError::TargetMismatch), "{region}: {error:?}");
    }
    let region = Region { x: 1, y: 0, width: 1, height: 1 };
    let held = device.copy_region(&shared, &target, region, || 7).expect("a one-pixel region");
    assert_eq!(held, Some(7), "the copy ran its while-held step under the free keyed mutex");
}

#[test]
fn rows_shorter_than_a_frame_row_are_a_frame_size_error() {
    let mut bytes = [0; 8];
    let error =
        kept_rows(&mapped(&mut bytes, 4), (2, 1), step(1)).expect_err("a 4-byte row of 2 pixels");
    assert!(
        matches!(error, GpuError::FrameSize { width: 2, height: 1, row_pitch: 4 }),
        "{error:?}"
    );
}
