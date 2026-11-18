//! The xr crate's unsafe edge: the runtime library load and the loader negotiation, the adapter
//! list, the session create and the swapchain images.
//!
//! The runtime's library is loaded straight from the path its manifest names, so no
//! `openxr_loader.dll` ships. It stays loaded until the process exits, since the entry points of
//! every instance point into it.

#![expect(
    unsafe_code,
    reason = "the runtime library load, the OpenXR session create and the DXGI and D3D11 calls are foreign calls"
)]

use std::ffi::c_void;
use std::mem;
use std::path::Path;

use openxr::sys::{self, NegotiateLoaderInfo, NegotiateRuntimeRequest, pfn};
use openxr::{D3D11, Entry, FrameStream, FrameWaiter, Instance, Session, SystemId};
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIFactory1};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::core::{HSTRING, Interface as _, s};

use crate::runtime::LoadError;

/// The loader interface version this crate speaks, the only one the OpenXR loader specification
/// defines.
const LOADER_INTERFACE: u32 = 1;

/// The oldest OpenXR version the instance asks for.
pub(crate) const API_VERSION: openxr::Version = openxr::Version::new(1, 0, 0);

/// Loads the runtime library at `library`, negotiates the loader interface with it and returns
/// the entry points it gives.
pub(crate) fn load_entry(library: &Path) -> Result<Entry, LoadError> {
    let name = HSTRING::from(library.as_os_str());
    // SAFETY: `name` is a NUL-terminated wide string that outlives the call. The library is the
    // runtime the active runtime's manifest names.
    let module = match unsafe { LoadLibraryW(&name) } {
        Ok(module) => module,
        Err(source) => return Err(LoadError::Library { path: library.to_owned(), source }),
    };
    // SAFETY: `module` is the library loaded above, which stays loaded, and the name is a static C
    // string.
    let Some(export) = (unsafe { GetProcAddress(module, s!("xrNegotiateLoaderRuntimeInterface")) })
    else {
        return Err(LoadError::NoNegotiation { path: library.to_owned() });
    };
    // SAFETY: the OpenXR loader specification gives the export this signature.
    let negotiate = unsafe {
        mem::transmute::<unsafe extern "system" fn() -> isize, pfn::NegotiateLoaderRuntimeInterface>(
            export,
        )
    };
    let get_instance_proc_addr = negotiate_with(negotiate)?;
    // SAFETY: the runtime gave the function in its negotiation, and the specification makes it
    // yield every core function.
    match unsafe { Entry::from_get_instance_proc_addr(get_instance_proc_addr, &()) } {
        Ok(entry) => Ok(entry),
        Err(code) => Err(LoadError::Entry(code)),
    }
}

/// Runs the runtime's side of the loader negotiation and returns its `xrGetInstanceProcAddr`.
fn negotiate_with(
    negotiate: pfn::NegotiateLoaderRuntimeInterface,
) -> Result<pfn::GetInstanceProcAddr, LoadError> {
    let info = NegotiateLoaderInfo {
        struct_type: NegotiateLoaderInfo::TYPE,
        struct_version: NegotiateLoaderInfo::VERSION,
        struct_size: size_of::<NegotiateLoaderInfo>(),
        min_interface_version: LOADER_INTERFACE,
        max_interface_version: LOADER_INTERFACE,
        min_api_version: API_VERSION,
        max_api_version: sys::CURRENT_API_VERSION,
    };
    let mut request = NegotiateRuntimeRequest {
        struct_type: NegotiateRuntimeRequest::TYPE,
        struct_version: NegotiateRuntimeRequest::VERSION,
        struct_size: size_of::<NegotiateRuntimeRequest>(),
        runtime_interface_version: 0,
        runtime_api_version: openxr::Version::new(0, 0, 0),
        get_instance_proc_addr: None,
    };
    // SAFETY: both structures are initialized as the loader specification describes and live for
    // the call.
    let result = unsafe { negotiate(&raw const info, &raw mut request) };
    if result != sys::Result::SUCCESS {
        return Err(LoadError::Negotiation(result));
    }
    request.get_instance_proc_addr.ok_or(LoadError::Negotiation(sys::Result::ERROR_RUNTIME_FAILURE))
}

/// The LUID of every adapter, in DXGI's order.
pub(crate) fn adapter_luids() -> windows::core::Result<Vec<LUID>> {
    // SAFETY: the call takes no arguments and returns a new factory.
    let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }?;
    let mut luids = Vec::new();
    for index in 0_u32.. {
        // SAFETY: `factory` is live. The call adds the reference the result holds.
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(error),
        };
        // SAFETY: `adapter` is live.
        luids.push(unsafe { adapter.GetDesc1() }?.AdapterLuid);
    }
    Ok(luids)
}

/// The feature level `device` was created with, as the runtime's requirements state it.
pub(crate) fn feature_level(device: &ID3D11Device) -> i32 {
    // SAFETY: `device` is live, and the call only reads it.
    unsafe { device.GetFeatureLevel() }.0
}

/// Creates a session of `system` that draws with `device`.
pub(crate) fn create_session(
    instance: &Instance,
    system: SystemId,
    device: &ID3D11Device,
) -> openxr::Result<(Session<D3D11>, FrameWaiter, FrameStream<D3D11>)> {
    let info = openxr::d3d::SessionCreateInfoD3D11 { device: device.as_raw() };
    // SAFETY: `device` is a live D3D11 device, the caller read the D3D11 requirements first and
    // checked the device's feature level against them, and the caller keeps a reference to the
    // device for as long as the session lives.
    unsafe { instance.create_session::<D3D11>(system, &info) }
}

/// The textures of a D3D11 swapchain's `images`, each with a reference of its own, or `None` when
/// the runtime gave a null image.
pub(crate) fn textures(images: &[*mut c_void]) -> Option<Vec<ID3D11Texture2D>> {
    images
        .iter()
        .map(|image| {
            // SAFETY: the runtime gave `image` as a texture of its swapchain, which lives while
            // the swapchain does. The clone adds the reference the result holds.
            unsafe { ID3D11Texture2D::from_raw_borrowed(image) }.cloned()
        })
        .collect()
}
