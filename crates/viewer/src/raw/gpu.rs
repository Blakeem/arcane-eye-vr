//! The D3D11 and DXGI calls that create the shared texture's private copy and draw it letterboxed
//! into the window. `viewer-client` opens the shared texture and copies it into the private copy.

use ipc_layout::viewer_header::AdapterLuid;
use viewer_client::{Copied, Device, GpuError, SharedTexture};
use windows::Win32::Foundation::DXGI_STATUS_OCCLUDED;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0,
    D3D11_RTV_DIMENSION_TEXTURE2D, D3D11_TEX2D_RTV, D3D11_VIEWPORT, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_UNSPECIFIED, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_B8G8R8X8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGISwapChain1,
};
use windows::core::HSTRING;

use super::Window;
use crate::letterbox;

mod shader;

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// The device on the core's adapter, with the two shaders and the sampler that draw the image.
pub(crate) struct Renderer {
    device: Device,
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
}

impl Renderer {
    /// Creates a device on the adapter `adapter_luid` names and the pipeline that draws on it.
    pub(crate) fn create(adapter_luid: AdapterLuid) -> Result<Self, GpuError> {
        let device = Device::create(adapter_luid)?;
        let (vertex, pixel, sampler) = shader::pipeline(device.d3d())?;
        Ok(Self { device, vertex, pixel, sampler })
    }

    pub(crate) const fn adapter_luid(&self) -> AdapterLuid {
        self.device.adapter_luid()
    }

    /// Opens the shared texture `name` and creates the private copy the viewer draws from.
    pub(crate) fn open(&self, name: &HSTRING) -> Result<Source, GpuError> {
        let shared = self.device.open(name)?;
        let mut desc = shared.desc();
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned();
        desc.MiscFlags = 0;
        let (mut copy, mut view) = (None, None);
        let d3d = self.device.d3d();
        // SAFETY: the descriptor and the out slot are live for the call.
        let copy_made = unsafe { d3d.CreateTexture2D(&raw const desc, None, Some(&raw mut copy)) };
        GpuError::check("creating the private copy", copy_made)?;
        let copy = GpuError::created("creating the private copy", copy)?;
        // SAFETY: `copy` is a shader resource of this device, and the out slot is live.
        let view_made = unsafe { d3d.CreateShaderResourceView(&copy, None, Some(&raw mut view)) };
        GpuError::check("creating the private copy's view", view_made)?;
        let view = GpuError::created("creating the private copy's view", view)?;
        let screen_format = screen_format(desc.Format);
        Ok(Source { shared, copy, view, screen_format })
    }

    /// Copies the shared texture into the private copy under the keyed mutex.
    ///
    /// # Errors
    ///
    /// Returns [`GpuError`] when the keyed mutex was abandoned or a mutex call failed. The source
    /// then has to be opened again.
    pub(crate) fn copy(&self, source: &Source) -> Result<Copied, GpuError> {
        self.device.copy(&source.shared, &source.copy)
    }

    /// Clears `target`, a view of a `size` back buffer, then draws `source` letterboxed into it.
    /// Nothing stays bound, so the swapchain can resize.
    pub(crate) fn draw(&self, target: &ID3D11RenderTargetView, size: (u32, u32), source: &Source) {
        let context = self.device.context();
        // SAFETY: `target` is a view of this device.
        unsafe { context.ClearRenderTargetView(target, &BLACK) };
        let Some(rect) = letterbox::fit(source.size(), size) else {
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

/// The shared texture, and the private copy the viewer draws while the core writes the next frame.
pub(crate) struct Source {
    shared: SharedTexture,
    copy: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    screen_format: DXGI_FORMAT,
}

impl Source {
    pub(crate) const fn size(&self) -> (u32, u32) {
        self.shared.size()
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
        let (factory, d3d) = (renderer.device.factory(), renderer.device.d3d());
        // SAFETY: the device and the window are live, and a size of 0 takes the window's size.
        let swapchain = unsafe {
            factory.CreateSwapChainForHwnd(d3d, window.handle, &raw const desc, None, None)
        };
        let swapchain = GpuError::check("creating the swapchain", swapchain)?;
        Ok(Self { swapchain, target: None })
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
        GpuError::check("presenting", code.ok())?;
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
        GpuError::check("resizing the swapchain", resized)?;
        // SAFETY: the swapchain is live. The call adds the reference the result holds.
        let buffer = unsafe { self.swapchain.GetBuffer::<ID3D11Texture2D>(0) };
        let buffer = GpuError::check("taking the back buffer", buffer)?;
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
            renderer.device.d3d().CreateRenderTargetView(
                &buffer,
                Some(&raw const desc),
                Some(&raw mut view),
            )
        };
        GpuError::check("creating the back buffer view", made)?;
        let view = GpuError::created("creating the back buffer view", view)?;
        Ok(Target { view, size, format })
    }
}

#[cfg(test)]
mod tests;
