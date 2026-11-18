//! The context state the layer draw changes, saved before the draw and restored after it.
//!
//! The saved state holds no shader class instances and no unordered-access view above slot 7.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_VIEWPORT, D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE, ID3D11BlendState,
    ID3D11DepthStencilView, ID3D11DeviceContext, ID3D11DomainShader, ID3D11GeometryShader,
    ID3D11HullShader, ID3D11InputLayout, ID3D11PixelShader, ID3D11RasterizerState,
    ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11UnorderedAccessView, ID3D11VertexShader,
};

use super::OUTPUT_SLOTS;

const VIEWPORT_SLOTS: usize = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE as usize;

/// The initial count that keeps an unordered-access view's hidden counter.
const KEEP_COUNTER: u32 = u32::MAX;

/// The context state the draw changes.
pub(super) struct Saved {
    stages: Stages,
    raster: Raster,
    output: Output,
}

impl Saved {
    pub(super) fn take(context: &ID3D11DeviceContext) -> Self {
        Self {
            stages: Stages::take(context),
            raster: Raster::take(context),
            output: Output::take(context),
        }
    }

    /// Unbinds the draw's target first, so a saved shader resource of that texture binds again.
    pub(super) fn restore(self, context: &ID3D11DeviceContext) {
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

/// The rasterizer state, the viewports and the scissor rectangles.
struct Raster {
    state: Option<ID3D11RasterizerState>,
    viewports: [D3D11_VIEWPORT; VIEWPORT_SLOTS],
    viewport_count: u32,
    scissors: [RECT; VIEWPORT_SLOTS],
    scissor_count: u32,
}

impl Raster {
    fn take(context: &ID3D11DeviceContext) -> Self {
        let mut viewports = [D3D11_VIEWPORT::default(); VIEWPORT_SLOTS];
        let mut viewport_count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
        let mut scissors = [RECT::default(); VIEWPORT_SLOTS];
        let mut scissor_count = D3D11_VIEWPORT_AND_SCISSORRECT_OBJECT_COUNT_PER_PIPELINE;
        // SAFETY: the context is live. An unset state comes back as an error, which is `None`.
        let state = unsafe { context.RSGetState() }.ok();
        // SAFETY: the count names the array's length, and both live for the call.
        unsafe { context.RSGetViewports(&raw mut viewport_count, Some(viewports.as_mut_ptr())) };
        // SAFETY: the count names the array's length, and both live for the call.
        unsafe { context.RSGetScissorRects(&raw mut scissor_count, Some(scissors.as_mut_ptr())) };
        Self { state, viewports, viewport_count, scissors, scissor_count }
    }

    fn restore(self, context: &ID3D11DeviceContext) {
        let viewports = usize::try_from(self.viewport_count).unwrap_or(VIEWPORT_SLOTS);
        let scissors = usize::try_from(self.scissor_count).unwrap_or(VIEWPORT_SLOTS);
        // SAFETY: the state is the one the context held, or none.
        unsafe { context.RSSetState(self.state.as_ref()) };
        // SAFETY: the slice holds the viewports the context held.
        unsafe { context.RSSetViewports(self.viewports.get(..viewports)) };
        // SAFETY: the slice holds the scissor rectangles the context held.
        unsafe { context.RSSetScissorRects(self.scissors.get(..scissors)) };
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
