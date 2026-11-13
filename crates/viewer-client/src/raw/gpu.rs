//! The D3D11 and DXGI calls that create a device on the core's adapter, open the shared texture,
//! copy it under its keyed mutex and read a copy back to memory.

use std::slice;

use ipc_layout::viewer_header::{Acquire, AdapterLuid, MUTEX_KEY, SHARED_ACCESS};
use windows::Win32::Foundation::{E_POINTER, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_FLAG, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11Device1, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory4, IDXGIKeyedMutex,
};
use windows::core::{HRESULT, HSTRING, Interface as _};

/// How long a copy waits for the core to release the shared texture before it gives up on this try.
const ACQUIRE_WAIT_MS: u32 = 2;

/// The bytes of one pixel in every format a frame read supports.
pub(super) const BYTES_PER_PIXEL: u32 = 4;

/// Why a D3D11 or DXGI step failed.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter has the LUID the header names, as after the core's adapter was removed.
    #[error("finding the core's adapter failed: {0}")]
    NoAdapter(#[source] windows::core::Error),
    /// A failed call, named by what it was for.
    #[error("{step} failed: {source}")]
    Call {
        step: &'static str,
        #[source]
        source: windows::core::Error,
    },
    /// `AcquireSync` returned `WAIT_ABANDONED`, which leaves the shared texture inconsistent.
    #[error("the core exited while holding the shared texture's keyed mutex")]
    Abandoned,
    /// A copy's textures do not both belong to the copying device, or differ in size or format.
    #[error(
        "the copy target is not a texture of the copying device with the shared texture's size and format"
    )]
    TargetMismatch,
    /// The frame's rows overflow a memory size, or `Map` gave rows shorter than a frame row.
    #[error("a {width}x{height} frame does not fit mapped rows of {row_pitch} bytes")]
    FrameSize { width: u32, height: u32, row_pitch: u32 },
}

impl GpuError {
    /// The value of a call made for `step`, or the error that names the step.
    pub fn check<T>(step: &'static str, result: windows::core::Result<T>) -> Result<T, Self> {
        match result {
            Ok(value) => Ok(value),
            Err(source) => Err(Self::Call { step, source }),
        }
    }

    /// The object a successful call made for `step` wrote to its out slot, or an error for a call
    /// that wrote none.
    pub fn created<T>(step: &'static str, slot: Option<T>) -> Result<T, Self> {
        match slot {
            Some(value) => Ok(value),
            None => Err(Self::Call { step, source: E_POINTER.into() }),
        }
    }
}

/// Whether `AcquireSync` returning `code` gave this process the keyed mutex. `false` means the core
/// held it past the wait.
fn acquired(code: HRESULT) -> Result<bool, GpuError> {
    match Acquire::from_code(code.0) {
        Acquire::Held => Ok(true),
        Acquire::Busy => Ok(false),
        Acquire::Abandoned => Err(GpuError::Abandoned),
        Acquire::Failed(failed) => Err(GpuError::Call {
            step: "acquiring the keyed mutex",
            source: HRESULT(failed).into(),
        }),
    }
}

/// What [`Device::copy`] left in the target texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    /// The shared texture's current frame.
    NewFrame,
    /// The target's earlier content, since the core held the keyed mutex past the wait.
    CoreBusy,
}

/// A D3D11 device on the core's adapter, since a texture shared by name opens only on the adapter
/// that created it.
pub struct Device {
    factory: IDXGIFactory4,
    d3d: ID3D11Device,
    context: ID3D11DeviceContext,
    adapter_luid: AdapterLuid,
}

impl Device {
    /// Creates a device on the adapter `adapter_luid` names.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::NoAdapter`] when no adapter has that LUID, and [`GpuError::Call`] when a
    /// factory or device call fails.
    pub fn create(adapter_luid: AdapterLuid) -> Result<Self, GpuError> {
        // SAFETY: the call takes no arguments and returns a new factory.
        let factory = GpuError::check("creating the DXGI factory", unsafe {
            CreateDXGIFactory1::<IDXGIFactory4>()
        })?;
        let luid = LUID { LowPart: adapter_luid.low, HighPart: adapter_luid.high };
        // SAFETY: `factory` is live. The call adds the reference the result holds.
        let adapter = unsafe { factory.EnumAdapterByLuid::<IDXGIAdapter>(luid) };
        let adapter = match adapter {
            Ok(adapter) => adapter,
            Err(error) => return Err(GpuError::NoAdapter(error)),
        };
        let (mut d3d, mut context) = (None, None);
        // SAFETY: both out slots are live for the call, and an explicit adapter takes driver type
        // unknown.
        let made = unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG::default(),
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut d3d),
                None,
                Some(&raw mut context),
            )
        };
        GpuError::check("creating the D3D11 device", made)?;
        let d3d = GpuError::created("creating the D3D11 device", d3d)?;
        let context = GpuError::created("creating the D3D11 device", context)?;
        Ok(Self { factory, d3d, context, adapter_luid })
    }

    pub const fn adapter_luid(&self) -> AdapterLuid {
        self.adapter_luid
    }

    /// The factory that enumerated the core's adapter.
    pub const fn factory(&self) -> &IDXGIFactory4 {
        &self.factory
    }

    pub const fn d3d(&self) -> &ID3D11Device {
        &self.d3d
    }

    /// The device's immediate context, which only the thread that owns this device uses.
    pub const fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    /// Opens the shared texture `name` and its keyed mutex.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::Call`] when no texture has that name on this adapter or a call fails.
    pub fn open(&self, name: &HSTRING) -> Result<SharedTexture, GpuError> {
        let device = GpuError::check("reading ID3D11Device1", self.d3d.cast::<ID3D11Device1>())?;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let texture =
            unsafe { device.OpenSharedResourceByName::<_, ID3D11Texture2D>(name, SHARED_ACCESS) };
        let texture = GpuError::check("opening the shared texture", texture)?;
        let mutex = GpuError::check("reading the keyed mutex", texture.cast::<IDXGIKeyedMutex>())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `texture` is live, and `desc` is a writable descriptor.
        unsafe { texture.GetDesc(&raw mut desc) };
        Ok(SharedTexture { texture, mutex, desc })
    }

    /// Copies `shared` into `target`, a texture of this device with the same size and format, under
    /// the keyed mutex.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError::TargetMismatch`] when `shared` or `target` belongs to another device or
    /// `target` differs from `shared` in size or format. Returns another [`GpuError`] when the keyed
    /// mutex was abandoned or a mutex call failed. The shared texture then has to be opened again.
    pub fn copy(
        &self,
        shared: &SharedTexture,
        target: &ID3D11Texture2D,
    ) -> Result<Copied, GpuError> {
        let mut target_desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `target` is live, and `target_desc` is a writable descriptor.
        unsafe { target.GetDesc(&raw mut target_desc) };
        let same_shape = (target_desc.Width, target_desc.Height, target_desc.Format)
            == (shared.desc.Width, shared.desc.Height, shared.desc.Format);
        if !same_shape || !self.owns(&shared.texture) || !self.owns(target) {
            return Err(GpuError::TargetMismatch);
        }
        let acquire_sync = shared.mutex.vtable().AcquireSync;
        // SAFETY: the vtable entry belongs to the live mutex, which is passed as its `this`. The raw
        // call keeps the wait codes, which the `windows` crate turns into `Ok(())`.
        let code = unsafe { acquire_sync(shared.mutex.as_raw(), MUTEX_KEY, ACQUIRE_WAIT_MS) };
        if !acquired(code)? {
            return Ok(Copied::CoreBusy);
        }
        // SAFETY: both textures belong to this device and have the same size and format, checked
        // above.
        unsafe { self.context.CopyResource(target, &shared.texture) };
        // SAFETY: the mutex was acquired above with `MUTEX_KEY`.
        let released = unsafe { shared.mutex.ReleaseSync(MUTEX_KEY) };
        GpuError::check("releasing the keyed mutex", released)?;
        Ok(Copied::NewFrame)
    }

    fn owns(&self, texture: &ID3D11Texture2D) -> bool {
        // SAFETY: `texture` is live. The call adds the reference the result holds.
        let device = unsafe { texture.GetDevice() };
        device.is_ok_and(|device| device == self.d3d)
    }

    /// Creates a texture the CPU can read, with the size and format of `shared`.
    pub(crate) fn create_staging(&self, shared: &SharedTexture) -> Result<Staging, GpuError> {
        let mut desc = shared.desc;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
        desc.MiscFlags = 0;
        let mut texture = None;
        // SAFETY: the descriptor and the out slot are live for the call.
        let made =
            unsafe { self.d3d.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) };
        GpuError::check("creating the staging texture", made)?;
        let texture = GpuError::created("creating the staging texture", texture)?;
        Ok(Staging { texture, size: (desc.Width, desc.Height) })
    }

    /// Reads the first subresource of `staging` as rows of 4-byte pixels with no row padding. The
    /// call waits for the copy into `staging` to finish.
    pub(crate) fn read_back(&self, staging: &Staging) -> Result<Vec<u8>, GpuError> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture has CPU read access and is not mapped, and `mapped` is
        // writable.
        let map = unsafe {
            self.context.Map(&staging.texture, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))
        };
        GpuError::check("mapping the staging texture", map)?;
        let pixels = unpadded_rows(&mapped, staging.size);
        // SAFETY: the staging texture was mapped above, and no slice of its memory outlives this.
        unsafe { self.context.Unmap(&staging.texture, 0) };
        pixels
    }
}

/// The bytes of a mapped `size` texture of 4-byte pixels, each row cut to its pixels.
fn unpadded_rows(mapped: &D3D11_MAPPED_SUBRESOURCE, size: (u32, u32)) -> Result<Vec<u8>, GpuError> {
    let (width, height) = size;
    let size_error = || GpuError::FrameSize { width, height, row_pitch: mapped.RowPitch };
    let row_pitch = mapped.RowPitch as usize;
    let rows = height as usize;
    let Some(row_bytes) = (width as usize).checked_mul(BYTES_PER_PIXEL as usize) else {
        return Err(size_error());
    };
    let Some(total) = row_bytes.checked_mul(rows) else {
        return Err(size_error());
    };
    if row_pitch < row_bytes {
        return Err(size_error());
    }
    let base = mapped.pData.cast::<u8>().cast_const();
    let mut pixels = Vec::with_capacity(total);
    for row in 0..rows {
        let Some(offset) = row.checked_mul(row_pitch) else {
            return Err(size_error());
        };
        // SAFETY: `Map` gave `rows` rows `row_pitch` bytes apart, each holding at least `row_bytes`
        // bytes, which stay mapped until the caller unmaps them.
        pixels.extend_from_slice(unsafe {
            slice::from_raw_parts(base.wrapping_add(offset), row_bytes)
        });
    }
    Ok(pixels)
}

/// A texture shared by name, its keyed mutex and its descriptor.
pub struct SharedTexture {
    texture: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    desc: D3D11_TEXTURE2D_DESC,
}

impl SharedTexture {
    /// The descriptor the shared texture was created with.
    pub const fn desc(&self) -> D3D11_TEXTURE2D_DESC {
        self.desc
    }

    pub const fn size(&self) -> (u32, u32) {
        (self.desc.Width, self.desc.Height)
    }
}

/// A texture of the size and format of a shared texture that the CPU can read.
pub(crate) struct Staging {
    texture: ID3D11Texture2D,
    size: (u32, u32),
}

impl Staging {
    pub(crate) const fn texture(&self) -> &ID3D11Texture2D {
        &self.texture
    }
}

#[cfg(test)]
mod tests;
