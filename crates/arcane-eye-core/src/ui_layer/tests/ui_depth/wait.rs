//! The forward wait across a frame that starts no read of the distance texture.

use std::time::Instant;

use super::super::{HEIGHT, SIDE_BY_SIDE, WIDTH, input_frame, running};
use super::{NEAR_SHARE, Scene, frames_until, input, later};
use crate::raw::test_pixels::{distances_of, flush};
use crate::raw::test_support::TestSwapchain;
use crate::ui_layer::ui_depth::{DepthInput, DepthReport};

/// The 16 ms frames of the default 400 ms forward wait.
const WAIT_FRAMES: u32 = 25;

/// The frames from the frame with no camera to the next frame with one, 800 ms in all.
const GAP_FRAMES: u32 = 50;

#[test]
fn a_frame_with_no_camera_makes_a_nearer_scene_wait_the_entire_wait_time_after_it() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = Scene::new(&test, distances_of(&test, 8, 4, NEAR_SHARE));
    let health = running();
    let mut layer = scene.game.layer();
    let start = Instant::now();
    let measured = |report: &DepthReport| report.nearest.is_some();
    let first = frames_until(&mut layer, &health, &test, (start, 0), measured);
    let waiting = layer.depth_report().columns;
    flush(&test);
    let unseen = later(first, 1);
    let no_camera = DepthInput { camera: None, ..input(start, unseen) };
    input_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, &no_camera);
    flush(&test);
    let seen = later(unseen, GAP_FRAMES);

    let forward =
        frames_until(&mut layer, &health, &test, (start, seen), |report| report.columns > 0);

    assert_eq!(waiting, 0, "the first result of the near scene starts the wait");
    assert!(
        forward >= later(seen, WAIT_FRAMES),
        "the UI came forward at frame {forward}, before 400 ms after the frame {seen}"
    );
}
