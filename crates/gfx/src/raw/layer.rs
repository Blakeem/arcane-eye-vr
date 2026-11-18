//! The D3D11 calls of the layer draw: its device objects, the copy of a target it draws back over
//! that target, a layer texture made from pixels, and the draw. The context state the draw saves
//! and restores is in `saved`.
//!
//! The draw leaves the sampler unset. Direct3D's default sampler filters linearly and clamps, which
//! is what the layer shader needs. Its rasterizer state is the default state with the scissor test
//! on, which draws a clockwise triangle and clips each region to its scissor rectangle. With no
//! depth-stencil view bound, the depth and stencil tests pass, so the draw leaves the depth-stencil
//! state as it is.

mod saved;

use windows::Win32::Foundation::{E_INVALIDARG, RECT};
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BOX, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_CULL_BACK, D3D11_FILL_SOLID,
    D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT,
    D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE, D3D11_VIEWPORT,
    ID3D11BlendState, ID3D11Device, ID3D11DeviceContext, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::HRESULT;

use self::saved::Saved;
use super::{CreateStep, SourceKey, copy_first_subresource, made};
use crate::layer::{LayerError, LayerStep};

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/layer.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/layer.ps.dxbc"));

/// The output slots that render targets and unordered-access views share on feature level 11.0.
const OUTPUT_SLOTS: usize = D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT as usize;

const WRITE_ALL_CHANNELS: u8 = 0x0F;
const _: () = assert!(WRITE_ALL_CHANNELS as i32 == D3D11_COLOR_WRITE_ENABLE_ALL.0);

/// How the draw combines the layer with the target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blend {
    /// The layer plus the target times one minus the layer's alpha.
    Premultiplied,
    /// The layer replaces the target's pixels, alpha included.
    Replace,
}

/// The views of one draw and how it blends them.
pub(crate) struct Pass<'views> {
    pub(crate) target: &'views ID3D11RenderTargetView,
    pub(crate) layer: &'views ID3D11ShaderResourceView,
    pub(crate) blend: Blend,
}

/// One region of a draw: the viewport the layer is stretched to and the rectangle it is clipped to.
pub(crate) struct Region {
    pub(crate) viewport: D3D11_VIEWPORT,
    pub(crate) scissor: RECT,
}

/// The device objects of the layer draw.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    device: ID3D11Device,
}

impl Pipeline {
    /// Creates the two shaders, the blend state for premultiplied alpha and the rasterizer state
    /// with the scissor test on `device`.
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, LayerError> {
        Ok(Self {
            vertex: vertex_shader(device)?,
            pixel: pixel_shader(device)?,
            blend: blend_state(device)?,
            raster: rasterizer_state(device)?,
            device: device.clone(),
        })
    }

    pub(crate) fn belongs_to(&self, device: &ID3D11Device) -> bool {
        self.device == *device
    }

    /// A view of `target` to draw into.
    pub(crate) fn target_view(
        &self,
        target: &ID3D11Texture2D,
    ) -> Result<ID3D11RenderTargetView, LayerError> {
        let mut view = None;
        // SAFETY: `target` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateRenderTargetView(target, None, Some(&raw mut view)) };
        made(LayerStep::TargetView, created, view)
    }

    /// A view of `layer` to sample.
    pub(crate) fn layer_view(
        &self,
        layer: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView, LayerError> {
        self.sampled_view(LayerStep::LayerView, layer)
    }

    /// A view of `texture` to sample, or the failure of `step`.
    fn sampled_view(
        &self,
        step: LayerStep,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11ShaderResourceView, LayerError> {
        let mut view = None;
        // SAFETY: `texture` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateShaderResourceView(texture, None, Some(&raw mut view)) };
        made(step, created, view)
    }

    /// A copy texture, and its view to sample, for a target that `desc` describes.
    pub(crate) fn target_copy(
        &self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<TargetCopy, LayerError> {
        let key = SourceKey::of(desc);
        let copy_desc = key.copy_desc(0);
        let mut texture = None;
        // SAFETY: `copy_desc` and the out slot are live for the call, and no initial data is passed.
        let created = unsafe {
            self.device.CreateTexture2D(&raw const copy_desc, None, Some(&raw mut texture))
        };
        let texture = made(LayerStep::TargetCopy, created, texture)?;
        let view = self.sampled_view(LayerStep::CopyView, &texture)?;
        Ok(TargetCopy { texture, view, key })
    }

    /// A shader resource texture of `width` by `height` pixels in the format the layer pixels have,
    /// which holds `pixels` in row order.
    pub(crate) fn image(
        &self,
        width: u16,
        height: u16,
        pixels: &[[u8; 4]],
    ) -> Result<ID3D11Texture2D, LayerError> {
        let invalid = LayerError { step: LayerStep::Image, code: E_INVALIDARG };
        let texels = usize::from(width).checked_mul(usize::from(height));
        if width == 0 || height == 0 || texels != Some(pixels.len()) {
            return Err(invalid);
        }
        let pitch = u32::from(width).checked_mul(4).ok_or(invalid)?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: u32::from(width),
            Height: u32::from(height),
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0.cast_unsigned(),
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr().cast(),
            SysMemPitch: pitch,
            SysMemSlicePitch: 0,
        };
        let mut texture = None;
        // SAFETY: `data` points at `height` rows of `pitch` bytes each, since `pixels` holds one
        // four-byte pixel per texel, and `desc`, `data` and the out slot are live for the call.
        let created = unsafe {
            self.device.CreateTexture2D(
                &raw const desc,
                Some(&raw const data),
                Some(&raw mut texture),
            )
        };
        made(LayerStep::Image, created, texture)
    }

    /// Draws the layer of `pass` over each of `regions` of its target, and leaves the state of
    /// `context` as it found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        pass: &Pass<'_>,
        regions: impl IntoIterator<Item = Region>,
    ) {
        let saved = Saved::take(context);
        self.bind(context, pass);
        for region in regions {
            // SAFETY: the slice holds one viewport and lives for the call.
            unsafe { context.RSSetViewports(Some(&[region.viewport])) };
            // SAFETY: the slice holds one rectangle and lives for the call.
            unsafe { context.RSSetScissorRects(Some(&[region.scissor])) };
            // SAFETY: the bound vertex shader builds its three vertices from the vertex index.
            unsafe { context.Draw(3, 0) };
        }
        saved.restore(context);
    }

    /// Binds the target first, so the layer is no output when its view is bound.
    fn bind(&self, context: &ID3D11DeviceContext, pass: &Pass<'_>) {
        let targets = [Some(pass.target.clone())];
        let blend_state = match pass.blend {
            Blend::Premultiplied => Some(&self.blend),
            Blend::Replace => None,
        };
        // SAFETY: the slice holds one live view, and no unordered-access view is passed.
        unsafe {
            context.OMSetRenderTargetsAndUnorderedAccessViews(
                Some(&targets),
                None,
                1,
                0,
                None,
                None,
            );
        }
        // SAFETY: the blend state belongs to the context's device. None selects Direct3D's default
        // state, which writes the layer's channels with no blending.
        unsafe { context.OMSetBlendState(blend_state, None, u32::MAX) };
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
        // SAFETY: the slice holds one live view.
        unsafe { context.PSSetShaderResources(0, Some(&[Some(pass.layer.clone())])) };
        // SAFETY: the slice holds one empty slot, which selects the default sampler.
        unsafe { context.PSSetSamplers(0, Some(&[None])) };
        // SAFETY: the state belongs to the context's device.
        unsafe { context.RSSetState(&self.raster) };
    }
}

/// A single-sampled copy of a target texture, since the draw cannot sample the texture it draws
/// into.
pub(crate) struct TargetCopy {
    texture: ID3D11Texture2D,
    view: ID3D11ShaderResourceView,
    key: SourceKey,
}

impl TargetCopy {
    /// Whether the copy has the size and format of a target that `desc` describes.
    pub(crate) fn fits(&self, desc: &D3D11_TEXTURE2D_DESC) -> bool {
        self.key == SourceKey::of(desc)
    }

    pub(crate) const fn view(&self) -> &ID3D11ShaderResourceView {
        &self.view
    }

    /// Copies the first subresource of `target`, which `desc` describes, into the copy. A
    /// single-sampled target with a `source` box inside it gives only the pixels of that box, to the
    /// same place. Any other target is copied entire, and a multisampled one is resolved, since a
    /// box copy cannot read a multisampled texture.
    pub(crate) fn fill(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11Texture2D,
        desc: &D3D11_TEXTURE2D_DESC,
        source: Option<D3D11_BOX>,
    ) {
        let Some(source) = source.filter(|_| desc.SampleDesc.Count == 1) else {
            copy_first_subresource(context, &self.texture, target, desc);
            return;
        };
        // SAFETY: both textures belong to the context's device, are single-sampled and share the
        // size and format of `desc`, and the caller's box lies inside that size.
        unsafe {
            context.CopySubresourceRegion(
                &self.texture,
                0,
                source.left,
                source.top,
                0,
                target,
                0,
                Some(&raw const source),
            );
        }
    }
}

fn vertex_shader(device: &ID3D11Device) -> Result<ID3D11VertexShader, LayerError> {
    let mut shader = None;
    // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
    let created = unsafe { device.CreateVertexShader(VERTEX_SHADER, None, Some(&raw mut shader)) };
    made(LayerStep::VertexShader, created, shader)
}

fn pixel_shader(device: &ID3D11Device) -> Result<ID3D11PixelShader, LayerError> {
    let mut shader = None;
    // SAFETY: the bytecode is the build script's DXBC, and the out slot is live for the call.
    let created = unsafe { device.CreatePixelShader(PIXEL_SHADER, None, Some(&raw mut shader)) };
    made(LayerStep::PixelShader, created, shader)
}

/// The blend state for a source with premultiplied alpha: the source plus the target times one minus
/// the source's alpha.
fn blend_state(device: &ID3D11Device) -> Result<ID3D11BlendState, LayerError> {
    let target = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: true.into(),
        SrcBlend: D3D11_BLEND_ONE,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: WRITE_ALL_CHANNELS,
    };
    let desc = D3D11_BLEND_DESC {
        AlphaToCoverageEnable: false.into(),
        IndependentBlendEnable: false.into(),
        RenderTarget: [target; OUTPUT_SLOTS],
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    let created = unsafe { device.CreateBlendState(&raw const desc, Some(&raw mut state)) };
    made(LayerStep::BlendState, created, state)
}

/// Direct3D's default rasterizer state with the scissor test on.
fn rasterizer_state(device: &ID3D11Device) -> Result<ID3D11RasterizerState, LayerError> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_BACK,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: true.into(),
        MultisampleEnable: false.into(),
        AntialiasedLineEnable: false.into(),
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    let created = unsafe { device.CreateRasterizerState(&raw const desc, Some(&raw mut state)) };
    made(LayerStep::RasterizerState, created, state)
}

impl CreateStep for LayerStep {
    type Error = LayerError;

    fn failed(self, code: HRESULT) -> LayerError {
        LayerError { step: self, code }
    }
}
