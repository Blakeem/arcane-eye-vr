//! The layer draw against a WARP device, with the shaders the build script compiled.

use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_LINELIST;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_STAGING, D3D11_VIEWPORT,
    ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
};

use super::{BYTES_PER_PIXEL, Gpu, desc, row_len, warp};
use crate::layer::{LayerDraw, LayerStep, Rect};
use crate::raw::texture_desc;

const BLUE: [u8; 4] = [0, 0, 255, 255];
/// Red at half coverage, with premultiplied alpha.
const HALF_RED: [u8; 4] = [128, 0, 0, 128];
/// `HALF_RED` over `BLUE`: the layer's color plus the target's color times one minus 128/255.
const HALF_RED_OVER_BLUE: [u8; 4] = [128, 0, 127, 255];

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// The number of pixels of a `width` by `height` texture.
fn texels(width: u32, height: u32) -> usize {
    (width as usize).checked_mul(height as usize).expect("a test texture size")
}

/// A texture of `width` by `height` pixels with `bind` flags that holds `pixels` in row order.
fn texture(gpu: &Gpu, width: u32, height: u32, bind: i32, pixels: &[[u8; 4]]) -> ID3D11Texture2D {
    assert_eq!(pixels.len(), texels(width, height), "one pixel per texel");
    let bytes = pixels.concat();
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: bytes.as_ptr().cast(),
        SysMemPitch: u32::try_from(row_len(width)).expect("a test pitch"),
        SysMemSlicePitch: 0,
    };
    let mut desc = desc(width, height, 1);
    desc.BindFlags = bind.cast_unsigned();
    let mut texture = None;
    // SAFETY: `data` points at one row of bytes per pitch, which outlive the call.
    unsafe {
        gpu.device.CreateTexture2D(&raw const desc, Some(&raw const data), Some(&raw mut texture))
    }
    .expect("create the test texture");
    texture.expect("the test texture")
}

fn filled(gpu: &Gpu, width: u32, height: u32, color: [u8; 4]) -> ID3D11Texture2D {
    let bind = (D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET).0;
    texture(gpu, width, height, bind, &vec![color; texels(width, height)])
}

/// The pixels of `texture` in row order.
fn pixels(gpu: &Gpu, texture: &ID3D11Texture2D) -> Vec<[u8; 4]> {
    let mut staging_desc = texture_desc(texture);
    staging_desc.Usage = D3D11_USAGE_STAGING;
    staging_desc.BindFlags = 0;
    staging_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0.cast_unsigned();
    let mut staging = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging)) }
        .expect("create the staging texture");
    let staging = staging.expect("the staging texture");
    // SAFETY: both textures belong to this device and have the same size and format.
    unsafe { gpu.context.CopyResource(&staging, texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the staging texture has CPU read access, and `mapped` is writable.
    unsafe { gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }.expect("map");
    let mut pixels = Vec::new();
    for y in 0..staging_desc.Height as usize {
        let start = y.checked_mul(mapped.RowPitch as usize).expect("a row offset");
        let line = mapped.pData.cast::<u8>().wrapping_add(start);
        // SAFETY: the mapped subresource holds `Height` rows of `RowPitch` bytes each, and a row
        // holds `Width` pixels of four bytes.
        let row = unsafe { std::slice::from_raw_parts(line, row_len(staging_desc.Width)) };
        pixels.extend_from_slice(row.as_chunks::<BYTES_PER_PIXEL>().0);
    }
    // SAFETY: the staging texture was mapped above.
    unsafe { gpu.context.Unmap(&staging, 0) };
    pixels
}

fn assert_near(actual: [u8; 4], expected: [u8; 4], place: &str) {
    for (channel, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(a.abs_diff(e) <= 1, "{place}, channel {channel}: {actual:?} against {expected:?}");
    }
}

#[test]
fn a_layer_blends_over_its_rect_with_premultiplied_alpha_and_leaves_the_rest() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("the built shaders load in a D3D11 device");
    let target = filled(&gpu, 8, 4, BLUE);
    let layer = filled(&gpu, 2, 2, HALF_RED);
    let right_half = Rect { x: 4, y: 0, width: 4, height: 4 };

    draw.draw(&gpu.context, &target, &layer, &[right_half]).expect("the draw");

    for (index, pixel) in pixels(&gpu, &target).into_iter().enumerate() {
        let expected = if index % 8 < 4 { BLUE } else { HALF_RED_OVER_BLUE };
        assert_near(pixel, expected, &format!("pixel {index}"));
    }
}

#[test]
fn each_rect_gets_the_entire_layer_in_the_layers_orientation() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = filled(&gpu, 4, 2, BLUE);
    let bind = D3D11_BIND_SHADER_RESOURCE.0;
    let layer = texture(&gpu, 2, 2, bind, &[RED, GREEN, WHITE, HALF_RED]);
    let halves =
        [Rect { x: 0, y: 0, width: 2, height: 2 }, Rect { x: 2, y: 0, width: 2, height: 2 }];

    draw.draw(&gpu.context, &target, &layer, &halves).expect("the draw");

    let rows = [[RED, GREEN, RED, GREEN], [WHITE, HALF_RED_OVER_BLUE, WHITE, HALF_RED_OVER_BLUE]];
    for (pixel, expected) in pixels(&gpu, &target).into_iter().zip(rows.as_flattened()) {
        assert_near(pixel, *expected, "a half");
    }
}

/// The state the restore test binds before the draw and reads after it.
#[derive(Debug, PartialEq)]
struct Bound {
    viewport: [f32; 4],
    target: Option<ID3D11RenderTargetView>,
    resource: Option<ID3D11ShaderResourceView>,
    topology: i32,
    blend_factor: [f32; 4],
    sample_mask: u32,
}

fn bound(gpu: &Gpu) -> Bound {
    let context = &gpu.context;
    let (mut viewport, mut count) = (D3D11_VIEWPORT::default(), 1);
    let (mut targets, mut resources) = ([None], [None]);
    let (mut blend_factor, mut sample_mask) = ([0.0; 4], 0);
    // SAFETY: the count names the one viewport slot, and both live for the call.
    unsafe { context.RSGetViewports(&raw mut count, Some(&raw mut viewport)) };
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
        target,
        resource,
        topology,
        blend_factor,
        sample_mask,
    }
}

/// Binds a viewport, a target, a pixel shader resource, a topology and blend values that differ from
/// the layer draw's, and returns what the context then holds.
fn bind_other_state(gpu: &Gpu) -> Bound {
    let (other_target, other_layer) = (filled(gpu, 4, 4, BLUE), filled(gpu, 2, 2, RED));
    let (mut target_view, mut layer_view) = (None, None);
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
    assert_eq!(state.topology, D3D11_PRIMITIVE_TOPOLOGY_LINELIST.0);
    assert_eq!(state.viewport, [1.0, 2.0, 3.0, 4.0]);
    state
}

#[test]
fn the_draw_leaves_the_context_state_as_it_found_it() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let (target, layer) = (filled(&gpu, 4, 4, BLUE), filled(&gpu, 2, 2, HALF_RED));
    let before = bind_other_state(&gpu);

    draw.draw(&gpu.context, &target, &layer, &[Rect { x: 0, y: 0, width: 4, height: 4 }])
        .expect("the draw");

    assert_eq!(bound(&gpu), before);
    let drawn = pixels(&gpu, &target);
    assert_near(*drawn.first().expect("a pixel"), HALF_RED_OVER_BLUE, "the draw ran");
}

#[test]
fn a_texture_without_the_needed_bind_flag_is_an_error_that_names_its_view() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let both = filled(&gpu, 2, 2, BLUE);
    let sample_only = texture(&gpu, 2, 2, D3D11_BIND_SHADER_RESOURCE.0, &[BLUE; 4]);
    let target_only = texture(&gpu, 2, 2, D3D11_BIND_RENDER_TARGET.0, &[BLUE; 4]);
    let rect = [Rect { x: 0, y: 0, width: 2, height: 2 }];

    let no_target = draw.draw(&gpu.context, &sample_only, &both, &rect).expect_err("no target");
    let no_layer = draw.draw(&gpu.context, &both, &target_only, &rect).expect_err("no layer");

    assert_eq!(no_target.step, LayerStep::TargetView);
    assert_eq!(no_layer.step, LayerStep::LayerView);
    assert_eq!(pixels(&gpu, &both), [BLUE; 4], "a failed draw changes nothing");
}

#[test]
fn the_draw_belongs_to_the_device_that_created_it() {
    let (gpu, other) = (warp(), warp());
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");

    assert!(draw.belongs_to(&gpu.device));
    assert!(!draw.belongs_to(&other.device));
}
