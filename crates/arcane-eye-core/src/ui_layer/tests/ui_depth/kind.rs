//! The eye scale across a change from screen frames to world frames.

use std::time::Instant;

use stereo_core::eye::EyeScale;

use super::super::{HEIGHT, SIDE_BY_SIDE, WIDTH, input_frame, running};
use super::{NEAR_SHARE, Scene, crossed_layer, input, later};
use crate::raw::test_pixels::distances_of;
use crate::raw::test_support::TestSwapchain;
use crate::ui_layer::depth_offset::DepthOffset;
use crate::ui_layer::ui_depth::DepthInput;
use crate::vr_view::ViewKind;

#[test]
fn after_a_shrunk_screen_phase_a_world_kind_hands_and_uses_the_full_eye_distance() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let frames = crossed_layer(&mut layer, &health, &test, start);
    let shrunk = layer.depth_report().eye_scale;

    let screen_call = layer.hand_eye_scale(ViewKind::Screen);
    let first_world_call = layer.hand_eye_scale(ViewKind::World);
    let last_screen_frame = layer.depth.rendered_eye_scale();
    layer.hand_eye_scale(ViewKind::World);
    let first_world_frame = layer.depth.rendered_eye_scale();
    let world = DepthInput {
        offset: DepthOffset::NONE,
        kind: ViewKind::World,
        ..input(start, later(frames, 1))
    };
    input_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, &world);
    let in_use = layer.depth_report().eye_scale;

    assert!(shrunk < EyeScale::FULL, "{shrunk:?}");
    assert_eq!(screen_call, shrunk, "a screen kind hands the eye scale in use");
    assert_eq!(first_world_call, EyeScale::FULL, "a world kind hands the full eye distance");
    assert_eq!(last_screen_frame, shrunk, "the frame of the last screen call");
    assert_eq!(first_world_frame, EyeScale::FULL, "the frame of the first world call");
    assert_eq!(in_use, EyeScale::FULL, "the eye scale in use of the first world frame");
}
