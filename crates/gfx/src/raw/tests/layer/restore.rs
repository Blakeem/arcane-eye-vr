//! The context state the layer draw finds is the state it leaves, against a WARP device.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_LINELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CULL_NONE, D3D11_FILL_SOLID, D3D11_RASTERIZER_DESC, D3D11_VIEWPORT,
    ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11ShaderResourceView,
};

use super::super::{Gpu, warp};
use super::{BLUE, HALF_RED, HALF_RED_OVER_BLUE, RED, area, assert_near, filled, halves, pixels};
use crate::layer::{LayerDraw, Placement, Rect};

/// The state the restore test binds before the draw and reads after it.
#[derive(Debug, PartialEq)]
struct Bound {
    viewport: [f32; 4],
    scissor: [i32; 4],
    raster: Option<ID3D11RasterizerState>,
    target: Option<ID3D11RenderTargetView>,
    resource: Option<ID3D11ShaderResourceView>,
    topology: i32,
    blend_factor: [f32; 4],
    sample_mask: u32,
}

fn bound(gpu: &Gpu) -> Bound {
    let context = &gpu.context;
    let (mut viewport, mut count) = (D3D11_VIEWPORT::default(), 1);
    let (mut scissor, mut scissor_count) = (RECT::default(), 1);
    let (mut targets, mut resources) = ([None], [None]);
    let (mut blend_factor, mut sample_mask) = ([0.0; 4], 0);
    // SAFETY: the count names the one viewport slot, and both live for the call.
    unsafe { context.RSGetViewports(&raw mut count, Some(&raw mut viewport)) };
    // SAFETY: the count names the one scissor slot, and both live for the call.
    unsafe { context.RSGetScissorRects(&raw mut scissor_count, Some(&raw mut scissor)) };
    // SAFETY: the context is live, and the call returns an error for no bound state.
    let raster = unsafe { context.RSGetState() }.ok();
    // SAFETY: the slice lives for the call.
    unsafe { context.OMGetRenderTargets(Some(&mut targets), None) };
    // SAFETY: the slice lives for the call.
    unsafe { context.PSGetShaderResources(0, Some(&mut resources)) };
    // SAFETY: both out slots live for the call.
    unsafe { context.OMGetBlendState(None, Some(&mut blend_factor), Some(&raw mut sample_mask)) };
    // SAFETY: the context is live.
    let topology = unsafe { context.IAGetPrimitiveTopology() }.0;
    let [target] = targets;
    let [resource] = resources;
    Bound {
        viewport: [viewport.TopLeftX, viewport.TopLeftY, viewport.Width, viewport.Height],
        scissor: [scissor.left, scissor.top, scissor.right, scissor.bottom],
        raster,
        target,
        resource,
        topology,
        blend_factor,
        sample_mask,
    }
}

/// A rasterizer state with the scissor test off and no culling, unlike the layer draw's state.
fn other_raster(gpu: &Gpu) -> ID3D11RasterizerState {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        DepthClipEnable: true.into(),
        ..Default::default()
    };
    let mut state = None;
    // SAFETY: `desc` and the out slot are live for the call.
    unsafe { gpu.device.CreateRasterizerState(&raw const desc, Some(&raw mut state)) }
        .expect("create the rasterizer state");
    state.expect("the rasterizer state")
}

/// Binds a viewport, a scissor rectangle, a rasterizer state, a target, a pixel shader resource, a
/// topology and blend values that differ from the layer draw's, and returns what the context then
/// holds.
fn bind_other_state(gpu: &Gpu) -> Bound {
    let (other_target, other_layer) = (filled(gpu, 4, 4, BLUE), filled(gpu, 2, 2, RED));
    let (mut target_view, mut layer_view) = (None, None);
    let raster = other_raster(gpu);
    let viewport = D3D11_VIEWPORT {
        TopLeftX: 1.0,
        TopLeftY: 2.0,
        Width: 3.0,
        Height: 4.0,
        ..Default::default()
    };
    // SAFETY: the texture and the out slot live for the call.
    unsafe { gpu.device.CreateRenderTargetView(&other_target, None, Some(&raw mut target_view)) }
        .expect("a target view");
    // SAFETY: the texture and the out slot live for the call.
    unsafe { gpu.device.CreateShaderResourceView(&other_layer, None, Some(&raw mut layer_view)) }
        .expect("a layer view");
    // SAFETY: the slice lives for the call.
    unsafe { gpu.context.RSSetViewports(Some(&[viewport])) };
    let scissor = RECT { left: 1, top: 1, right: 3, bottom: 2 };
    // SAFETY: the slice lives for the call.
    unsafe { gpu.context.RSSetScissorRects(Some(&[scissor])) };
    // SAFETY: the state belongs to this context's device.
    unsafe { gpu.context.RSSetState(&raster) };
    // SAFETY: the slice lives for the call.
    unsafe { gpu.context.OMSetRenderTargets(Some(&[target_view.clone()]), None) };
    // SAFETY: the slice lives for the call.
    unsafe { gpu.context.PSSetShaderResources(0, Some(&[layer_view.clone()])) };
    // SAFETY: the topology is a plain value.
    unsafe { gpu.context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_LINELIST) };
    // SAFETY: the blend factor lives for the call.
    unsafe { gpu.context.OMSetBlendState(None, Some(&[0.1, 0.2, 0.3, 0.4]), 0x0F) };
    let state = bound(gpu);
    assert_eq!(state.target, target_view, "the test bound its target");
    assert_eq!(state.resource, layer_view, "the test bound its resource");
    assert_eq!(state.raster, Some(raster), "the test bound its rasterizer state");
    assert_eq!(state.topology, D3D11_PRIMITIVE_TOPOLOGY_LINELIST.0);
    assert_eq!(state.viewport, [1.0, 2.0, 3.0, 4.0]);
    assert_eq!(state.scissor, [1, 1, 3, 2]);
    state
}

#[test]
fn the_draw_leaves_the_context_state_as_it_found_it() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let (target, layer) = (filled(&gpu, 4, 4, BLUE), filled(&gpu, 2, 2, HALF_RED));
    let before = bind_other_state(&gpu);

    draw.draw(&gpu.context, &target, &layer, &[Rect { x: 0, y: 0, width: 4, height: 4 }])
        .expect("the draw");
    assert_eq!(bound(&gpu), before);
    draw.draw_copy(&gpu.context, &target, &halves(4, 4)).expect("the copy draw");
    assert_eq!(bound(&gpu), before);
    let placed = Placement { area: area(1.0, 0.0, 2.0, 4.0), clip: halves(4, 4)[1] };
    draw.draw_placed(&gpu.context, &target, &layer, &[placed]).expect("the placed draw");

    assert_eq!(bound(&gpu), before);
    let drawn = pixels(&gpu, &target);
    assert_near(*drawn.first().expect("a pixel"), HALF_RED_OVER_BLUE, "the draw ran");
}
