//! The mesh draw against a WARP device, with the shaders the build script compiled, into targets of
//! 64 by 64 pixels in the linear `R8G8B8A8_UNORM` format.

mod resources;

use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

use super::layer::pixels;
use super::{Gpu, desc, warp};
use crate::mesh::tests::{perspective, product, scale, translation, turn_y};
use crate::mesh::{Item, Mesh, MeshDraw, MeshTarget, MeshTexture, Vertex};

const SIDE: u16 = 64;
const TRANSPARENT: [u8; 4] = [0; 4];
const WHITE: [f32; 3] = [1.0; 3];

/// The draw's view: the viewer at the origin looking down -Z, with a half width of 1 at depth 1.
fn view() -> [f32; 16] {
    perspective(0.1, 10.0)
}

/// An `R8G8B8A8_UNORM` texture of `width` by `height` pixels that can be drawn into.
fn texture(gpu: &Gpu, width: u16, height: u16) -> ID3D11Texture2D {
    let desc = desc(width.into(), height.into(), 1);
    let mut texture = None;
    // SAFETY: the descriptor and the out slot are live for the call.
    unsafe { gpu.device.CreateTexture2D(&raw const desc, None, Some(&raw mut texture)) }
        .expect("create the target texture");
    texture.expect("the target texture")
}

/// A texture of `width` by `height` pixels and the draw's view of it.
fn target(gpu: &Gpu, draw: &MeshDraw, width: u16, height: u16) -> (ID3D11Texture2D, MeshTarget) {
    let texture = texture(gpu, width, height);
    let view = draw.target(&texture, DXGI_FORMAT_R8G8B8A8_UNORM).expect("the target view");
    (texture, view)
}

/// A vertex at (x, y, 0) whose normal is +Z.
fn vertex(x: f32, y: f32, uv: [f32; 2]) -> Vertex {
    Vertex { position: [x, y, 0.0], normal: [0.0, 0.0, 1.0], uv }
}

/// A triangle of half size `half` around the origin that faces +Z, counterclockwise seen from +Z.
fn triangle(draw: &MeshDraw, half: f32) -> Mesh {
    let vertices = [
        vertex(-half, -half, [0.0; 2]),
        vertex(half, -half, [0.0; 2]),
        vertex(0.0, half, [0.0; 2]),
    ];
    draw.mesh(&vertices, &[0, 1, 2]).expect("the triangle mesh")
}

/// A square of half size `half` around the origin that faces +Z, with the texture's top left
/// corner at its top left corner.
fn square(draw: &MeshDraw, half: f32) -> Mesh {
    let vertices = [
        vertex(-half, -half, [0.0, 1.0]),
        vertex(half, -half, [1.0, 1.0]),
        vertex(half, half, [1.0, 0.0]),
        vertex(-half, half, [0.0, 0.0]),
    ];
    draw.mesh(&vertices, &[0, 1, 2, 2, 3, 0]).expect("the square mesh")
}

fn item(mesh: &Mesh, model: [f32; 16], factor: [f32; 3]) -> Item<'_> {
    Item { mesh, model, factor, texture: None }
}

/// The pixels of a target after one draw of `items` into it.
fn drawn(gpu: &Gpu, draw: &mut MeshDraw, items: &[Item<'_>]) -> Vec<[u8; 4]> {
    let (texture, target) = target(gpu, draw, SIDE, SIDE);
    draw.draw(&gpu.context, &target, &view(), items).expect("the draw");
    pixels(gpu, &texture)
}

/// The pixel at column `x` and row `y` of a target `SIDE` pixels wide.
fn at(pixels: &[[u8; 4]], x: usize, y: usize) -> [u8; 4] {
    let index = y.checked_mul(usize::from(SIDE)).and_then(|row| row.checked_add(x));
    *index.and_then(|index| pixels.get(index)).expect("a pixel of the target")
}

fn create(gpu: &Gpu) -> MeshDraw {
    MeshDraw::create(&gpu.device).expect("the built shaders load in a D3D11 device")
}

#[test]
fn a_triangle_over_the_middle_is_opaque_there_and_a_corner_stays_transparent_black() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let pixels = drawn(&gpu, &mut draw, &[item(&mesh, translation(0.0, 0.0, -1.0), WHITE)]);
    assert_eq!(at(&pixels, 32, 32)[3], 255, "the middle pixel");
    assert_eq!(at(&pixels, 0, 0), TRANSPARENT, "the corner pixel");
}

#[test]
fn the_nearer_of_two_overlapping_triangles_shows_in_both_draw_orders() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let near = item(&mesh, translation(0.0, 0.0, -1.0), [1.0, 0.0, 0.0]);
    let far =
        item(&mesh, product(&translation(0.0, 0.0, -2.0), &scale(3.0, 3.0, 1.0)), [0.0, 1.0, 0.0]);
    for (order, items) in [("near first", [near, far]), ("far first", [far, near])] {
        let middle = at(&drawn(&gpu, &mut draw, &items), 32, 32);
        assert!(middle[0] > 200 && middle[1] == 0, "{order}: {middle:?}");
    }
}

#[test]
fn a_triangle_facing_away_from_the_viewer_draws_no_pixel() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let away = product(&translation(0.0, 0.0, -1.0), &turn_y(180.0));
    let pixels = drawn(&gpu, &mut draw, &[item(&mesh, away, WHITE)]);
    assert!(pixels.iter().all(|&pixel| pixel == TRANSPARENT), "a back face drew");
}

#[test]
fn a_mirroring_model_matrix_keeps_the_front_drawn_and_lit() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = triangle(&draw, 0.5);
    let mirrored = product(&translation(0.0, 0.0, -1.0), &scale(-1.0, 1.0, 1.0));
    let pixels = drawn(&gpu, &mut draw, &[item(&mesh, mirrored, WHITE)]);
    assert!(at(&pixels, 32, 32)[0] > 250, "the mirrored front: {:?}", at(&pixels, 32, 32));
}

#[test]
fn a_textured_square_shows_each_texel_color_in_its_quarter() {
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 1.0);
    let texture: MeshTexture =
        draw.texture(&gpu.context, 2, 2, &[RED, GREEN, BLUE, YELLOW]).expect("the texture");
    let textured =
        Item { texture: Some(&texture), ..item(&mesh, translation(0.0, 0.0, -1.0), WHITE) };
    let pixels = drawn(&gpu, &mut draw, &[textured]);
    for (color, (x, y)) in
        [RED, GREEN, BLUE, YELLOW].into_iter().zip([(16, 16), (48, 16), (16, 48), (48, 48)])
    {
        let pixel = at(&pixels, x, y);
        for (channel, (shown, texel)) in pixel.iter().zip(color).take(3).enumerate() {
            let lit = if texel == 255 { *shown > 128 } else { *shown < 32 };
            assert!(lit, "pixel ({x}, {y}), channel {channel}: {pixel:?} against {color:?}");
        }
    }
}

#[test]
fn a_surface_turned_by_60_degrees_is_darker_by_the_ratio_of_the_light_rule() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let place = translation(0.0, 0.0, -1.0);
    let facing = at(&drawn(&gpu, &mut draw, &[item(&mesh, place, WHITE)]), 32, 32);
    let turned_model = product(&place, &turn_y(60.0));
    let turned = at(&drawn(&gpu, &mut draw, &[item(&mesh, turned_model, WHITE)]), 32, 32);
    let expected = (0.35 + 0.65) / (0.35 + 0.65 * 0.5);
    let ratio = f32::from(facing[0]) / f32::from(turned[0]);
    assert!((ratio / expected - 1.0).abs() < 0.03, "{facing:?} over {turned:?} is {ratio}");
}

#[test]
fn a_turned_surface_stretched_along_one_axis_is_lit_by_its_true_normal() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let place = translation(0.0, 0.0, -1.0);
    let facing = at(&drawn(&gpu, &mut draw, &[item(&mesh, place, WHITE)]), 32, 32);
    let stretch = 2.0_f32;
    let model = product(&place, &product(&scale(stretch, 1.0, 1.0), &turn_y(60.0)));
    let stretched = at(&drawn(&gpu, &mut draw, &[item(&mesh, model, WHITE)]), 32, 32);
    // The turned square spans +Y and (cos 60, 0, -sin 60). The stretch makes the second
    // (2 cos 60, 0, -sin 60), so the true normal is (sin 60, 0, 2 cos 60). Transforming the normal
    // by the model matrix itself would give (2 sin 60, 0, cos 60) and a light of 0.53.
    let (sin, cos) = 60.0_f32.to_radians().sin_cos();
    let cosine = stretch * cos / sin.hypot(stretch * cos);
    let expected = (0.35 + 0.65) / 0.65_f32.mul_add(cosine, 0.35);
    let ratio = f32::from(facing[0]) / f32::from(stretched[0]);
    assert!((ratio / expected - 1.0).abs() < 0.03, "{facing:?} over {stretched:?} is {ratio}");
}

#[test]
fn a_model_matrix_that_moves_half_the_view_width_moves_the_picture_half_the_target_width() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let columns = |pixels: &[[u8; 4]]| -> Vec<usize> {
        (0..usize::from(SIDE)).filter(|&x| at(pixels, x, 32)[3] == 255).collect()
    };
    let left = translation(-0.5, 0.0, -1.0);
    let moved = product(&translation(1.0, 0.0, 0.0), &left);
    let before = columns(&drawn(&gpu, &mut draw, &[item(&mesh, left, WHITE)]));
    let after = columns(&drawn(&gpu, &mut draw, &[item(&mesh, moved, WHITE)]));
    assert_eq!(before, (8..24).collect::<Vec<_>>(), "the square's columns before the move");
    assert_eq!(
        after,
        before.iter().map(|x| x.checked_add(32).expect("a column")).collect::<Vec<_>>()
    );
}

#[test]
fn a_target_of_a_new_size_draws_after_a_target_of_another_size() {
    let gpu = warp();
    let mut draw = create(&gpu);
    let mesh = square(&draw, 0.25);
    let items = [item(&mesh, translation(0.0, 0.0, -1.0), WHITE)];
    let (small_texture, small) = target(&gpu, &draw, 32, 16);
    assert_eq!(at(&drawn(&gpu, &mut draw, &items), 32, 32)[3], 255, "the first size");
    draw.draw(&gpu.context, &small, &view(), &items).expect("the draw at a new size");
    let small_pixels = pixels(&gpu, &small_texture);
    // Row 8 and column 16 of a target 32 pixels wide.
    let middle = small_pixels.get(272).expect("the small target's middle");
    assert_eq!(middle[3], 255, "the new size");
    assert_eq!(small_pixels.first(), Some(&TRANSPARENT), "the new size's corner");
    assert_eq!(at(&drawn(&gpu, &mut draw, &items), 32, 32)[3], 255, "the first size again");
}
