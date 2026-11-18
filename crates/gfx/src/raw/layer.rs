//! The D3D11 calls of the layer draw: its device objects, the context state it saves and restores,
//! and the draw.
//!
//! The draw leaves the rasterizer state and the sampler unset. Direct3D's default rasterizer state
//! draws a clockwise triangle with no scissor test, and its default sampler filters linearly and
//! clamps, which is what the layer shader needs. With no depth-stencil view bound, the depth and
//! stencil tests pass, so the draw leaves the depth-stencil state as it is.
//!
//! The saved state holds no shader class instances and no unordered-access view above slot 7.

use crate::layer::{LayerError, LayerStep};
use crate::raw::{CreateStep, made};
use windows::Win32::Graphics::Direct3D::{
    D3D_PRIMITIVE_TOPOLOGY, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE, D3D11_BLEND_OP_ADD,
    D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT, D3D11_VIEWPORT,
    D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE, ID3D11BlendState,
    ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext, ID3D11DomainShader,
    ID3D11GeometryShader, ID3D11HullShader, ID3D11InputLayout, ID3D11PixelShader,
    ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D, ID3D11UnorderedAccessView, ID3D11VertexShader,
};
use windows::core::HRESULT;

const VERTEX_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/layer.vs.dxbc"));
const PIXEL_SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/layer.ps.dxbc"));

/// The output slots that render targets and unordered-access views share on feature level 11.0.
const OUTPUT_SLOTS: usize = D3D11_SIMULTANEOUS_RENDER_TARGET_COUNT as usize;
const VIEWPORT_SLOTS: usize = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as usize;

const WRITE_ALL_CHANNELS: u8 = 0x0F;
const _: () = assert!(WRITE_ALL_CHANNELS as i32 == D3D11_COLOR_WRITE_ENABLE_ALL.0);

/// The initial count that keeps an unordered-access view's hidden counter.
const KEEP_COUNTER: u32 = u32::MAX;

/// The device objects of the layer draw.
pub(crate) struct Pipeline {
    vertex: ID3D11VertexShader,
    pixel: ID3D11PixelShader,
    blend: ID3D11BlendState,
    device: ID3D11Device,
}

impl Pipeline {
    /// Creates the two shaders and the blend state for premultiplied alpha on `device`.
    pub(crate) fn create(device: &ID3D11Device) -> Result<Self, LayerError> {
        Ok(Self {
            vertex: vertex_shader(device)?,
            pixel: pixel_shader(device)?,
            blend: blend_state(device)?,
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
        let mut view = None;
        // SAFETY: `layer` is live, the view takes the texture's own format, and the out slot is
        // live for the call.
        let created =
            unsafe { self.device.CreateShaderResourceView(layer, None, Some(&raw mut view)) };
        made(LayerStep::LayerView, created, view)
    }

    /// Draws `layer` over each of `viewports` of `target`, and leaves the state of `context` as it
    /// found it.
    pub(crate) fn draw(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11RenderTargetView,
        layer: &ID3D11ShaderResourceView,
        viewports: impl IntoIterator<Item = D3D11_VIEWPORT>,
    ) {
        let saved = Saved::take(context);
        self.bind(context, target, layer);
        for viewport in viewports {
            // SAFETY: the slice holds one viewport and lives for the call.
            unsafe { context.RSSetViewports(Some(&[viewport])) };
            // SAFETY: the bound vertex shader builds its three vertices from the vertex index.
            unsafe { context.Draw(3, 0) };
        }
        saved.restore(context);
    }

    /// Binds the target first, so the layer is no output when its view is bound.
    fn bind(
        &self,
        context: &ID3D11DeviceContext,
        target: &ID3D11RenderTargetView,
        layer: &ID3D11ShaderResourceView,
    ) {
        let targets = [Some(target.clone())];
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
        // SAFETY: the blend state belongs to the context's device.
        unsafe { context.OMSetBlendState(&self.blend, None, u32::MAX) };
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
        unsafe { context.PSSetShaderResources(0, Some(&[Some(layer.clone())])) };
        // SAFETY: the slice holds one empty slot, which selects the default sampler.
        unsafe { context.PSSetSamplers(0, Some(&[None])) };
        // SAFETY: no state selects the default rasterizer state.
        unsafe { context.RSSetState(None) };
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

/// The context state the draw changes.
struct Saved {
    stages: Stages,
    raster: Raster,
    output: Output,
}

impl Saved {
    fn take(context: &ID3D11DeviceContext) -> Self {
        Self {
            stages: Stages::take(context),
            raster: Raster::take(context),
            output: Output::take(context),
        }
    }

    /// Unbinds the draw's target first, so a saved shader resource of that texture binds again.
    fn restore(self, context: &ID3D11DeviceContext) {
        // SAFETY: no view is passed.
        unsafe { context.OMSetRenderTargetsAndUnorderedAccessViews(None, None, 0, 0, None, None) };
        self.stages.restore(context);
        self.raster.restore(context);
        self.output.restore(context);
    }
}

/// The input assembler state, the five shaders and the pixel shader's first resource and sampler.
struct Stages {
    topology: D3D_PRIMITIVE_TOPOLOGY,
    layout: Option<ID3D11InputLayout>,
    vertex: Option<ID3D11VertexShader>,
    pixel: Option<ID3D11PixelShader>,
    geometry: Option<ID3D11GeometryShader>,
    hull: Option<ID3D11HullShader>,
    domain: Option<ID3D11DomainShader>,
    resource: [Option<ID3D11ShaderResourceView>; 1],
    sampler: [Option<ID3D11SamplerState>; 1],
}

impl Stages {
    fn take(context: &ID3D11DeviceContext) -> Self {
        let (mut vertex, mut pixel, mut geometry, mut hull, mut domain) =
            (None, None, None, None, None);
        let (mut resource, mut sampler) = ([None], [None]);
        // SAFETY: the context is live. The call returns the topology by value.
        let topology = unsafe { context.IAGetPrimitiveTopology() };
        // SAFETY: the context is live. An unbound layout comes back as an error, which is `None`.
        let layout = unsafe { context.IAGetInputLayout() }.ok();
        // SAFETY: the out slot is live for the call, and no class instance is asked for.
        unsafe { context.VSGetShader(&raw mut vertex, None, None) };
        // SAFETY: the out slot is live for the call, and no class instance is asked for.
        unsafe { context.PSGetShader(&raw mut pixel, None, None) };
        // SAFETY: the out slot is live for the call, and no class instance is asked for.
        unsafe { context.GSGetShader(&raw mut geometry, None, None) };
        // SAFETY: the out slot is live for the call, and no class instance is asked for.
        unsafe { context.HSGetShader(&raw mut hull, None, None) };
        // SAFETY: the out slot is live for the call, and no class instance is asked for.
        unsafe { context.DSGetShader(&raw mut domain, None, None) };
        // SAFETY: the slice holds one slot and lives for the call.
        unsafe { context.PSGetShaderResources(0, Some(&mut resource)) };
        // SAFETY: the slice holds one slot and lives for the call.
        unsafe { context.PSGetSamplers(0, Some(&mut sampler)) };
        Self { topology, layout, vertex, pixel, geometry, hull, domain, resource, sampler }
    }

    fn restore(self, context: &ID3D11DeviceContext) {
        // SAFETY: the topology is the value the context held.
        unsafe { context.IASetPrimitiveTopology(self.topology) };
        // SAFETY: the layout is the one the context held, or none.
        unsafe { context.IASetInputLayout(self.layout.as_ref()) };
        // SAFETY: the shader is the one the context held, or none.
        unsafe { context.VSSetShader(self.vertex.as_ref(), None) };
        // SAFETY: the shader is the one the context held, or none.
        unsafe { context.PSSetShader(self.pixel.as_ref(), None) };
        // SAFETY: the shader is the one the context held, or none.
        unsafe { context.GSSetShader(self.geometry.as_ref(), None) };
        // SAFETY: the shader is the one the context held, or none.
        unsafe { context.HSSetShader(self.hull.as_ref(), None) };
        // SAFETY: the shader is the one the context held, or none.
        unsafe { context.DSSetShader(self.domain.as_ref(), None) };
        // SAFETY: the slice holds the view the context held, or none.
        unsafe { context.PSSetShaderResources(0, Some(&self.resource)) };
        // SAFETY: the slice holds the sampler the context held, or none.
        unsafe { context.PSSetSamplers(0, Some(&self.sampler)) };
    }
}

/// The rasterizer state and the viewports.
struct Raster {
    state: Option<ID3D11RasterizerState>,
    viewports: [D3D11_VIEWPORT; VIEWPORT_SLOTS],
    viewport_count: u32,
}

impl Raster {
    fn take(context: &ID3D11DeviceContext) -> Self {
        let mut viewports = [D3D11_VIEWPORT::default(); VIEWPORT_SLOTS];
        let mut viewport_count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
        // SAFETY: the context is live. An unset state comes back as an error, which is `None`.
        let state = unsafe { context.RSGetState() }.ok();
        // SAFETY: the count names the array's length, and both live for the call.
        unsafe { context.RSGetViewports(&raw mut viewport_count, Some(viewports.as_mut_ptr())) };
        Self { state, viewports, viewport_count }
    }

    fn restore(self, context: &ID3D11DeviceContext) {
        let count = usize::try_from(self.viewport_count).unwrap_or(VIEWPORT_SLOTS);
        // SAFETY: the state is the one the context held, or none.
        unsafe { context.RSSetState(self.state.as_ref()) };
        // SAFETY: the slice holds the viewports the context held.
        unsafe { context.RSSetViewports(self.viewports.get(..count)) };
    }
}

/// The output merger's targets, its unordered-access views in the slots after them, and its blend
/// state.
struct Output {
    targets: [Option<ID3D11RenderTargetView>; OUTPUT_SLOTS],
    depth: Option<ID3D11DepthStencilView>,
    unordered: [Option<ID3D11UnorderedAccessView>; OUTPUT_SLOTS],
    blend: Option<ID3D11BlendState>,
    blend_factor: [f32; 4],
    sample_mask: u32,
}

impl Output {
    fn take(context: &ID3D11DeviceContext) -> Self {
        let mut targets = [const { None }; OUTPUT_SLOTS];
        let mut unordered = [const { None }; OUTPUT_SLOTS];
        let (mut depth, mut blend) = (None, None);
        let (mut blend_factor, mut sample_mask) = ([0.0; 4], 0);
        // SAFETY: the slice and the depth slot are live for the call.
        unsafe { context.OMGetRenderTargets(Some(&mut targets), Some(&raw mut depth)) };
        let first = first_unordered_slot(&targets);
        if let (Ok(start), Some(slots)) = (u32::try_from(first), unordered.get_mut(first..)) {
            // SAFETY: the slice lives for the call and holds the slots from `start` on.
            unsafe {
                context.OMGetRenderTargetsAndUnorderedAccessViews(None, None, start, Some(slots));
            }
        }
        // SAFETY: the three out slots are live for the call.
        unsafe {
            context.OMGetBlendState(
                Some(&raw mut blend),
                Some(&mut blend_factor),
                Some(&raw mut sample_mask),
            );
        }
        Self { targets, depth, unordered, blend, blend_factor, sample_mask }
    }

    fn restore(self, context: &ID3D11DeviceContext) {
        let first = first_unordered_slot(&self.targets);
        let targets = self.targets.get(..first);
        let unordered = self.unordered.get(first..).unwrap_or_default();
        let (start, count) = (
            u32::try_from(first).unwrap_or_default(),
            u32::try_from(unordered.len()).unwrap_or_default(),
        );
        let keep = [KEEP_COUNTER; OUTPUT_SLOTS];
        // SAFETY: the target slice, the `count` views and the `OUTPUT_SLOTS` counts live for the
        // call, and each view is one the context held.
        unsafe {
            context.OMSetRenderTargetsAndUnorderedAccessViews(
                targets,
                self.depth.as_ref(),
                start,
                count,
                Some(unordered.as_ptr()),
                Some(keep.as_ptr()),
            );
        }
        // SAFETY: the blend state is the one the context held, or none.
        unsafe {
            context.OMSetBlendState(
                self.blend.as_ref(),
                Some(&self.blend_factor),
                self.sample_mask,
            );
        }
    }
}

/// The slot after the last bound target, where a pixel shader's unordered-access views start.
fn first_unordered_slot(targets: &[Option<ID3D11RenderTargetView>]) -> usize {
    targets.iter().rposition(Option::is_some).map_or(0, |last| last.saturating_add(1))
}

impl CreateStep for LayerStep {
    type Error = LayerError;

    fn failed(self, code: HRESULT) -> LayerError {
        LayerError { step: self, code }
    }
}
