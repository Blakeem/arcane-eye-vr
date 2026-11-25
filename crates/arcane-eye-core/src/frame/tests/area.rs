//! The interface area of the frames of the launch mode `vr`: a world frame's of its menu scale and
//! frame aspect, a screen frame's the entire half, and the `ui depth` line's `ui_w` and `ui_h`.

use std::time::Instant;

use diag::health::{Feature, Health};
use game_bindings::camera::Camera;
use game_bindings::render_view::RenderView;
use game_mem::GameAddr;
use ipc_layout::launch_token::Mode;
use settings::dev::{UiScalePercent, VrFlags};
use settings::player::StereoSettings;

use super::{call_times, log_lines};
use crate::eye_camera::ProxyFill;
use crate::eye_view::Layout;
use crate::frame::{FrameInput, FrameState, on_frame, record_interval};
use crate::raw::test_support::{TestSwapchain, test_view_names};
use crate::ui_layer::test_support::{Game, RED};
use crate::viewer::ViewerLink;
use crate::vr_view::{ViewKind, VrView};

/// A main view camera whose lens has the aspect 1, half the aspect of the 64 by 32 UI texture.
const SQUARE: Camera = Camera {
    position: [0.0; 3],
    forward: [0.0, 0.0, 1.0],
    fov_y_rad: 1.2,
    aspect: 1.0,
    near: 0.1,
    far: 1000.0,
};

/// A frame state of the test `test` in the mode `vr` whose dev flag `vr.ui_scale_percent` is 50 and
/// whose UI layer draws the UI texture of `game`.
fn scaled_state(test: &str, game: &Game) -> FrameState {
    let ui_scale_percent = UiScalePercent::try_from(50).expect("a scale in range");
    let vr = VrView::new(Mode::Vr, VrFlags { ui_scale_percent, ..VrFlags::default() });
    let viewer = ViewerLink::new(test_view_names(test));
    FrameState { viewer, vr, ui_layer: Some(game.layer()), ..FrameState::default() }
}

/// The side by side layout of a frame whose main view and eye view lie at test addresses.
const SIDE_BY_SIDE: Layout = Layout::SideBySide {
    main: RenderView::new(GameAddr::new(0x1000)),
    eye: RenderView::new(GameAddr::new(0x2000)),
};

/// One frame of `test` whose main view proxy was filled as `fill` says, with the camera
/// [`SQUARE`], after which the eye view feature read a side by side layout.
fn scaled_frame(state: &mut FrameState, test: &TestSwapchain, health: &Health, fill: ProxyFill) {
    let stereo = StereoSettings::default();
    let next = SIDE_BY_SIDE;
    let input = FrameInput { health, next, stereo, camera: Some(SQUARE), fill };
    test.with(|address, swapchain| on_frame(state, address, swapchain, input, Instant::now()))
        .expect("a swapchain");
}

#[test]
fn a_world_frame_takes_the_area_of_its_scale_and_frame_aspect_and_a_screen_frame_the_entire_half() {
    let test = TestSwapchain::new(64, 32);
    let game = Game::showing(&test, 64, &[RED; 64 * 32]);
    let health = Health::new();
    health.install_ok(Feature::PresentHook);
    health.install_ok(Feature::UiLayer);
    let mut state = scaled_state("vr-area", &game);
    // The game drew its UI into this flat frame, which clears the byte, so the core draws the UI
    // into each side by side frame next.
    scaled_frame(&mut state, &test, &health, ProxyFill::NONE);
    let world = ProxyFill { kind: ViewKind::World, head_sequence: 2 };

    let lines = log_lines("vr-area", || {
        for fill in [world, ProxyFill::NONE] {
            for now in call_times() {
                scaled_frame(&mut state, &test, &health, fill);
                record_interval(&mut state, now);
            }
        }
    });

    let depth: Vec<&String> = lines.iter().filter(|line| line.contains("\"ui depth\"")).collect();
    let [world_line, screen_line] = depth.as_slice() else {
        panic!("two ui depth lines: {lines:#?}")
    };
    // The scale 0.5 across and 0.5 * 1 / 2 down, so the UI keeps its shape in the square lens.
    assert!(world_line.ends_with(" ui_w=0.5 ui_h=0.25"), "{world_line}");
    assert!(screen_line.ends_with(" ui_w=1.0 ui_h=1.0"), "{screen_line}");
}
