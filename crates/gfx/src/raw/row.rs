//! The D3D11 calls the GPU reads share: the row texture a pixel shader draws one texel per column
//! into, the staging textures the CPU maps with no wait, the views of the textures a read samples,
//! and the draw of one triangle over the row.
//!
//! The draw binds Direct3D's default rasterizer state, which draws a clockwise triangle with no
//! scissor test, and its default blend state, which writes the shader's output unblended. The
//! shaders read texels with `Load`, which takes no sampler. The context state the draw changes is
//! saved and restored as the layer draw's is.

use std::slice;

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext,
    ID3D11PixelShader, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
    ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32G32_FLOAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;
use windows::core::Interface as _;

use super::layer::saved::Saved;
use super::{CreateStep, made};

/// One `R32G32_FLOAT` texel of a row: the red channel, then the green channel.
pub(crate) type Texel = [f32; 2];

/// The row texture a read's pixel shader draws one [`Texel`] per column into, and its view to draw
/// into.
pub(crate) struct Row {
    texture: ID3D11Texture2D,
    view: ID3D11RenderTargetView,
    width: u16,
}

/// A staging texture that receives a copy of a row texture for the CPU to map.
pub(crate) struct Staging {
    texture: ID3D11Texture2D,
    width: u16,
}

/// The shaders and the inputs of one draw over a row.
pub(crate) struct RowDraw<'draw> {
    pub(crate) vertex: &'draw ID3D11VertexShader,
    pub(crate) pixel: &'draw ID3D11PixelShader,
    /// The pixel shader's resources in the slots t0 and t1.
    pub(crate) resources: [Option<ID3D11ShaderResourceView>; 2],
    /// The pixel shader's constant buffer in the slot b0, or `None` for a shader that reads none.
    pub(crate) constants: Option<&'draw ID3D11Buffer>,
}

/// A view of `texture` to sample, in its own format, or the failure of `step`.
pub(crate) fn sampled_view<S: CreateStep>(
    device: &ID3D11Device,
    step: S,
    texture: &ID3D11Texture2D,
) -> Result<ID3D11ShaderResourceView, S::Error> {
    let mut view = None;
    // SAFETY: `texture` is live, the view takes the texture's own format, and the out slot is live
    // for the call.
    let created = unsafe { device.CreateShaderResourceView(texture, None, Some(&raw mut view)) };
    made(step, created, view)
}

impl Row {
    /// A row texture of `width` texels on `device` and its view to draw into, or the failure of
    /// `texture_step` or of `view_step`.
    pub(crate) fn create<S: CreateStep + Copy>(
        device: &ID3D11Device,
        width: u32,
        texture_step: S,
        view_step: S,
    ) -> Result<Self, S::Error> {
        let (desc, width) = row_desc(texture_step, width, D3D11_USAGE_DEFAULT)?;
        let texture = row_texture(device, texture_step, &desc)?;
        let mut view = None;
        // SAFETY: `texture` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created = unsafe { device.CreateRenderTargetView(&texture, None, Some(&raw mut view)) };
        let view = made(view_step, created, view)?;
        Ok(Self { texture, view, width })
    }

    pub(crate) fn width(&self) -> u32 {
        u32::from(self.width)
    }

    /// Draws one [`Texel`] per column into the row with `draw`, and leaves the state of `context`
    /// as it found it.
    pub(crate) fn draw(&self, context: &ID3D11DeviceContext, draw: &RowDraw<'_>) {
        let saved = Saved::take(context);
        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: f32::from(self.width),
            Height: 1.0,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        // SAFETY: the slice holds one live view of this device, and no unordered-access view is
        // passed.
        unsafe {
            context.OMSetRenderTargetsAndUnorderedAccessViews(
                Some(&[Some(self.view.clone())]),
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
        bind_stages(context, draw);
        // SAFETY: the bound vertex shader builds its three vertices from the vertex index.
        unsafe { context.Draw(3, 0) };
        saved.restore(context);
    }
}

/// Binds the shaders, the two views and the constant buffer of `draw`, after the target, so neither
/// view is an output.
fn bind_stages(context: &ID3D11DeviceContext, draw: &RowDraw<'_>) {
    // SAFETY: the vertex shader takes no vertex input, so no layout is bound.
    unsafe { context.IASetInputLayout(None) };
    // SAFETY: the topology is a plain value.
    unsafe { context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST) };
    // SAFETY: the shader belongs to the context's device.
    unsafe { context.VSSetShader(draw.vertex, None) };
    // SAFETY: the shader belongs to the context's device.
    unsafe { context.PSSetShader(draw.pixel, None) };
    // SAFETY: an unbound stage takes no argument.
    unsafe { context.GSSetShader(None, None) };
    // SAFETY: an unbound stage takes no argument.
    unsafe { context.HSSetShader(None, None) };
    // SAFETY: an unbound stage takes no argument.
    unsafe { context.DSSetShader(None, None) };
    // SAFETY: the slice holds two views of this device, or none.
    unsafe { context.PSSetShaderResources(0, Some(&draw.resources)) };
    if let Some(constants) = draw.constants {
        // SAFETY: the slice holds one live buffer of this device made with the constant buffer
        // bind flag.
        unsafe { context.PSSetConstantBuffers(0, Some(&[Some(constants.clone())])) };
    }
}

impl Staging {
    /// A staging texture of `width` texels on `device` that the CPU can read, or the failure of
    /// `step`.
    pub(crate) fn create<S: CreateStep + Copy>(
        device: &ID3D11Device,
        width: u32,
        step: S,
    ) -> Result<Self, S::Error> {
        let (desc, width) = row_desc(step, width, D3D11_USAGE_STAGING)?;
        Ok(Self { texture: row_texture(device, step, &desc)?, width })
    }

    pub(crate) fn width(&self) -> u32 {
        u32::from(self.width)
    }

    /// Copies `row`, a row texture of the same width, into the staging texture.
    pub(crate) fn fill(&self, context: &ID3D11DeviceContext, row: &Row) {
        // SAFETY: both textures belong to the context's device and share the width, the one row and
        // the format.
        unsafe { context.CopyResource(&self.texture, &row.texture) };
    }

    /// Maps the staging texture with no wait and gives its row's texels to `visit`. Returns `None`
    /// while the GPU has not finished the copy into it, and the failure of `step` with the code of
    /// a map that failed for another reason. Calls `Map` through the raw vtable, since the
    /// `windows` crate turns the expected `DXGI_ERROR_WAS_STILL_DRAWING` into an error that takes
    /// the thread's COM error object.
    pub(crate) fn read<S: CreateStep, R>(
        &self,
        context: &ID3D11DeviceContext,
        step: S,
        visit: impl FnOnce(&[Texel]) -> R,
    ) -> Result<Option<R>, S::Error> {
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
            return Err(step.failed(code));
        }
        let len = usize::from(self.width);
        // SAFETY: the mapped row holds `width` texels of two four-byte floats, the layout of a
        // `Texel`, and `Map` aligns its data to at least 4 bytes, the alignment of a `Texel`, on
        // every feature level.
        let texels = unsafe { slice::from_raw_parts(mapped.pData.cast::<Texel>(), len) };
        let result = visit(texels);
        // SAFETY: the staging texture was mapped above, and `visit` returned, so no reference to
        // `texels` is left.
        unsafe { context.Unmap(&self.texture, 0) };
        Ok(Some(result))
    }
}

fn row_texture<S: CreateStep>(
    device: &ID3D11Device,
    step: S,
    desc: &D3D11_TEXTURE2D_DESC,
) -> Result<ID3D11Texture2D, S::Error> {
    let mut texture = None;
    // SAFETY: `desc` and the out slot are live for the call, and no initial data is passed.
    let created = unsafe { device.CreateTexture2D(desc, None, Some(&raw mut texture)) };
    made(step, created, texture)
}

/// The description of a row texture of `width` [`Texel`]s for `usage`, and the width as a viewport
/// side takes it. A width of 0 or above 16-bit fails `step` with `E_INVALIDARG`.
fn row_desc<S: CreateStep>(
    step: S,
    width: u32,
    usage: D3D11_USAGE,
) -> Result<(D3D11_TEXTURE2D_DESC, u16), S::Error> {
    let Some(short) = u16::try_from(width).ok().filter(|short| *short != 0) else {
        return Err(step.failed(E_INVALIDARG));
    };
    let staging = usage == D3D11_USAGE_STAGING;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: 1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R32G32_FLOAT,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: if staging { 0 } else { D3D11_BIND_RENDER_TARGET.0.cast_unsigned() },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0.cast_unsigned() } else { 0 },
        MiscFlags: 0,
    };
    Ok((desc, short))
}
