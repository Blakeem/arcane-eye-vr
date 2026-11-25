//! The layer draw against a WARP device, with the shaders the build script compiled.

mod placed_copy;
pub(super) mod restore;

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_USAGE_STAGING, ID3D11Texture2D,
};

use super::{BYTES_PER_PIXEL, Gpu, desc, row_len, warp};
use crate::layer::{Area, LayerDraw, LayerStep, Placement, Rect, layer_aspect};
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
pub(super) fn texels(width: u32, height: u32) -> usize {
    (width as usize).checked_mul(height as usize).expect("a test texture size")
}

/// A texture of `width` by `height` pixels with `bind` flags that holds `pixels` in row order.
pub(super) fn texture(
    gpu: &Gpu,
    width: u32,
    height: u32,
    bind: i32,
    pixels: &[[u8; 4]],
) -> ID3D11Texture2D {
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
pub(super) fn pixels(gpu: &Gpu, texture: &ID3D11Texture2D) -> Vec<[u8; 4]> {
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

/// The placement that stretches a layer to `rect` and clips it to `rect`.
pub(super) fn fitted(rect: Rect) -> Placement {
    let area = area(rect.x.into(), rect.y.into(), rect.width.into(), rect.height.into());
    Placement { area, clip: rect }
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

    draw.draw_placed(&gpu.context, &target, &layer, &[fitted(right_half)]).expect("the draw");

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

    draw.draw_placed(&gpu.context, &target, &layer, &halves.map(fitted)).expect("the draw");

    let rows = [[RED, GREEN, RED, GREEN], [WHITE, HALF_RED_OVER_BLUE, WHITE, HALF_RED_OVER_BLUE]];
    for (pixel, expected) in pixels(&gpu, &target).into_iter().zip(rows.as_flattened()) {
        assert_near(pixel, *expected, "a half");
    }
}

#[test]
fn a_texture_without_the_needed_bind_flag_is_an_error_that_names_its_view() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let both = filled(&gpu, 2, 2, BLUE);
    let sample_only = texture(&gpu, 2, 2, D3D11_BIND_SHADER_RESOURCE.0, &[BLUE; 4]);
    let target_only = texture(&gpu, 2, 2, D3D11_BIND_RENDER_TARGET.0, &[BLUE; 4]);
    let placed = [fitted(Rect { x: 0, y: 0, width: 2, height: 2 })];

    let no_target =
        draw.draw_placed(&gpu.context, &sample_only, &both, &placed).expect_err("no target");
    let no_layer =
        draw.draw_placed(&gpu.context, &both, &target_only, &placed).expect_err("no layer");

    assert_eq!(no_target.step, LayerStep::TargetView);
    assert_eq!(no_layer.step, LayerStep::LayerView);
    assert_eq!(pixels(&gpu, &both), [BLUE; 4], "a failed draw changes nothing");
}

/// Two colors with alpha 0 whose mean in each channel is a whole number.
const DARK_CLEAR: [u8; 4] = [64, 32, 192, 0];
const LIGHT_CLEAR: [u8; 4] = [192, 96, 64, 0];
const MEAN_CLEAR: [u8; 4] = [128, 64, 128, 0];

/// The left half and the right half of a target `width` pixels wide and `height` pixels high.
fn halves(width: u16, height: u16) -> [Rect; 2] {
    let half = width / 2;
    [Rect { x: 0, y: 0, width: half, height }, Rect { x: half, y: 0, width: half, height }]
}

/// A render target of `width` by 2 pixels whose columns alternate `DARK_CLEAR` and `LIGHT_CLEAR`.
fn striped(gpu: &Gpu, width: u32) -> ID3D11Texture2D {
    let row: Vec<[u8; 4]> =
        (0..width).map(|x| if x % 2 == 0 { DARK_CLEAR } else { LIGHT_CLEAR }).collect();
    texture(gpu, width, 2, D3D11_BIND_RENDER_TARGET.0, &row.repeat(2))
}

#[test]
fn a_copy_draw_squeezes_each_pair_of_columns_into_their_mean_and_replaces_the_pixels() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = striped(&gpu, 8);

    draw.draw_copy(&gpu.context, &target, &halves(8, 2)).expect("the copy draw");

    for (index, pixel) in pixels(&gpu, &target).into_iter().enumerate() {
        assert_near(pixel, MEAN_CLEAR, &format!("pixel {index}"));
    }
}

#[test]
fn each_half_gets_the_entire_target_at_half_its_width_in_its_orientation() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let picture = [RED, RED, GREEN, GREEN, WHITE, WHITE, BLUE, BLUE];
    let target = texture(&gpu, 4, 2, D3D11_BIND_RENDER_TARGET.0, &picture);

    draw.draw_copy(&gpu.context, &target, &halves(4, 2)).expect("the copy draw");

    let rows = [[RED, GREEN, RED, GREEN], [WHITE, BLUE, WHITE, BLUE]];
    assert_eq!(pixels(&gpu, &target), rows.as_flattened());
}

#[test]
fn a_target_of_another_size_gets_a_new_copy() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let (small, large) = (striped(&gpu, 4), striped(&gpu, 8));

    draw.draw_copy(&gpu.context, &small, &halves(4, 2)).expect("the small copy draw");
    draw.draw_copy(&gpu.context, &large, &halves(8, 2)).expect("the large copy draw");

    for pixel in pixels(&gpu, &small).into_iter().chain(pixels(&gpu, &large)) {
        assert_near(pixel, MEAN_CLEAR, "a squeezed pixel");
    }
}

/// A 4x multisampled render target of 4 by 2 pixels, cleared to green.
fn green_multisampled(gpu: &Gpu) -> ID3D11Texture2D {
    let multisampled = desc(4, 2, 4);
    let (mut target, mut view) = (None, None);
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const multisampled, None, Some(&raw mut target)) }
        .expect("create a 4x multisampled target");
    let target = target.expect("the multisampled target");
    // SAFETY: `target` is a render target of this device, and the out slot is live.
    unsafe { gpu.device.CreateRenderTargetView(&target, None, Some(&raw mut view)) }
        .expect("create the render target view");
    // SAFETY: the view belongs to this device's immediate context.
    unsafe { gpu.context.ClearRenderTargetView(&view.expect("the view"), &[0.0, 1.0, 0.0, 1.0]) };
    target
}

/// The pixels of `target`, a multisampled texture of 4 by 2 pixels, resolved.
fn resolved_pixels(gpu: &Gpu, target: &ID3D11Texture2D) -> Vec<[u8; 4]> {
    let resolved_desc = desc(4, 2, 1);
    let mut resolved = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const resolved_desc, None, Some(&raw mut resolved)) }
        .expect("create the resolved texture");
    let resolved = resolved.expect("the resolved texture");
    let format = resolved_desc.Format;
    // SAFETY: both textures belong to this device, and `resolved` is single-sampled with the
    // target's size and format.
    unsafe { gpu.context.ResolveSubresource(&resolved, 0, target, 0, format) };
    pixels(gpu, &resolved)
}

#[test]
fn a_multisampled_target_is_resolved_into_its_copy() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = green_multisampled(&gpu);

    draw.draw_copy(&gpu.context, &target, &halves(4, 2)).expect("the copy draw");

    assert_eq!(resolved_pixels(&gpu, &target), [GREEN; 8]);
}

#[test]
fn a_multisampled_target_is_resolved_entire_for_a_placed_copy_of_part_of_it() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = green_multisampled(&gpu);
    let [left, _] = halves(4, 2);
    let whole = Rect { x: 0, y: 0, width: 4, height: 2 };
    let in_place = Placement { area: area(0.0, 0.0, 4.0, 2.0), clip: whole };

    draw.draw_copy_placed(&gpu.context, &target, left, &[in_place]).expect("the placed copy");

    assert_eq!(resolved_pixels(&gpu, &target), [GREEN; 8]);
}

#[test]
fn a_copy_draw_into_a_texture_that_cannot_be_a_target_names_the_view_and_changes_nothing() {
    let gpu = warp();
    let mut draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let sample_only = texture(&gpu, 2, 2, D3D11_BIND_SHADER_RESOURCE.0, &[RED, BLUE, RED, BLUE]);

    let error = draw
        .draw_copy(&gpu.context, &sample_only, &halves(2, 2))
        .expect_err("no view to draw into");

    assert_eq!(error.step, LayerStep::TargetView);
    assert_eq!(pixels(&gpu, &sample_only), [RED, BLUE, RED, BLUE]);
}

fn area(x: f32, y: f32, width: f32, height: f32) -> Area {
    Area { x, y, width, height }
}

#[test]
fn a_placed_layer_that_crosses_its_clip_draws_only_inside_the_clip() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = filled(&gpu, 8, 2, BLUE);
    let image = draw.image(4, 1, &[RED, GREEN, WHITE, WHITE]).expect("the layer texture");
    let [left, _] = halves(8, 2);
    // Columns 2 to 5, which cross the left half's edge at column 4.
    let crossing = Placement { area: area(2.0, 0.0, 4.0, 2.0), clip: left };

    draw.draw_placed(&gpu.context, &target, &image, &[crossing]).expect("the placed draw");

    let row = [BLUE, BLUE, RED, GREEN, BLUE, BLUE, BLUE, BLUE];
    assert_eq!(pixels(&gpu, &target), [row, row].as_flattened());
}

#[test]
fn a_placed_layer_can_start_left_of_the_target() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");
    let target = filled(&gpu, 4, 1, BLUE);
    let image = draw.image(4, 1, &[RED, GREEN, WHITE, RED]).expect("the layer texture");
    let whole = Rect { x: 0, y: 0, width: 4, height: 1 };
    let placed = Placement { area: area(-2.0, 0.0, 4.0, 1.0), clip: whole };

    draw.draw_placed(&gpu.context, &target, &image, &[placed]).expect("the placed draw");

    assert_eq!(pixels(&gpu, &target), [WHITE, RED, BLUE, BLUE]);
}

#[test]
fn an_image_needs_one_pixel_per_texel() {
    let gpu = warp();
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");

    let short = draw.image(2, 2, &[RED; 3]).expect_err("three pixels for four texels");
    let empty = draw.image(0, 2, &[]).expect_err("no texel");

    assert_eq!(short.step, LayerStep::Image);
    assert_eq!(empty.step, LayerStep::Image);
}

#[test]
fn the_draw_belongs_to_the_device_that_created_it() {
    let (gpu, other) = (warp(), warp());
    let draw = LayerDraw::create(&gpu.device).expect("create the layer draw");

    assert!(draw.belongs_to(&gpu.device));
    assert!(!draw.belongs_to(&other.device));
}

#[test]
fn a_layers_aspect_is_its_width_over_its_height() {
    let gpu = warp();

    let near = |width, height, expected: f32| {
        let aspect = layer_aspect(&filled(&gpu, width, height, [0; 4]));
        assert!(aspect.is_some_and(|aspect| (aspect - expected).abs() < 1e-6), "{aspect:?}");
    };

    near(64, 32, 2.0);
    near(16, 9, 16.0 / 9.0);
}
