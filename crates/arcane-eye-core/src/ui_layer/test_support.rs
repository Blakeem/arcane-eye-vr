//! UI layer frames that draw only the cursor into a WARP back buffer, for the tests of this module
//! and of the frame handler.

use std::time::Instant;

use game_mem::GameAddr;
use ipc_layout::launch_token::Mode;
use settings::player::StereoSettings;

use super::cursor::Aim;
use super::cursor::counter::WindowsCursor;
use super::decide::{Draw, Step, Write};
use super::ui_depth::DepthInput;
use super::{DrawFault, Target, UiLayer};
use crate::raw::cursor::Window;
use crate::raw::test_support::TestSwapchain;
use crate::raw::{self, BackBuffer, Devices};

/// The pipeline global and the composite stage vtable of a layer that draws only the cursor. No
/// read goes to either address.
const UNREAD: GameAddr = GameAddr::new(0x7FF6_58C7_0000);

/// The back buffer, its device and immediate context, and the swapchain's output window.
pub(crate) fn target_parts(test: &TestSwapchain) -> (BackBuffer, Devices, Window) {
    let device = raw::device(&test.swapchain).expect("the swapchain's device");
    let context = raw::immediate_context(&device).expect("the immediate context");
    let (size, window) = raw::swapchain_desc(&test.swapchain).expect("the swapchain's description");
    (size, (device, context), window)
}

/// The UI depth's input of a frame with the default settings and no camera, which starts no read.
pub(crate) fn no_camera() -> DepthInput {
    let stereo = StereoSettings::default();
    DepthInput {
        stereo,
        eye_m: stereo.ipd_mm.meters(),
        camera: None,
        now: Instant::now(),
        mode: Mode::DesktopStereo,
    }
}

/// The draw of a frame of `layer` into `target` whose step draws `draw` and wants the Windows
/// cursor `want`, with the cursor of `aim` and no camera.
pub(crate) fn draw_step(
    layer: &mut UiLayer,
    draw: Draw,
    want: WindowsCursor,
    aim: Option<Aim>,
    target: &Target<'_>,
) -> Result<(), DrawFault> {
    let step = Step { draw, write: Write::Keep, windows_cursor: want };
    layer.draw(step, aim, target, &no_camera())
}

/// A frame of `layer` that draws only the cursor of `aim` into `test`'s back buffer, in which the
/// frame's step wants the Windows cursor `want`.
pub(crate) fn cursor_frame(
    layer: &mut UiLayer,
    test: &TestSwapchain,
    want: WindowsCursor,
    aim: Option<Aim>,
) {
    let (size, devices, window) = target_parts(test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    draw_step(layer, Draw::Nothing, want, aim, &target).expect("the frame draws");
}

/// A layer whose frames draw only the cursor, and that has drawn none.
pub(crate) fn idle_layer() -> UiLayer {
    UiLayer::new(UNREAD, UNREAD)
}

/// A layer whose frames draw only the cursor, after one such frame of `aim` into `test`'s back
/// buffer that hid the Windows cursor.
pub(crate) fn hidden_cursor_layer(test: &TestSwapchain, aim: Aim) -> UiLayer {
    let mut layer = idle_layer();
    cursor_frame(&mut layer, test, WindowsCursor::Hidden, Some(aim));
    layer
}
