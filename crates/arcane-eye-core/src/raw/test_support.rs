//! Fixtures for the core's tests.
//!
//! It holds a launch token mapping, a WARP device and swapchain on a hidden window, a debug viewer's
//! view of the viewer header, a call of a proxy controller's slot 9 and pages with a read-only tail.

use std::ffi::c_void;
use std::{mem, process, ptr};

use game_mem::GameAddr;
use gfx::publish::ViewNames;
use ipc_layout::launch_token::{self, STATUS_OFFSET};
use ipc_layout::viewer_header::ViewerHeader;
use viewer_client::HeaderView;
use viewer_client::header::read_header;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, HWND, INVALID_HANDLE_VALUE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11CreateDeviceAndSwapChain,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_MODE_DESC, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SWAP_CHAIN_DESC, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISwapChain,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE, VirtualProtect,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
};
use windows::core::{HSTRING, Interface as _, w};

use super::{SharedPages, StatusView};

/// A zero-filled launch token mapping under a name only this test process uses, closed on drop.
pub(crate) struct TestMapping {
    pub(crate) name: String,
    mapping: HANDLE,
}

impl TestMapping {
    pub(crate) fn create(test: &str) -> Self {
        let name = format!(r"Local\arcane-eye-vr-core-test-{}-{test}", process::id());
        let size = u32::try_from(launch_token::SIZE).expect("64 fits a u32");
        let wide_name = HSTRING::from(&name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let mapping = unsafe {
            CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, &wide_name)
        }
        .expect("create the test mapping");
        Self { name, mapping }
    }

    /// The token's status field.
    pub(crate) fn status(&self) -> u32 {
        let opened = StatusView::open(&self.name).expect("map the test mapping");
        let view = opened.expect("the test mapping exists");
        let field =
            ptr::with_exposed_provenance::<u32>(view.address).wrapping_byte_add(STATUS_OFFSET);
        // SAFETY: the status field is a 4-aligned u32 inside the mapped 64-byte view.
        unsafe { ptr::read_volatile(field) }
    }
}

impl Drop for TestMapping {
    fn drop(&mut self) {
        // SAFETY: `mapping` is the handle `create` opened, closed once here.
        unsafe { CloseHandle(self.mapping) }.expect("close the test mapping");
    }
}

/// Viewer names that only this test process and test use, never the real viewer's.
pub(crate) fn test_view_names(test: &str) -> ViewNames {
    let prefix = format!(r"Local\arcane-eye-vr-core-test-{}-{test}", process::id());
    ViewNames { mapping: format!("{prefix}-view"), texture: format!("{prefix}-texture") }
}

/// A debug viewer's view of the viewer header, which writes the heartbeat as the viewer does.
pub(crate) struct TestViewer {
    mapping: String,
    view: Option<HeaderView>,
    heartbeat: u32,
}

impl TestViewer {
    /// Opens the header the core created under `names.mapping`.
    pub(crate) fn open(names: &ViewNames) -> Self {
        let mut view = None;
        read_header(&mut view, &names.mapping).expect("open the header the core created");
        Self { mapping: names.mapping.clone(), view, heartbeat: 0 }
    }

    pub(crate) fn beat(&mut self) {
        self.heartbeat = self.heartbeat.wrapping_add(1);
        let (view, _) =
            read_header(&mut self.view, &self.mapping).expect("a header the core wrote");
        view.write_heartbeat(self.heartbeat);
    }

    pub(crate) fn header(&mut self) -> ViewerHeader {
        let (_, header) =
            read_header(&mut self.view, &self.mapping).expect("a header the core wrote");
        header
    }
}

/// The back buffer format of every test swapchain.
pub(crate) const FORMAT: DXGI_FORMAT = DXGI_FORMAT_R8G8B8A8_UNORM;

/// A swapchain on a window that is never shown. The swapchain drops before its window.
pub(crate) struct TestSwapchain {
    pub(crate) swapchain: IDXGISwapChain,
    _window: Window,
}

struct Window(HWND);

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the window was created on this thread by `TestSwapchain::new`, and only this drop
        // destroys it.
        unsafe { DestroyWindow(self.0) }.expect("destroy the test window");
    }
}

impl TestSwapchain {
    pub(crate) fn new(width: u32, height: u32) -> Self {
        let size = |value| i32::try_from(value).expect("a test size fits an i32");
        // SAFETY: STATIC is a system window class, and the window has no parent, menu or create data.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("arcane-eye-core test"),
                WS_POPUP,
                0,
                0,
                size(width),
                size(height),
                None,
                None,
                None,
                None,
            )
        }
        .expect("create the hidden test window");
        let window = Window(window);
        let desc = DXGI_SWAP_CHAIN_DESC {
            BufferDesc: DXGI_MODE_DESC {
                Width: width,
                Height: height,
                Format: FORMAT,
                ..Default::default()
            },
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 1,
            OutputWindow: window.0,
            Windowed: true.into(),
            SwapEffect: DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };
        let mut swapchain = None;
        // SAFETY: `desc` and `swapchain` are live for the call, and every other output is skipped.
        unsafe {
            D3D11CreateDeviceAndSwapChain(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG::default(),
                None,
                D3D11_SDK_VERSION,
                Some(&raw const desc),
                Some(&raw mut swapchain),
                None,
                None,
                None,
            )
        }
        .expect("create a WARP device and swapchain");
        Self { swapchain: swapchain.expect("the created swapchain"), _window: window }
    }

    pub(crate) fn address(&self) -> usize {
        self.swapchain.as_raw().expose_provenance()
    }

    /// Resizes the back buffer. DXGI fails the call while any reference to the back buffer exists.
    pub(crate) fn resize(&self, width: u32, height: u32) -> windows::core::Result<()> {
        // SAFETY: the swapchain is live, and buffer count 0 and format unknown keep both as they are.
        unsafe {
            self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG::default(),
            )
        }
    }

    /// Runs `f` through the guarded borrow the Present call-site hook uses.
    pub(crate) fn with<R>(&self, f: impl FnOnce(usize, &IDXGISwapChain) -> R) -> Option<R> {
        // SAFETY: `self.swapchain` holds a reference, so the swapchain lives until `f` returns.
        unsafe { super::with_swapchain(self.address(), f) }
    }
}

/// Calls the controller vtable slot 9 function at `function` with `controller` in `rcx`, as the
/// engine calls it, and returns the address it returns. `function` must be the eye camera's proxy
/// slot 9, which the caller checks first.
pub(crate) fn call_world_translate(function: GameAddr, controller: GameAddr) -> GameAddr {
    let pointer = ptr::with_exposed_provenance::<()>(function.get());
    // SAFETY: the caller checked that `function` is the proxy's `extern "system" fn(usize) -> usize`.
    let slot_9 =
        unsafe { mem::transmute::<*const (), extern "system" fn(usize) -> usize>(pointer) };
    GameAddr::new(slot_9(controller.get()))
}

/// The x64 Windows page size.
pub(crate) const PAGE_SIZE: usize = 4096;

/// Read-write pages followed by one read-only page, so a write past the read-write pages faults.
pub(crate) struct ReadOnlyTail {
    pages: SharedPages,
    writable_len: usize,
}

impl ReadOnlyTail {
    /// The pages with `tail_words` at the start of the read-only page, written before it is
    /// protected.
    pub(crate) fn new(writable_pages: usize, tail_words: &[usize]) -> Self {
        let writable_len = writable_pages.checked_mul(PAGE_SIZE).expect("a test size");
        let len = writable_len.checked_add(PAGE_SIZE).expect("a test size");
        let pages = SharedPages::commit(len).expect("commit the test pages");
        let tail = pages.address().checked_add(writable_len).expect("a test address");
        game_mem::try_write_slice(tail, tail_words).expect("fill the tail page");
        let tail = ptr::with_exposed_provenance::<c_void>(tail.get());
        let mut old = PAGE_PROTECTION_FLAGS::default();
        // SAFETY: `tail` is the last page of the pages above, which only this value uses.
        unsafe { VirtualProtect(tail, PAGE_SIZE, PAGE_READONLY, &raw mut old) }
            .expect("protect the last test page");
        Self { pages, writable_len }
    }

    /// The first byte of the read-only page.
    pub(crate) fn read_only_start(&self) -> GameAddr {
        self.pages.address().checked_add(self.writable_len).expect("a test address")
    }
}
