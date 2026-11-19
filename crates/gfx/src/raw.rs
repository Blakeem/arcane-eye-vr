//! The gfx crate's unsafe edge: the viewer header mapping, the D3D11 and DXGI calls of the
//! publisher, the layer draw's calls in `layer`, the nearest read's calls in `nearest`, the pointer
//! read's calls in `pointer`, the calls both reads share in `row` and the region draw's calls in
//! `region`.

#![expect(
    unsafe_code,
    reason = "the viewer header mapping and the D3D11 and DXGI calls are Windows calls"
)]

use std::os::windows::io::{FromRawHandle as _, OwnedHandle};
use std::ptr;

use ipc_layout::viewer_header::{
    self, AdapterLuid, MUTEX_KEY, SHARED_ACCESS, SIZE, WORD_SIZE, WORDS,
};
use windows::Win32::Foundation::{E_POINTER, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_TIMEOUT};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIDevice, IDXGIKeyedMutex,
    IDXGIResource1,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::{HRESULT, HSTRING, Interface as _};

use crate::publish::{PublishError, SetupError, Step};

pub(crate) mod layer;
pub(crate) mod nearest;
pub(crate) mod pointer;
pub(crate) mod region;
pub(crate) mod row;

/// The header's size as `CreateFileMappingW` takes it.
const HEADER_MAPPING_SIZE: u32 = 128;
const _: () = assert!(HEADER_MAPPING_SIZE as usize == SIZE);

const _: () = assert!(SHARED_ACCESS == DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0);
const _: () = assert!(viewer_header::WAIT_TIMEOUT == WAIT_TIMEOUT.0);
const _: () = assert!(viewer_header::WAIT_ABANDONED == WAIT_ABANDONED.0);

/// A read-write view of the viewer header mapping this process created or opened. The view and the
/// mapping close on drop.
///
/// The address is a plain number, so a publisher in the core context stays `Send + Sync`.
pub(crate) struct HeaderView {
    address: usize,
    _mapping: OwnedHandle,
}

impl HeaderView {
    /// Creates the mapping `name`, or opens it when it exists.
    pub(crate) fn create(name: &str) -> Result<Self, SetupError> {
        let name = HSTRING::from(name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let created = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                HEADER_MAPPING_SIZE,
                &name,
            )
        };
        let handle = match created {
            Ok(handle) => handle,
            Err(source) => return Err(SetupError::Create(source)),
        };
        // SAFETY: `handle` is the mapping handle the create call opened, and only `mapping` closes it.
        let mapping = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `handle` stays open while `mapping` lives, and the view spans the header's bytes.
        let view = unsafe { MapViewOfFile(handle, access, 0, 0, SIZE) };
        if view.Value.is_null() {
            return Err(SetupError::Map(windows::core::Error::from_thread()));
        }
        Ok(Self { address: view.Value.expose_provenance(), _mapping: mapping })
    }

    /// Copies the header's words. Each read is volatile, since the viewer writes the heartbeat.
    pub(crate) fn read_words(&self) -> [u32; WORDS] {
        let base = ptr::with_exposed_provenance::<u32>(self.address);
        let mut words = [0; WORDS];
        for (index, word) in words.iter_mut().enumerate() {
            // SAFETY: `index` is below `WORDS`, and the page-aligned view spans the header's `SIZE`
            // bytes, so the word is in bounds and aligned.
            *word = unsafe { ptr::read_volatile(base.wrapping_add(index)) };
        }
        words
    }

    /// Writes each word at its byte offset, in the order given. Each write is volatile, since the
    /// viewer reads the header. An offset that names no word of the header is skipped.
    pub(crate) fn write_words(&self, words: impl IntoIterator<Item = (usize, u32)>) {
        let base = ptr::with_exposed_provenance_mut::<u32>(self.address);
        for (offset, word) in words {
            if offset >= SIZE || !offset.is_multiple_of(WORD_SIZE) {
                continue;
            }
            // SAFETY: `offset` is a multiple of 4 below `SIZE`, and the page-aligned view spans the
            // header's `SIZE` bytes, so the word is in bounds and aligned.
            unsafe { ptr::write_volatile(base.wrapping_byte_add(offset), word) };
        }
    }
}

impl Drop for HeaderView {
    fn drop(&mut self) {
        let view =
            MEMORY_MAPPED_VIEW_ADDRESS { Value: ptr::with_exposed_provenance_mut(self.address) };
        // A failed unmap leaves this one view mapped until the process exits.
        // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(view) });
    }
}

/// The properties a copy of a texture takes from it, in the shared texture and in the layer draw's
/// copy: its size and format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceKey {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: DXGI_FORMAT,
}

impl SourceKey {
    pub(crate) const fn of(desc: &D3D11_TEXTURE2D_DESC) -> Self {
        Self { width: desc.Width, height: desc.Height, format: desc.Format }
    }

    /// The description of a single-sampled texture of the key's size and format that a shader
    /// samples, with `misc_flags`.
    pub(crate) const fn copy_desc(self, misc_flags: u32) -> D3D11_TEXTURE2D_DESC {
        D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: self.format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned(),
            CPUAccessFlags: 0,
            MiscFlags: misc_flags,
        }
    }
}

/// Copies the first subresource of `source`, which `desc` describes, into `copy`, a texture of
/// `SourceKey::copy_desc` for `desc`. A multisampled source is resolved.
pub(crate) fn copy_first_subresource(
    context: &ID3D11DeviceContext,
    copy: &ID3D11Texture2D,
    source: &ID3D11Texture2D,
    desc: &D3D11_TEXTURE2D_DESC,
) {
    if desc.SampleDesc.Count > 1 {
        // SAFETY: both textures belong to the context's device, and `copy` is single-sampled with
        // the source's size and format.
        unsafe { context.ResolveSubresource(copy, 0, source, 0, desc.Format) };
    } else {
        // SAFETY: both textures belong to the context's device, and the first subresource of each
        // has the same size and format.
        unsafe { context.CopySubresourceRegion(copy, 0, 0, 0, 0, source, 0, None) };
    }
}

pub(crate) fn texture_desc(texture: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: `texture` is live, and `desc` is a writable descriptor.
    unsafe { texture.GetDesc(&raw mut desc) };
    desc
}

/// A named shared texture with a keyed mutex, which the viewer opens by name. Dropping it closes the
/// name's handle.
pub(crate) struct SharedTexture {
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    _name: OwnedHandle,
    device: ID3D11Device,
    key: SourceKey,
    adapter_luid: AdapterLuid,
}

impl SharedTexture {
    /// Creates a single-sampled texture of `key`'s size and format on `device` and shares it as `name`.
    pub(crate) fn create(
        device: &ID3D11Device,
        key: SourceKey,
        name: &HSTRING,
    ) -> Result<Self, PublishError> {
        let misc = D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX | D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
        let desc = key.copy_desc(misc.0.cast_unsigned());
        let mut texture = None;
        // SAFETY: `desc` and `texture` are live for the call, and no initial data is passed.
        let created =
            unsafe { device.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) };
        let texture = step(Step::CreateTexture, created).and_then(|()| {
            texture.ok_or(PublishError { step: Step::CreateTexture, code: E_POINTER })
        })?;
        let resource = step(Step::ShareTexture, texture.cast::<IDXGIResource1>())?;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call, and the handle gets
        // default security.
        let shared = unsafe { resource.CreateSharedHandle(None, SHARED_ACCESS, name) };
        let handle = step(Step::ShareTexture, shared)?;
        // SAFETY: `handle` is the handle `CreateSharedHandle` returned, and only `named` closes it.
        let named = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let mutex = step(Step::KeyedMutex, texture.cast::<IDXGIKeyedMutex>())?;
        let adapter_luid = adapter_luid(device)?;
        Ok(Self { texture, mutex, _name: named, device: device.clone(), key, adapter_luid })
    }

    /// Whether the texture belongs to `device` and copies a back buffer of `key`.
    pub(crate) fn fits(&self, device: &ID3D11Device, key: SourceKey) -> bool {
        self.key == key && self.device == *device
    }

    pub(crate) const fn adapter_luid(&self) -> AdapterLuid {
        self.adapter_luid
    }

    /// Calls `AcquireSync` with no wait through the raw vtable, since the `windows` crate turns the
    /// wait codes into `Ok(())`.
    pub(crate) fn acquire(&self) -> HRESULT {
        let acquire_sync = self.mutex.vtable().AcquireSync;
        // SAFETY: the vtable entry belongs to the live mutex, which is passed as its `this`.
        unsafe { acquire_sync(self.mutex.as_raw(), MUTEX_KEY, 0) }
    }

    /// Copies `source`, which `desc` describes, into the texture. A multisampled source is resolved.
    /// The caller holds the keyed mutex.
    pub(crate) fn copy_from(
        &self,
        context: &ID3D11DeviceContext,
        source: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
    ) {
        copy_first_subresource(context, &self.texture, source, desc);
    }

    pub(crate) fn release(&self) -> Result<(), HRESULT> {
        // SAFETY: the caller acquired the mutex with `MUTEX_KEY`.
        unsafe { self.mutex.ReleaseSync(MUTEX_KEY) }?;
        Ok(())
    }
}

fn adapter_luid(device: &ID3D11Device) -> Result<AdapterLuid, PublishError> {
    let dxgi_device = step(Step::Adapter, device.cast::<IDXGIDevice>())?;
    // SAFETY: `dxgi_device` is live. The call adds the reference the result holds.
    let adapter = step(Step::Adapter, unsafe { dxgi_device.GetAdapter() })?;
    // SAFETY: `adapter` is live, and `GetDesc` takes no reference.
    let desc = step(Step::Adapter, unsafe { adapter.GetDesc() })?;
    Ok(AdapterLuid { low: desc.AdapterLuid.LowPart, high: desc.AdapterLuid.HighPart })
}

/// Names the failed step of a `windows` call.
fn step<T>(step: Step, result: windows::core::Result<T>) -> Result<T, PublishError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(PublishError { step, code: error.code() }),
    }
}

/// A draw's or a read's step, which names its error of a failed create or map call.
pub(crate) trait CreateStep {
    type Error;

    fn failed(self, code: HRESULT) -> Self::Error;
}

/// The object a create call made, or the failed `step` with its code. A call that succeeds and
/// fills no object fails with `E_POINTER`.
pub(crate) fn made<S: CreateStep, T>(
    step: S,
    created: windows::core::Result<()>,
    object: Option<T>,
) -> Result<T, S::Error> {
    match (created, object) {
        (Ok(()), Some(object)) => Ok(object),
        (Ok(()), None) => Err(step.failed(E_POINTER)),
        (Err(error), _) => Err(step.failed(error.code())),
    }
}

#[cfg(test)]
mod tests;
