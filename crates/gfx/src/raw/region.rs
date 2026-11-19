//! The D3D11 calls of the region draw: its shaders, a source texture made from pixels, the views of
//! its source and its target, and the draw.
//!
//! The draw sets no rasterizer state, blend state or sampler. Direct3D's default rasterizer state
//! draws a clockwise triangle with no scissor test, its default blend state writes the shader's
//! output unblended, and the shader reads texels with `Load`, which takes no sampler.

use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0, D3D11_RTV_DIMENSION_TEXTURE2D,
    D3D11_TEX2D_RTV, D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT;
use windows::core::HRESULT;

use crate::raw::{CreateStep, made};
use crate::region::{RegionError, RegionStep};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/region.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/region.ps.dxbc"));

/// The device objects of the region draw.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    device: ID3D11Device,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, RegionError> {
        let (mut vertex, mut pixel) = (None, None);
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let vertex_made =
            unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut vertex)) };
        let vertex = made(RegionStep::VertexShader, vertex_made, vertex)?;
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let pixel_made =
            unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut pixel)) };
        let pixel = made(RegionStep::PixelShader, pixel_made, pixel)?;
        Ok(Self { vertex, pixel, device: device.clone() })
    }

    pub(crate) fn source_view(
        &self,
        source: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView, RegionError> {
        let mut view = None;
        // SAFETY: `source` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateShaderResourceView(source, None, Some(&raw mut view)) };
        made(RegionStep::SourceView, created, view)
    }

    pub(crate) fn image(
        &self,
        width: u16,
        height: u16,
        pixels: &[[u8; 4]],
    ) -> Result<ID3D11Texture2D, RegionError> {
        super::image(&self.device, RegionStep::Image, width, height, pixels)
    }

    pub(crate) fn target_view(
        &self,
        target: &ID3D11Texture2D,
        format: DXGI_FORMAT,
    ) -> Result<ID3D11RenderTargetView, RegionError> {
        let desc = D3D11_RENDER_TARGET_VIEW_DESC {
            Format: format,
            ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
            },
        };
        let mut view = None;
        // SAFETY: `target` is live, and the descriptor and the out slot are live for the call. The
        // device rejects a format outside the target's format family with an error code.
        let created = unsafe {
            self.device.CreateRenderTargetView(target, Some(&raw const desc), Some(&raw mut view))
        };
        made(RegionStep::TargetView, created, view)
    }

    /// Draws `source` into `target` over `viewport`, then unbinds both views.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11RenderTargetView,
        source: &ID3D11ShaderResourceView,
        viewport: D3D11_VIEWPORT,
    ) {
        // SAFETY: the slice holds one live view of this device.
        unsafe { context.OMSetRenderTargets(Some(&[Some(target.clone())]), None) };
        // SAFETY: an empty blend state selects the default, which writes unblended.
        unsafe { context.OMSetBlendState(None, None, u32::MAX) };
        // SAFETY: no state selects the default rasterizer state.
        unsafe { context.RSSetState(None) };
        // SAFETY: the slice holds one viewport and lives for the call.
        unsafe { context.RSSetViewports(Some(&[viewport])) };
        // SAFETY: the vertex shader takes no vertex input, so no layout is bound.
        unsafe { context.IASetInputLayout(None) };
        // SAFETY: the topology is a plain value.
        unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.VSSetShader(&self.vertex, None) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.PSSetShader(&self.pixel, None) };
        // SAFETY: the slice holds one live view, bound after the target, so it is no output.
        unsafe { context.PSSetShaderResources(0, Some(&[Some(source.clone())])) };
        // SAFETY: the bound vertex shader builds its three vertices from the vertex index.
        unsafe { context.Draw(3, 0) };
        // SAFETY: the slice holds one empty slot.
        unsafe { context.PSSetShaderResources(0, Some(&[None])) };
        // SAFETY: no view is passed.
        unsafe { context.OMSetRenderTargets(None, None) };
    }
}

impl CreateStep for RegionStep {
    type Error = RegionError;

    fn failed(self, code: HRESULT) -> RegionError {
        RegionError { step: self, code }
    }
}
