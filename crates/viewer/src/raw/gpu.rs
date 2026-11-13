//! The D3D11 and DXGI calls that open the shared texture and draw it letterboxed into the window.

use ipc_layout::viewer_header::{Acquire, AdapterLuid, MUTEX_KEY, SHARED_ACCESS};
use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, E_POINTER, HMODULE, LUID};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_FLAG, D3D11_RENDER_TARGET_VIEW_DESC,
    D3D11_RENDER_TARGET_VIEW_DESC_0, D3D11_RTV_DIMENSION_TEXTURE2D, D3D11_SDK_VERSION,
    D3D11_TEX2D_RTV, D3D11_TEXTURE2D_DESC, D3D11_VIEWPORT, D3D11CreateDevice, ID3D11Device,
    ID3D11Device1, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RenderTargetView,
    ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_B8G8R8X8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIAdapter, IDXGIFactory4, IDXGIKeyedMutex, IDXGISwapChain1,
};
use windows::core::{HRESULT, HSTRING, Interface as _};

use super::Window;
use crate::letterbox;

mod shader;

/// How long the viewer waits for the core to release the shared texture before it skips a copy.
const ACQUIRE_WAIT_MS: u32 = 2;

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Why a D3D11 or DXGI step failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GpuError {
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
}

fn step<T>(step: &'static str, result: windows::core::Result<T>) -> Result<T, GpuError> {
    match result {
        Ok(value) => Ok(value),
        Err(source) => Err(GpuError::Call { step, source }),
    }
}

/// The object a successful call wrote to its out slot, or an error for a call that wrote none.
fn created<T>(step: &'static str, slot: Option<T>) -> Result<T, GpuError> {
    match slot {
        Some(value) => Ok(value),
        None => Err(GpuError::Call { step, source: E_POINTER.into() }),
    }
}

/// Whether `AcquireSync` returning `code` gave the viewer the keyed mutex. `false` means the core
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

/// What [`Renderer::copy`] left in the private copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Copied {
    /// The shared texture's current frame.
    NewFrame,
    /// The previous frame, since the core held the keyed mutex past the wait.
    CoreBusy,
}

/// The device on the core's adapter, with the two shaders and the sampler that draw the image.
pub(crate) struct Renderer {
    factory: IDXGIFactory4,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    adapter_luid: AdapterLuid,
}

impl Renderer {
    /// Creates a device on the adapter `adapter_luid` names, since a texture shared by name opens
    /// only on the adapter that created it.
    pub(crate) fn create(adapter_luid: AdapterLuid) -> Result<Self, GpuError> {
        // SAFETY: the call takes no arguments and returns a new factory.
        let factory =
            step("creating the DXGI factory", unsafe { CreateDXGIFactory1::<IDXGIFactory4>() })?;
        let luid = LUID { LowPart: adapter_luid.low, HighPart: adapter_luid.high };
        // SAFETY: `factory` is live. The call adds the reference the result holds.
        let adapter = unsafe { factory.EnumAdapterByLuid::<IDXGIAdapter>(luid) };
        let adapter = step("finding the core's adapter", adapter)?;
        let (mut device, mut context) = (None, None);
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
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )
        };
        step("creating the D3D11 device", made)?;
        let device = created("creating the D3D11 device", device)?;
        let context = created("creating the D3D11 device", context)?;
        let (vertex, pixel, sampler) = shader::pipeline(&device)?;
        Ok(Self { factory, device, context, vertex, pixel, sampler, adapter_luid })
    }

    pub(crate) const fn adapter_luid(&self) -> AdapterLuid {
        self.adapter_luid
    }

    /// Opens the shared texture `name` and creates the private copy the viewer draws from.
    pub(crate) fn open(&self, name: &HSTRING) -> Result<Source, GpuError> {
        let device = step("reading ID3D11Device1", self.device.cast::<ID3D11Device1>())?;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let shared =
            unsafe { device.OpenSharedResourceByName::<_, ID3D11Texture2D>(name, SHARED_ACCESS) };
        let shared = step("opening the shared texture", shared)?;
        let mutex = step("reading the keyed mutex", shared.cast::<IDXGIKeyedMutex>())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: `shared` is live, and `desc` is a writable descriptor.
        unsafe { shared.GetDesc(&raw mut desc) };
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned();
        desc.MiscFlags = 0;
        let (mut copy, mut view) = (None, None);
        // SAFETY: the descriptor and the out slot are live for the call.
        let copy_made =
            unsafe { self.device.CreateTexture2D(&raw const desc, None, Some(&raw mut copy)) };
        step("creating the private copy", copy_made)?;
        let copy = created("creating the private copy", copy)?;
        // SAFETY: `copy` is a shader resource of this device, and the out slot is live.
        let view_made =
            unsafe { self.device.CreateShaderResourceView(&copy, None, Some(&raw mut view)) };
        step("creating the private copy's view", view_made)?;
        let view = created("creating the private copy's view", view)?;
        let (size, screen_format) = ((desc.Width, desc.Height), screen_format(desc.Format));
        Ok(Source { shared, mutex, copy, view, size, screen_format })
    }

    /// Copies the shared texture into the private copy under the keyed mutex.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError`] when the keyed mutex was abandoned or a mutex call failed. The source
    /// then has to be opened again.
    pub(crate) fn copy(&self, source: &Source) -> Result<Copied, GpuError> {
        let acquire_sync = source.mutex.vtable().AcquireSync;
        // SAFETY: the vtable entry belongs to the live mutex, which is passed as its `this`. The raw
        // call keeps the wait codes, which the `windows` crate turns into `Ok(())`.
        let code = unsafe { acquire_sync(source.mutex.as_raw(), MUTEX_KEY, ACQUIRE_WAIT_MS) };
        if !acquired(code)? {
            return Ok(Copied::CoreBusy);
        }
        // SAFETY: both textures belong to this device and have the same size and format.
        unsafe { self.context.CopyResource(&source.copy, &source.shared) };
        // SAFETY: the mutex was acquired above with `MUTEX_KEY`.
        let released = unsafe { source.mutex.ReleaseSync(MUTEX_KEY) };
        step("releasing the keyed mutex", released)?;
        Ok(Copied::NewFrame)
    }

    /// Clears `target`, a view of a `size` back buffer, then draws `source` letterboxed into it.
    /// Nothing stays bound, so the swapchain can resize.
    pub(crate) fn draw(&self, target: &ID3D11RenderTargetView, size: (u32, u32), source: &Source) {
        let context = &self.context;
        // SAFETY: `target` is a view of this device.
        unsafe { context.ClearRenderTargetView(target, &BLACK) };
        let Some(rect) = letterbox::fit(source.size, size) else {
            return;
        };
        let viewport = D3D11_VIEWPORT {
            TopLeftX: rect.x,
            TopLeftY: rect.y,
            Width: rect.width,
            Height: rect.height,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        // SAFETY: the view belongs to this device.
        unsafe { context.OMSetRenderTargets(Some(&[Some(target.clone())]), None) };
        // SAFETY: the viewport lies inside the target.
        unsafe { context.RSSetViewports(Some(&[viewport])) };
        // SAFETY: the vertex shader reads only SV_VertexID, so no input layout is bound.
        unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
        // SAFETY: the shader belongs to this device.
        unsafe { context.VSSetShader(&self.vertex, None) };
        // SAFETY: the shader belongs to this device.
        unsafe { context.PSSetShader(&self.pixel, None) };
        // SAFETY: the view belongs to this device, and slot 0 is the shader's `source`.
        unsafe { context.PSSetShaderResources(0, Some(&[Some(source.view.clone())])) };
        // SAFETY: the sampler belongs to this device, and slot 0 is the shader's `linear_clamp`.
        unsafe { context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())])) };
        // SAFETY: the pipeline above is complete, and three vertices make the one triangle.
        unsafe { context.Draw(3, 0) };
        // SAFETY: unbinding passes no objects.
        unsafe { context.OMSetRenderTargets(None, None) };
        // SAFETY: unbinding passes no objects.
        unsafe { context.PSSetShaderResources(0, Some(&[None])) };
    }
}

/// The shared texture, its keyed mutex, and the private copy the viewer draws while the core writes
/// the next frame.
pub(crate) struct Source {
    shared: ID3D11Texture2D,
    mutex: IDXGIKeyedMutex,
    copy: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    size: (u32, u32),
    screen_format: DXGI_FORMAT,
}

impl Source {
    pub(crate) const fn size(&self) -> (u32, u32) {
        self.size
    }
}

/// The back buffer view format that shows a `source` texture with its stored values. A shader reads
/// an sRGB texture as linear values, so an sRGB view encodes them again.
const fn screen_format(source: DXGI_FORMAT) -> DXGI_FORMAT {
    match source {
        DXGI_FORMAT_R8G8B8A8_UNORM_SRGB
        | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
        | DXGI_FORMAT_B8G8R8X8_UNORM_SRGB => DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
        _ => DXGI_FORMAT_B8G8R8A8_UNORM,
    }
}

/// The view of a back buffer and the size and format it was made for.
struct Target {
    view: ID3D11RenderTargetView,
    size: (u32, u32),
    format: DXGI_FORMAT,
}

/// The window's swapchain and the view of its current back buffer.
pub(crate) struct Screen {
    swapchain: IDXGISwapChain1,
    target: Option<Target>,
}

impl Screen {
    pub(crate) fn create(renderer: &Renderer, window: &Window) -> Result<Self, GpuError> {
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
            ..Default::default()
        };
        let (factory, device) = (&renderer.factory, &renderer.device);
        // SAFETY: the device and the window are live, and a size of 0 takes the window's size.
        let swapchain = unsafe {
            factory.CreateSwapChainForHwnd(device, window.handle, &raw const desc, None, None)
        };
        Ok(Self { swapchain: step("creating the swapchain", swapchain)?, target: None })
    }

    /// Draws `source` into a back buffer of `size` and presents it, waiting for the vertical blank.
    /// Returns `false` when nothing was shown, as for a minimized or covered window.
    pub(crate) fn present(
        &mut self,
        renderer: &Renderer,
        size: (u32, u32),
        source: &Source,
    ) -> Result<bool, GpuError> {
        if size.0 == 0 || size.1 == 0 {
            return Ok(false);
        }
        let format = source.screen_format;
        let target = match self.target.take() {
            Some(target) if target.size == size && target.format == format => target,
            stale => {
                // An unbound value would live until the end of the statement, past the resize.
                drop(stale);
                self.resize(renderer, size, format)?
            }
        };
        renderer.draw(&target.view, size, source);
        self.target = Some(target);
        // SAFETY: the swapchain is live, and no view of its back buffer is bound.
        let code = unsafe { self.swapchain.Present(1, DXGI_PRESENT(0)) };
        if code == DXGI_STATUS_OCCLUDED {
            return Ok(false);
        }
        step("presenting", code.ok())?;
        Ok(true)
    }

    /// Resizes the back buffers to `size` and returns a `format` view of the new back buffer. The
    /// caller dropped the old view, since a live view fails the resize.
    fn resize(
        &self,
        renderer: &Renderer,
        size: (u32, u32),
        format: DXGI_FORMAT,
    ) -> Result<Target, GpuError> {
        let flags = DXGI_SWAP_CHAIN_FLAG::default();
        // SAFETY: no view of a back buffer exists, and buffer count 0 and format unknown keep both
        // as they are.
        let resized =
            unsafe { self.swapchain.ResizeBuffers(0, size.0, size.1, DXGI_FORMAT_UNKNOWN, flags) };
        step("resizing the swapchain", resized)?;
        // SAFETY: the swapchain is live. The call adds the reference the result holds.
        let buffer = unsafe { self.swapchain.GetBuffer::<ID3D11Texture2D>(0) };
        let buffer = step("taking the back buffer", buffer)?;
        let desc = D3D11_RENDER_TARGET_VIEW_DESC {
            Format: format,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
            },
        };
        let mut view = None;
        // SAFETY: `buffer` is a render target of the renderer's device, a flip model back buffer
        // takes an sRGB view of its format, and the descriptor and the out slot are live.
        let made = unsafe {
            renderer.device.CreateRenderTargetView(
                &buffer,
                Some(&raw const desc),
                Some(&raw mut view),
            )
        };
        step("creating the back buffer view", made)?;
        let view = created("creating the back buffer view", view)?;
        Ok(Target { view, size, format })
    }
}

#[cfg(test)]
mod tests;
