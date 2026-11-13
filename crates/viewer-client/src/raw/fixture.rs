//! Test fixtures in the core's place: a viewer header mapping and a WARP device that shares a
//! texture. Each name holds the test process id, so parallel test processes never meet.

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::{process, ptr};

use ipc_layout::viewer_header::{
    self, AdapterLuid, HEARTBEAT_OFFSET, MUTEX_KEY, SHARED_ACCESS, ViewerHeader,
};
use windows::Win32::Foundation::{
    CloseHandle, E_INVALIDARG, E_POINTER, HANDLE, HMODULE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_SUBRESOURCE_DATA,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGIKeyedMutex, IDXGIResource1};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::Threading::INFINITE;
use windows::core::{HSTRING, Interface as _};

use super::gpu::BYTES_PER_PIXEL;

/// A LUID far above the small counters Windows hands out to adapters since boot.
pub const MISSING_LUID: AdapterLuid = AdapterLuid { low: u32::MAX, high: i32::MAX };

/// The byte offset of the layout version, the header's second word.
pub const LAYOUT_VERSION_OFFSET: usize = viewer_header::WORD_SIZE;

/// A mapping or texture name that only the test `test` of this process uses.
pub fn test_name(test: &str) -> String {
    format!(r"Local\arcane-eye-vr-viewer-test-{}-{test}", process::id())
}

/// A header mapping as the core creates it.
pub struct CoreMapping {
    pub name: String,
    handle: HANDLE,
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl CoreMapping {
    /// Creates the mapping [`test_name`] gives `test` and writes the core's words of `header`.
    ///
    /// # Errors
    ///
    /// Returns the error of a failed create or map call, and the `E_INVALIDARG` of
    /// [`Self::write_header`].
    pub fn create(test: &str, header: &ViewerHeader) -> windows::core::Result<Self> {
        let name = test_name(test);
        let Ok(size) = u32::try_from(viewer_header::SIZE) else {
            return Err(E_INVALIDARG.into());
        };
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size,
                &HSTRING::from(&name),
            )
        }?;
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `handle` is the open mapping above, and the view spans the header's bytes.
        let address = unsafe { MapViewOfFile(handle, access, 0, 0, viewer_header::SIZE) };
        let map_error = address.Value.is_null().then(windows::core::Error::from_thread);
        let mapping = Self { name, handle, address };
        if let Some(error) = map_error {
            return Err(error);
        }
        mapping.write_header(header)?;
        Ok(mapping)
    }

    /// Writes the core's words of `header`, as the core does when it shares a new texture.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALIDARG` when a word of `header` lies outside the mapping.
    pub fn write_header(&self, header: &ViewerHeader) -> windows::core::Result<()> {
        for (offset, word) in header.core_words() {
            self.write_word(offset, word)?;
        }
        Ok(())
    }

    /// Writes `word` at the byte offset `offset` of the header.
    ///
    /// # Errors
    ///
    /// Returns `E_INVALIDARG` when `offset` is not a word inside the header.
    pub fn write_word(&self, offset: usize, word: u32) -> windows::core::Result<()> {
        if !offset.is_multiple_of(viewer_header::WORD_SIZE) || offset >= viewer_header::SIZE {
            return Err(E_INVALIDARG.into());
        }
        let field = self.address.Value.wrapping_byte_add(offset).cast::<u32>();
        // SAFETY: `offset` is a 4-aligned word inside the page-aligned view, checked above.
        unsafe { ptr::write_volatile(field, word) };
        Ok(())
    }

    pub fn heartbeat(&self) -> u32 {
        let field = self.address.Value.wrapping_byte_add(HEARTBEAT_OFFSET).cast::<u32>();
        // SAFETY: the heartbeat is a 4-aligned word inside the page-aligned view of the header.
        unsafe { ptr::read_volatile(field) }
    }
}

impl Drop for CoreMapping {
    fn drop(&mut self) {
        // A failed unmap or close leaks into a test process that ends soon.
        if !self.address.Value.is_null() {
            // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
            drop(unsafe { UnmapViewOfFile(self.address) });
        }
        // SAFETY: `handle` is the handle `create` opened, closed once here.
        drop(unsafe { CloseHandle(self.handle) });
    }
}

/// The descriptor of a `width` by `height` texture of `format` with the `bind` and `misc` flags.
pub fn texture_desc(
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    bind: u32,
    misc: u32,
) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: misc,
    }
}

/// A texture a WARP device shares by name as the core does. The name and the texture live as long
/// as this value.
pub struct CoreTexture {
    pub luid: AdapterLuid,
    mutex: IDXGIKeyedMutex,
    _handle: OwnedHandle,
    _device: ID3D11Device,
}

impl CoreTexture {
    /// Acquires the keyed mutex and releases the texture, its name and its device while holding it,
    /// as a core that exits mid-copy does.
    ///
    /// # Errors
    ///
    /// Returns the error of a failed acquire.
    pub fn abandon(self) -> windows::core::Result<()> {
        // SAFETY: the mutex is live, and the key is the one both processes use.
        unsafe { self.mutex.AcquireSync(MUTEX_KEY, INFINITE) }
    }
}

/// Shares a `format` texture of `pixels` in row order, `width` pixels per row, under `name`.
///
/// # Errors
///
/// Returns `E_INVALIDARG` when `width` is 0 or does not divide the pixel count, and the error of a
/// failed device, texture or share call.
pub fn share(
    name: &HSTRING,
    width: u32,
    format: DXGI_FORMAT,
    pixels: &[[u8; 4]],
) -> windows::core::Result<CoreTexture> {
    let Ok(count) = u32::try_from(pixels.len()) else {
        return Err(E_INVALIDARG.into());
    };
    let (Some(height), Some(pitch)) =
        (count.checked_div(width), width.checked_mul(BYTES_PER_PIXEL))
    else {
        return Err(E_INVALIDARG.into());
    };
    if height.checked_mul(width) != Some(count) {
        return Err(E_INVALIDARG.into());
    }
    let device = warp_device()?;
    let misc = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
    let bind = D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned();
    let desc = texture_desc(width, height, format, bind, misc.0.cast_unsigned());
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr().cast(),
        SysMemPitch: pitch,
        SysMemSlicePitch: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    // SAFETY: `data` points at `height` rows of `width` pixels, which outlive the call.
    unsafe {
        device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }?;
    let texture = texture.ok_or_else(|| windows::core::Error::from(E_POINTER))?;
    let resource = texture.cast::<IDXGIResource1>()?;
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
    let handle = unsafe { resource.CreateSharedHandle(None, SHARED_ACCESS, name) }?;
    // SAFETY: `handle` is the handle `CreateSharedHandle` returned, owned only here.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
    let mutex = texture.cast::<IDXGIKeyedMutex>()?;
    let dxgi_device = device.cast::<IDXGIDevice>()?;
    // SAFETY: `dxgi_device` is live. The call adds the reference the result holds.
    let adapter = unsafe { dxgi_device.GetAdapter() }?;
    // SAFETY: `adapter` is live.
    let luid = unsafe { adapter.GetDesc() }?.AdapterLuid;
    let luid = AdapterLuid { low: luid.LowPart, high: luid.HighPart };
    Ok(CoreTexture { luid, mutex, _handle: handle, _device: device })
}

fn warp_device() -> windows::core::Result<ID3D11Device> {
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
    }?;
    device.ok_or_else(|| E_POINTER.into())
}
