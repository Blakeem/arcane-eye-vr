//! The D3D11 calls of the nearest read: its shaders, its row texture of the grid columns, the
//! staging textures the CPU maps, the views of the layer and the distance texture, and the draw.
//!
//! The draw binds Direct3D's default rasterizer state, which draws a clockwise triangle with no
//! scissor test, and its default blend state, which writes the shader's output unblended. The shader
//! reads texels with `Load`, which takes no sampler. The context state the draw changes is saved
//! and restored as the layer draw's is.

use std::slice;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader,
    ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32_FLOAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;
use windows::core::{HRESULT, Interface as _};

use super::layer::saved::Saved;
use super::{CreateStep, made};
use crate::nearest::{NearestError, NearestStep};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nearest.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nearest.ps.dxbc"));

/// The device objects of the nearest read.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    device: ID3D11Device,
}

/// The row texture the draw writes one value per grid column into, and its view to draw into.
pub(crate) struct Columns {
    texture: ID3D11Texture2D,
    view: ID3D11RenderTargetView,
    width: u16,
}

/// A staging texture that receives a copy of the row texture for the CPU to map.
pub(crate) struct Staging {
    texture: ID3D11Texture2D,
    width: u16,
}

impl Pipeline {
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, NearestError> {
        let (mut vertex, mut pixel) = (None, None);
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let vertex_made =
            unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut vertex)) };
        let vertex = made(NearestStep::VertexShader, vertex_made, vertex)?;
        // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
        let pixel_made =
            unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut pixel)) };
        let pixel = made(NearestStep::PixelShader, pixel_made, pixel)?;
        Ok(Self { vertex, pixel, device: device.clone() })
    }

    pub(crate) fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.device == *device
    }

    /// A view of `texture` to sample, in its own format, or the failure of `step`.
    pub(crate) fn sampled_view(
        &self,
        step: NearestStep,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView, NearestError> {
        let mut view = None;
        // SAFETY: `texture` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateShaderResourceView(texture, None, Some(&raw mut view)) };
        made(step, created, view)
    }

    /// A row texture of `width` grid columns and its view to draw into.
    pub(crate) fn columns(&self, width: u32) -> Result<Columns, NearestError> {
        let (desc, width) = row_desc(NearestStep::Columns, width, D3D11_USAGE_DEFAULT)?;
        let texture = self.texture(NearestStep::Columns, &desc)?;
        let mut view = None;
        // SAFETY: `texture` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateRenderTargetView(&texture, None, Some(&raw mut view)) };
        let view = made(NearestStep::ColumnsView, created, view)?;
        Ok(Columns { texture, view, width })
    }

    /// A staging texture of `width` grid columns that the CPU can read.
    pub(crate) fn staging(&self, width: u32) -> Result<Staging, NearestError> {
        let (desc, width) = row_desc(NearestStep::Staging, width, D3D11_USAGE_STAGING)?;
        Ok(Staging { texture: self.texture(NearestStep::Staging, &desc)?, width })
    }

    fn texture(
        &self,
        step: NearestStep,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<ID3D11Texture2D, NearestError> {
        let mut texture = None;
        // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed.
        let created = unsafe { self.device.CreateTexture2D(desc, None, Some(&raw mut texture)) };
        made(step, created, texture)
    }

    /// Draws one value per grid column of `layer` over `distances` into `columns`, and leaves the
    /// state of `context` as it found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        columns: &Columns,
        layer: &ID3D11ShaderResourceView,
        distances: &ID3D11ShaderResourceView,
    ) {
        let saved = Saved::take(context);
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: f32::from(columns.width),
            Height: 1.0,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        // SAFETY: the slice holds one live view of this device, and no unordered-access view is
        // passed.
        unsafe {
            context.OMSetRenderTargetsAndUnorderedAccessViews(
                Some(&[Some(columns.view.clone())]),
                None,
                1,
                0,
                None,
                None,
            );
        }
        // SAFETY: an empty blend state selects the default, which writes unblended.
        unsafe { context.OMSetBlendState(None, None, u32::MAX) };
        // SAFETY: no state selects the default rasterizer state.
        unsafe { context.RSSetState(None) };
        // SAFETY: the slice holds one viewport and lives for the call.
        unsafe { context.RSSetViewports(Some(&[viewport])) };
        self.bind_stages(context, layer, distances);
        // SAFETY: the bound vertex shader builds its three vertices from the vertex index.
        unsafe { context.Draw(3, 0) };
        saved.restore(context);
    }

    /// Binds the shaders and the two views, after the target, so neither view is an output.
    fn bind_stages(
        &self,
        context: &ID3D11DeviceContext,
        layer: &ID3D11ShaderResourceView,
        distances: &ID3D11ShaderResourceView,
    ) {
        // SAFETY: the vertex shader takes no vertex input, so no layout is bound.
        unsafe { context.IASetInputLayout(None) };
        // SAFETY: the topology is a plain value.
        unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.VSSetShader(&self.vertex, None) };
        // SAFETY: the shader belongs to the context's device.
        unsafe { context.PSSetShader(&self.pixel, None) };
        // SAFETY: an unbound stage takes no argument.
        unsafe { context.GSSetShader(None, None) };
        // SAFETY: an unbound stage takes no argument.
        unsafe { context.HSSetShader(None, None) };
        // SAFETY: an unbound stage takes no argument.
        unsafe { context.DSSetShader(None, None) };
        let views = [Some(layer.clone()), Some(distances.clone())];
        // SAFETY: the slice holds two live views of this device.
        unsafe { context.PSSetShaderResources(0, Some(&views)) };
    }
}

impl Columns {
    pub(crate) fn width(&self) -> u32 {
        u32::from(self.width)
    }
}

impl Staging {
    pub(crate) fn width(&self) -> u32 {
        u32::from(self.width)
    }

    /// Copies `columns`, a row texture of the same width, into the staging texture.
    pub(crate) fn fill(&self, context: &ID3D11DeviceContext, columns: &Columns) {
        // SAFETY: both textures belong to the context's device and share the width, the one row and
        // the format.
        unsafe { context.CopyResource(&self.texture, &columns.texture) };
    }

    /// Maps the staging texture with no wait and gives its row's values to `visit`. Returns `None`
    /// while the GPU has not finished the copy into it. Calls `Map` through the raw vtable, since
    /// the `windows` crate turns the expected `DXGI_ERROR_WAS_STILL_DRAWING` into an error that
    /// takes the thread's COM error object.
    pub(crate) fn read<R>(
        &self,
        context: &ID3D11DeviceContext,
        visit: impl FnOnce(&[f32]) -> R,
    ) -> Result<Option<R>, NearestError> {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let map = context.vtable().Map;
        let flags = D3D11_MAP_FLAG_DO_NOT_WAIT.0.cast_unsigned();
        // SAFETY: the vtable entry belongs to the live context, which is passed as its `this`, the
        // staging texture is a live resource of its device with CPU read access, and `mapped` is
        // writable for the call.
        let code = unsafe {
            map(context.as_raw(), self.texture.as_raw(), 0, D3D11_MAP_READ, flags, &raw mut mapped)
        };
        if code == DXGI_ERROR_WAS_STILL_DRAWING {
            return Ok(None);
        }
        if code.is_err() {
            return Err(NearestError { step: NearestStep::Map, code });
        }
        let len = usize::from(self.width);
        // SAFETY: the mapped row holds `width` four-byte values, and `Map` aligns its data to at
        // least 4 bytes, the alignment of an f32, on every feature level.
        let values = unsafe { slice::from_raw_parts(mapped.pData.cast::<f32>(), len) };
        let result = visit(values);
        // SAFETY: the staging texture was mapped above, and `visit` returned, so no reference to
        // `values` is left.
        unsafe { context.Unmap(&self.texture, 0) };
        Ok(Some(result))
    }
}

/// The description of a row texture of `width` `R32_FLOAT` texels for `usage`, and the width as a
/// viewport side takes it. A width above 16-bit fails `step` with `E_INVALIDARG`.
fn row_desc(
    step: NearestStep,
    width: u32,
    usage: D3D11_USAGE,
) -> Result<(D3D11_TEXTURE2D_DESC, u16), NearestError> {
    let short = u16::try_from(width).ok().filter(|short| *short != 0);
    let short = short.ok_or(NearestError { step, code: E_INVALIDARG })?;
    let staging = usage == D3D11_USAGE_STAGING;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: 1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R32_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: if staging { 0 } else { D3D11_BIND_RENDER_TARGET.0.cast_unsigned() },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0.cast_unsigned() } else { 0 },
        MiscFlags: 0,
    };
    Ok((desc, short))
}

impl CreateStep for NearestStep {
    type Error = NearestError;

    fn failed(self, code: HRESULT) -> NearestError {
        NearestError { step: self, code }
    }
}
