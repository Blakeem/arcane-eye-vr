//! The mesh draw's textures, its rejected input and the context it gets and leaves, against a WARP
//! device.

use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R16_FLOAT,
    DXGI_SAMPLE_DESC,
};
use windows::core::Interface as _;

use super::super::layer::pixels;
use super::super::{Gpu, warp};
use super::{WHITE, create, item, square, target, texture, triangle, view};
use crate::mesh::tests::translation;
use crate::mesh::{MeshError, MeshStep, MeshTexture, Vertex};
use crate::raw::texture_desc;
use crate::region::{Placement, RegionDraw};

/// The texture behind a mesh texture's view.
fn texture_of(texture: &MeshTexture) -> ID3D11Texture2D {
    // SAFETY: the view is live, and the call adds the reference the result holds.
    let resource = unsafe { texture.view.GetResource() }.expect("the view's resource");
    resource.cast().expect("a 2D texture")
}

/// The one texel of the 1 by 1 level `level` of `texture`.
fn last_level_texel(gpu: &Gpu, texture: &ID3D11Texture2D, level: u32) -> [u8; 4] {
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: 1,
        Height: 1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0.cast_unsigned(),
        MiscFlags: 0,
    };
    let mut staging = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging)) }
        .expect("create the staging texture");
    let staging = staging.expect("the staging texture");
    // SAFETY: both textures belong to this device, and the level is one texel of the same format.
    unsafe { gpu.context.CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, level, None) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the staging texture has CPU read access, and `mapped` is writable.
    unsafe { gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }.expect("map");
    // SAFETY: the mapped subresource holds one texel of four bytes.
    let texel = unsafe { mapped.pData.cast::<[u8; 4]>().read() };
    // SAFETY: the staging texture was mapped above.
    unsafe { gpu.context.Unmap(&staging, 0) };
    texel
}

#[test]
fn a_texture_has_a_full_mip_chain_averaged_in_linear_light() {
    let gpu = warp();
    let draw = create(&gpu);
    let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];
    let texture = texture_of(&draw.texture(&gpu.context, 2, 2, &colors).expect("the texture"));
    let desc = texture_desc(&texture);
    assert_eq!((desc.MipLevels, desc.Format), (2, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB));
    // The linear means 0.5, 0.5 and 0.25 in the sRGB encoding. A mean of the encoded values would
    // give 128, 128 and 64.
    let expected = [188_u8, 188, 137, 255];
    let texel = last_level_texel(&gpu, &texture, 1);
    for (shown, wanted) in texel.iter().zip(expected) {
        assert!(shown.abs_diff(wanted) <= 2, "{texel:?} against {expected:?}");
    }
    let big = draw.texture(&gpu.context, 6, 3, &[[0; 4]; 18]).expect("a texture of 6 by 3");
    assert_eq!(texture_desc(&texture_of(&big)).MipLevels, 3, "6 by 3, 3 by 1 and 1 by 1");
}

#[test]
fn a_mesh_or_texture_of_inconsistent_data_is_rejected_with_the_step_that_checked_it() {
    let gpu = warp();
    let draw = create(&gpu);
    let corner = Vertex { position: [0.0; 3], normal: [0.0, 0.0, 1.0], uv: [0.0; 2] };
    let bad_index = MeshError { step: MeshStep::IndexBuffer, code: E_INVALIDARG };
    assert_eq!(draw.mesh(&[corner; 3], &[0, 1, 3]).err(), Some(bad_index), "an index past the end");
    assert_eq!(draw.mesh(&[corner; 3], &[0, 1]).err(), Some(bad_index), "a partial triangle");
    assert_eq!(draw.mesh(&[], &[]).err(), Some(bad_index), "no triangle");
    let bad_texture = MeshError { step: MeshStep::Texture, code: E_INVALIDARG };
    let short = draw.texture(&gpu.context, 2, 2, &[[0; 4]; 3]);
    assert_eq!(short.err(), Some(bad_texture), "a texel with no pixel");
    assert_eq!(draw.texture(&gpu.context, 0, 2, &[]).err(), Some(bad_texture), "no width");
}

#[test]
fn a_target_view_in_a_format_outside_the_targets_family_is_rejected() {
    let gpu = warp();
    let draw = create(&gpu);
    let wrong = draw.target(&texture(&gpu, 4, 4), DXGI_FORMAT_R16_FLOAT);
    assert_eq!(wrong.err().map(|error| error.step), Some(MeshStep::TargetView));
}

/// The pixels a region draw gives of a 4 by 4 pattern.
fn region_pixels(gpu: &Gpu, region: &RegionDraw) -> Vec<[u8; 4]> {
    let pattern: Vec<[u8; 4]> = (0..16_u8).map(|value| [value, !value, 7, 255]).collect();
    let source = region.image(4, 4, &pattern).expect("the region source");
    let source_view = region.source_view(&source).expect("the source view");
    let target_texture = texture(gpu, 4, 4);
    let target_view =
        region.target_view(&target_texture, DXGI_FORMAT_R8G8B8A8_UNORM).expect("the region target");
    let placement = Placement::new((0, 0), (4, 4)).expect("the placement");
    region.draw(&gpu.context, &target_view, &source_view, placement);
    pixels(gpu, &target_texture)
}

#[test]
fn a_region_draw_after_a_mesh_draw_gives_the_pixels_it_gives_with_none_before_it() {
    let gpu = warp();
    let region = RegionDraw::create(&gpu.device).expect("the region draw");
    let alone = region_pixels(&gpu, &region);
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let (_texture, mesh_target) = target(&gpu, &draw, 8, 8);
    let items = [item(&mesh, translation(0.0, 0.0, -1.0), WHITE)];
    draw.draw(&gpu.context, &mesh_target, &view(), &items).expect("the mesh draw");

    assert_eq!(region_pixels(&gpu, &region), alone);
}

#[test]
fn the_context_holds_no_object_of_the_mesh_draw_after_a_draw() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let (_texture, mesh_target) = target(&gpu, &draw, 8, 8);
    let items = [item(&mesh, translation(0.0, 0.0, -1.0), WHITE)];
    draw.draw(&gpu.context, &mesh_target, &view(), &items).expect("the mesh draw");
    let context = &gpu.context;
    let (mut targets, mut depth_view) = ([None], None);
    let (mut vertices, mut indices, mut depth_state, mut blend) = ([None], None, None, None);
    // SAFETY: the slice and the out slot live for the call.
    unsafe { context.OMGetRenderTargets(Some(&mut targets), Some(&raw mut depth_view)) };
    // SAFETY: the out slot names one buffer slot and lives for the call.
    unsafe { context.IAGetVertexBuffers(0, 1, Some(vertices.as_mut_ptr()), None, None) };
    // SAFETY: the out slot lives for the call.
    unsafe { context.IAGetIndexBuffer(Some(&raw mut indices), None, None) };
    // SAFETY: the out slot lives for the call.
    unsafe { context.OMGetDepthStencilState(Some(&raw mut depth_state), None) };
    // SAFETY: the out slot lives for the call.
    unsafe { context.OMGetBlendState(Some(&raw mut blend), None, None) };
    // SAFETY: the context is live, and the call returns an error for no bound layout.
    let layout = unsafe { context.IAGetInputLayout() }.ok();
    // SAFETY: the context is live, and the call returns an error for no bound state.
    let raster = unsafe { context.RSGetState() }.ok();
    // SAFETY: the context is live.
    let topology = unsafe { context.IAGetPrimitiveTopology() };
    assert_eq!((targets, depth_view), ([None], None), "the render targets");
    assert_eq!((vertices, indices), ([None], None), "the vertex and index buffers");
    assert_eq!((depth_state, blend), (None, None), "the depth stencil and blend states");
    assert_eq!((layout, raster), (None, None), "the input layout and rasterizer state");
    assert_eq!(topology, D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED, "the topology");
}

#[test]
fn the_shader_stages_hold_no_object_of_the_mesh_draw_after_a_draw() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let (_texture, mesh_target) = target(&gpu, &draw, 8, 8);
    let items = [item(&mesh, translation(0.0, 0.0, -1.0), WHITE)];
    draw.draw(&gpu.context, &mesh_target, &view(), &items).expect("the mesh draw");
    let context = &gpu.context;
    let (mut vertex, mut pixel) = (None, None);
    let (mut vertex_constants, mut pixel_constants) = ([None, None], [None, None]);
    let (mut resources, mut samplers) = ([None], [None]);
    // SAFETY: the out slot lives for the call.
    unsafe { context.VSGetShader(&raw mut vertex, None, None) };
    // SAFETY: the out slot lives for the call.
    unsafe { context.PSGetShader(&raw mut pixel, None, None) };
    // SAFETY: the slice names two slots and lives for the call.
    unsafe { context.VSGetConstantBuffers(0, Some(&mut vertex_constants)) };
    // SAFETY: the slice names two slots and lives for the call.
    unsafe { context.PSGetConstantBuffers(0, Some(&mut pixel_constants)) };
    // SAFETY: the slice names one slot and lives for the call.
    unsafe { context.PSGetShaderResources(0, Some(&mut resources)) };
    // SAFETY: the slice names one slot and lives for the call.
    unsafe { context.PSGetSamplers(0, Some(&mut samplers)) };
    assert_eq!((vertex, pixel), (None, None), "the shaders");
    assert_eq!((vertex_constants, pixel_constants), ([None, None], [None, None]), "b0 and b1");
    assert_eq!((resources, samplers), ([None], [None]), "t0 and s0");
}

#[test]
fn a_blend_state_left_on_the_context_does_not_reach_the_mesh_draw() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let (texture, mesh_target) = target(&gpu, &draw, 8, 8);
    let items = [item(&mesh, translation(0.0, 0.0, -1.0), WHITE)];
    // SAFETY: an empty blend state selects the default, and the sample mask of 0 writes no sample.
    unsafe { gpu.context.OMSetBlendState(None, None, 0) };
    draw.draw(&gpu.context, &mesh_target, &view(), &items).expect("the mesh draw");
    let middle = pixels(&gpu, &texture).get(36).copied().expect("row 4, column 4");
    assert_eq!(middle[3], 255, "the middle pixel: {middle:?}");
}
