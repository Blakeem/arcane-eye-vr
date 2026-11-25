//! The interface area of a frame, read from the back buffer of a test game's side by side frames.

use game_bindings::camera::Camera;
use ipc_layout::viewer_header::depth_map::InterfaceArea;

use super::{HEIGHT, RED, SIDE_BY_SIDE, WIDTH, drawing, input_frame, running};
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, fill_back_buffer};
use crate::raw::test_support::TestSwapchain;
use crate::ui_layer::depth_offset::DepthOffset;
use crate::ui_layer::test_support::{Game, no_camera};
use crate::ui_layer::ui_depth::DepthInput;
use crate::vr_view::ViewKind;

const BLUE: Pixel = [0, 0, 255, 255];

/// A main view camera whose lens has the aspect 2, the aspect of the 64 by 32 UI texture.
const CAMERA: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 0.96,
    aspect: 2.0,
    near: 0.1,
    far: 1000.0,
};

/// The input of a frame of the view kind `kind` at the menu scale 0.5, with no depth offset.
fn input(kind: ViewKind) -> DepthInput {
    DepthInput { offset: DepthOffset::NONE, camera: Some(CAMERA), kind, scale: 0.5, ..no_camera() }
}

/// The pixels at the columns 2, 16, 34 and 48 of the rows 2 and 16 after a side by side frame of
/// `kind` over a blue picture, with the UI texture red across its entire 64 by 32 texels, and the
/// interface area the frame reported.
fn drawn(kind: ViewKind) -> ([Pixel; 8], InterfaceArea) {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::showing(&test, WIDTH, &[RED; 64 * 32]);
    let health = running();
    let mut layer = drawing(&game, &health, &test);
    let columns = WIDTH as usize;
    fill_back_buffer(&test, &[BLUE; 64 * 32]);
    input_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, &input(kind));
    let pixels = back_buffer_pixels(&test);
    let at = |x: usize, y: usize| {
        let index = y.checked_mul(columns).and_then(|row| row.checked_add(x));
        pixels[index.expect("a pixel index")]
    };
    let shown =
        [at(2, 2), at(16, 2), at(34, 2), at(48, 2), at(2, 16), at(16, 16), at(34, 16), at(48, 16)];
    (shown, layer.depth_report().area)
}

#[test]
fn a_world_frame_draws_the_ui_into_the_area_of_its_scale_and_leaves_the_scene_around_it() {
    let (shown, area) = drawn(ViewKind::World);

    // The area is 16 of each half's 32 columns and rows, around the middle 16, 16.
    assert_eq!(shown, [BLUE, BLUE, BLUE, BLUE, BLUE, RED, BLUE, RED]);
    assert_eq!(area, InterfaceArea { width: 0.5, height: 0.5 });
}

#[test]
fn a_screen_frame_draws_the_ui_over_the_entire_half_whatever_the_scale() {
    let (shown, area) = drawn(ViewKind::Screen);

    assert_eq!(shown, [RED; 8]);
    assert_eq!(area, InterfaceArea::FULL);
}
