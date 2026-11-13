//! Fixtures for the viewer's tests: a header mapping and a WARP device that shares a texture, both
//! in the core's place.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::{process, ptr};

use ipc_layout::viewer_header::{self, AdapterLuid, HEARTBEAT_OFFSET, SHARED_ACCESS, ViewerHeader};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIResource1};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::{HSTRING, Interface as _};

/// A header mapping as the core creates it, under a name only this test process and test use.
pub(crate) struct CoreMapping {
    pub(crate) name: String,
    handle: HANDLE,
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl CoreMapping {
    pub(crate) fn create(test: &str, header: &ViewerHeader) -> Self {
        let name = format!(r"Local\arcane-eye-vr-viewer-test-{}-{test}", process::id());
        let size = u32::try_from(viewer_header::SIZE).expect("128 fits a u32");
        let wide_name = HSTRING::from(&name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let handle = unsafe {
            CreateFileMappingW(INVALID_HANDLE_VALUE, None, PAGE_READWRITE, 0, size, &wide_name)
        }
        .expect("create the test mapping");
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `handle` is the open mapping above, and the view spans the header's bytes.
        let address = unsafe { MapViewOfFile(handle, access, 0, 0, viewer_header::SIZE) };
        assert!(!address.Value.is_null(), "map the test mapping");
        let mapping = Self { name, handle, address };
        for (offset, word) in header.core_words() {
            mapping.write_word(offset, word);
        }
        mapping
    }

    /// Writes `word` at the byte offset `offset` of the header.
    pub(crate) fn write_word(&self, offset: usize, word: u32) {
        assert!(offset.is_multiple_of(4) && offset < viewer_header::SIZE, "offset {offset}");
        let field = self.address.Value.wrapping_byte_add(offset).cast::<u32>();
        // SAFETY: `offset` is 4-aligned and inside the page-aligned 128-byte view, asserted above.
        unsafe { ptr::write_volatile(field, word) };
    }

    pub(crate) fn heartbeat(&self) -> u32 {
        let field = self.address.Value.wrapping_byte_add(HEARTBEAT_OFFSET).cast::<u32>();
        // SAFETY: the heartbeat is a 4-aligned word inside the page-aligned view of the header.
        unsafe { ptr::read_volatile(field) }
    }
}

impl Drop for CoreMapping {
    fn drop(&mut self) {
        // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
        unsafe { UnmapViewOfFile(self.address) }.expect("unmap the test mapping");
        // SAFETY: `handle` is the handle `create` opened, closed once here.
        unsafe { CloseHandle(self.handle) }.expect("close the test mapping");
    }
}

/// The descriptor of a `width` by `height` RGBA texture with the `bind` and `misc` flags.
pub(crate) fn texture_desc(width: u32, height: u32, bind: u32, misc: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: misc,
    }
}

/// A WARP device that shares a texture of `pixels` in row order, `width` pixels per row, under
/// `name`, as the core does. The texture lives while the returned handle and device live.
pub(crate) fn share(
    name: &HSTRING,
    width: u32,
    pixels: &[[u8; 4]],
) -> (ID3D11Device, OwnedHandle, AdapterLuid) {
    let count = u32::try_from(pixels.len()).expect("a test pixel count");
    let height = count.checked_div(width).expect("a nonzero test width");
    let mut device = None;
    // SAFETY: the out slot is live for the call, and no feature level list is passed.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_WARP,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG::default(),
            None,
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            None,
        )
    }
    .expect("create the core's WARP device");
    let device = device.expect("the core's device");
    let misc = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
    let bind = D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned();
    let desc = texture_desc(width, height, bind, misc.0.cast_unsigned());
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: width.checked_mul(4).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let mut texture = None;
    // SAFETY: `data` points at `height` rows of `width` pixels, which outlive the call.
    unsafe {
        device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the shared texture");
    let resource = texture.expect("the shared texture").cast::<IDXGIResource1>().expect("resource");
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
    let handle = unsafe { resource.CreateSharedHandle(None, SHARED_ACCESS, name) }.expect("share");
    // SAFETY: `handle` is the handle `CreateSharedHandle` returned, owned only here.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let dxgi_device = device.cast::<IDXGIDevice>().expect("IDXGIDevice");
    // SAFETY: `dxgi_device` is live. The call adds the reference the result holds.
    let adapter = unsafe { dxgi_device.GetAdapter() }.expect("the core's adapter");
    // SAFETY: `adapter` is live.
    let luid = unsafe { adapter.GetDesc() }.expect("the adapter description").AdapterLuid;
    (device, handle, AdapterLuid { low: luid.LowPart, high: luid.HighPart })
}
