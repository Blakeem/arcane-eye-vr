//! The renderer against a texture a WARP device shares in the core's place.

use std::process;

use ipc_layout::viewer_header::AdapterLuid;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_B8G8R8X8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
    DXGI_FORMAT_R10G10B10A2_UNORM,
};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_DEVICE_REMOVED;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
};
use windows::core::{HRESULT, HSTRING, Interface as _, w};

use super::{Copied, GpuError, Renderer, Screen, Window, acquired, screen_format};
use crate::raw::test_support::{share, texture_desc};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const OPAQUE_BLACK: [u8; 4] = [0, 0, 0, 255];

/// The bytes of `target`, a 4x4 RGBA texture of `renderer`'s device, in row order.
fn read(renderer: &Renderer, target: &ID3D11Texture2D) -> Vec<[u8; 4]> {
    let mut desc = texture_desc(4, 4, 0, 0);
    desc.Usage = D3D11_USAGE_STAGING;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
    let mut staging = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { renderer.device.CreateTexture2D(&raw const desc, None, Some(&raw mut staging)) }
        .expect("create the staging texture");
    let staging = staging.expect("the staging texture");
    // SAFETY: both textures belong to the renderer's device and have the same size and format.
    unsafe { renderer.context.CopyResource(&staging, target) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the staging texture has CPU read access, and `mapped` is writable.
    unsafe { renderer.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }
        .expect("map the staging texture");
    let mut pixels = Vec::new();
    for y in 0..4_usize {
        let start = y.checked_mul(mapped.RowPitch as usize).expect("a row offset");
        let row = mapped.pData.cast::<[u8; 4]>().wrapping_byte_add(start);
        // SAFETY: the mapping holds 4 rows of `RowPitch` bytes, each with 4 pixels of 4 bytes.
        pixels.extend_from_slice(unsafe { std::slice::from_raw_parts(row, 4) });
    }
    // SAFETY: the staging texture was mapped above.
    unsafe { renderer.context.Unmap(&staging, 0) };
    pixels
}

#[test]
fn renderer_opens_the_shared_texture_and_draws_it_letterboxed() {
    let name = HSTRING::from(format!(r"Local\arcane-eye-vr-viewer-test-{}-draw", process::id()));
    let (_core, _handle, luid) = share(&name, 2, &[RED, GREEN]);
    let renderer = Renderer::create(luid).expect("a device on the core's adapter");
    assert_eq!(renderer.adapter_luid(), luid);
    let source = renderer.open(&name).expect("open the shared texture by name");
    assert_eq!(source.size(), (2, 1));
    let copied = renderer.copy(&source).expect("the copy acquired the free keyed mutex");
    assert_eq!(copied, Copied::NewFrame);
    let target_desc = texture_desc(4, 4, D3D11_BIND_RENDER_TARGET.0.cast_unsigned(), 0);
    let mut target = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { renderer.device.CreateTexture2D(&raw const target_desc, None, Some(&raw mut target)) }
        .expect("create the target texture");
    let target = target.expect("the target texture");
    let mut view = None;
    // SAFETY: `target` is a render target of the renderer's device, and the out slot is live.
    unsafe { renderer.device.CreateRenderTargetView(&target, None, Some(&raw mut view)) }
        .expect("create the target view");
    renderer.draw(&view.expect("the target view"), (4, 4), &source);
    let pixels = read(&renderer, &target);
    let (rows, _rest) = pixels.as_chunks::<4>();
    let [top, upper, lower, bottom] = rows else {
        panic!("4 rows of 4 pixels, not {pixels:?}");
    };
    assert_eq!(*top, [OPAQUE_BLACK; 4], "the bar above the image");
    assert_eq!(*bottom, [OPAQUE_BLACK; 4], "the bar below the image");
    for [left, left_middle, right_middle, right] in [upper, lower] {
        assert_eq!((*left, *right), (RED, GREEN), "{upper:?}");
        let [left_red, left_green, ..] = *left_middle;
        assert!(left_red > left_green, "the left middle pixel leans red: {left_middle:?}");
        let [right_red, right_green, ..] = *right_middle;
        assert!(right_green > right_red, "the right middle pixel leans green: {right_middle:?}");
    }
}

#[test]
fn an_srgb_source_gets_an_srgb_back_buffer_view_and_every_other_format_a_unorm_view() {
    let srgb = [
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        DXGI_FORMAT_B8G8R8X8_UNORM_SRGB,
    ];
    for format in srgb {
        assert_eq!(screen_format(format), DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, "{format:?}");
    }
    let unorm =
        [DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R10G10B10A2_UNORM];
    for format in unorm {
        assert_eq!(screen_format(format), DXGI_FORMAT_B8G8R8A8_UNORM, "{format:?}");
    }
}

/// A LUID far above the small counters Windows hands out to adapters since boot.
const MISSING_LUID: AdapterLuid = AdapterLuid { low: u32::MAX, high: i32::MAX };

#[test]
fn renderer_on_an_unknown_adapter_names_the_failed_step() {
    let error = Renderer::create(MISSING_LUID).map(|_renderer| ()).expect_err("no such adapter");
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

/// A hidden window of the system `STATIC` class, which needs no registered class.
fn hidden_window() -> Window {
    // SAFETY: `STATIC` is a system class, and the window has no parent, menu or create data.
    let handle = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            64,
            64,
            None,
            None,
            None,
            None,
        )
    }
    .expect("create a hidden window");
    Window { handle }
}

/// The view the screen holds, after asserting the back buffers have `size` and the view `format`.
fn current_view(screen: &Screen, size: (u32, u32), format: DXGI_FORMAT) -> ID3D11RenderTargetView {
    // SAFETY: the swapchain is live.
    let desc = unsafe { screen.swapchain.GetDesc1() }.expect("the swapchain description");
    assert_eq!((desc.Width, desc.Height), size, "the back buffer size");
    let view = screen.target.as_ref().expect("a back buffer view").view.clone();
    let mut view_desc = D3D11_RENDER_TARGET_VIEW_DESC::default();
    // SAFETY: `view` is live, and `view_desc` is writable.
    unsafe { view.GetDesc(&raw mut view_desc) };
    assert_eq!(view_desc.Format, format, "the back buffer view's format");
    // SAFETY: `view` is live.
    let resource = unsafe { view.GetResource() }.expect("the view's resource");
    let buffer: ID3D11Texture2D = resource.cast().expect("the view targets a 2D texture");
    let mut buffer_desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: `buffer` is live, and `buffer_desc` is writable.
    unsafe { buffer.GetDesc(&raw mut buffer_desc) };
    assert_eq!((buffer_desc.Width, buffer_desc.Height), size, "the size of the view's buffer");
    view
}

#[test]
fn screen_resizes_its_back_buffers_for_a_new_size_and_a_new_view_format() {
    let name = HSTRING::from(format!(r"Local\arcane-eye-vr-viewer-test-{}-resize", process::id()));
    let (_core, _handle, luid) = share(&name, 2, &[RED, GREEN]);
    let renderer = Renderer::create(luid).expect("a device on the core's adapter");
    let source = renderer.open(&name).expect("open the shared texture by name");
    let window = hidden_window();
    let mut screen = Screen::create(&renderer, &window).expect("a swapchain for the window");
    let mut views = Vec::new();
    for size in [(8, 8), (16, 8), (16, 8)] {
        let presented = screen.present(&renderer, size, &source);
        assert!(presented.is_ok(), "{size:?}: {presented:?}");
        views.push(current_view(&screen, size, DXGI_FORMAT_B8G8R8A8_UNORM).as_raw());
    }
    // Windows may give a new view the address of the released one, so the first address proves
    // nothing. `current_view` checks the size of the buffer each view targets instead.
    let [_, large, same] = views[..] else {
        panic!("three views, not {views:?}");
    };
    assert_eq!(large, same, "the same size and format keep the view");
    let mut srgb = source;
    srgb.screen_format = DXGI_FORMAT_B8G8R8A8_UNORM_SRGB;
    let presented = screen.present(&renderer, (16, 8), &srgb);
    assert!(presented.is_ok(), "an sRGB view of the same size: {presented:?}");
    current_view(&screen, (16, 8), DXGI_FORMAT_B8G8R8A8_UNORM_SRGB);
}
