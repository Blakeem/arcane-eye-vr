//! The cursor depth in the UI layer's frames: a test game whose UI texture has a clear lower half
//! and whose distance texture is near under the UI's upper half and far under the pointer, read
//! back from a WARP back buffer.

use std::time::Instant;

use game_bindings::state::arm_alloc_guard;
use gfx::nearest::{NearestError, NearestStep};
use gfx::pointer::{PointerError, PointerStep};
use stereo_core::eye::EyeScale;
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_DEVICE_HUNG;

use super::cursor::{ALL_BLACK, aim};
use super::ui_depth::{
    BLACK, FAR_M, MAX_FRAMES, NEAR_SHARE, Scene, UI_WIDTH, at, input, later, ui_pixels,
};
use super::{GREEN, HEIGHT, RED, UiLayer, WIDTH, depth, input_frame, running};
use crate::eye_view::Layout;
use crate::logged;
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, distance_rows, fill_back_buffer, flush};
use crate::raw::test_support::{TestCursor, TestSwapchain};
use crate::ui_layer::Target;
use crate::ui_layer::cursor::Aim;
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::decide::{Draw, Moves, Step, Write, depth_shift};
use crate::ui_layer::test_support::target_parts;
use crate::ui_layer::ui_depth::{CursorReport, DepthInput, Over};

const BLUE: Pixel = [0, 0, 255, 255];
const CLEAR: Pixel = [0, 0, 0, 0];

/// The distance texture's width: two halves of 256 texels, so the pointer read's march visits the
/// shares 11/256 down to 1/256 up to its reach of 0.045 at the 5 percent depth offset.
const DISTANCE_WIDTH: u32 = 512;

/// A scene 9000 m away as a share of the test camera's far plane, past the ray at every share.
const FAR_SHARE: f32 = 0.9;

/// The UI texture of the UI depth's tests with a clear lower row, where the scene shows.
fn clear_below() -> Vec<Pixel> {
    let mut pixels = ui_pixels();
    if let Some(lower) = pixels.get_mut(UI_WIDTH as usize..) {
        lower.fill(CLEAR);
    }
    pixels
}

/// A scene 0.5 m away in the distance rows the nearest read takes under the UI's upper row, and
/// 9000 m away in the rows the pointer read takes under a pointer in the lower half of the frame.
fn scene(test: &TestSwapchain) -> Scene {
    scene_below(test, FAR_SHARE)
}

/// A scene 0.5 m away under the UI's upper row, and `below` shares of the far plane away under a
/// pointer in the lower half of the frame.
fn scene_below(test: &TestSwapchain, below: f32) -> Scene {
    let rows = [NEAR_SHARE, NEAR_SHARE, below, below];
    Scene::showing(test, &clear_below(), distance_rows(test, DISTANCE_WIDTH, &rows))
}

/// The step of a side by side frame that moves the right half 2 columns, then draws the UI.
fn move_then_ui() -> Step {
    let shift = depth_shift(WIDTH, HEIGHT, depth(5.0)).expect("a 2-column move");
    Step {
        draw: Draw::MoveThenUi(shift),
        write: Write::Keep,
        windows_cursor: WindowsCursor::Hidden,
    }
}

/// Draws one frame of `layer` with the cursor of `aim` into `test`'s back buffer.
fn draw(layer: &mut UiLayer, test: &TestSwapchain, aim: Aim, input: &DepthInput) {
    let (size, devices, window) = target_parts(test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };
    layer.draw(move_then_ui(), Some(aim), &target, input).expect("the frame draws");
}

/// Draws frames of `layer` from `start` on until its UI depth moves the UI and the newest pointer
/// read's result is over the scene, and returns the frame that met it.
fn frames_until_scene(layer: &mut UiLayer, test: &TestSwapchain, aim: Aim, start: Instant) -> u32 {
    for frame in 0..u32::try_from(MAX_FRAMES).expect("a test frame count") {
        draw(layer, test, aim, &input(start, frame));
        if layer.depth_report().columns > 0 && layer.cursor_report().over == Over::Scene {
            return frame;
        }
        flush(test);
    }
    panic!("no pointer read result over the scene in {MAX_FRAMES} frames");
}

/// The back buffer's pixels after one frame of `layer` drawn over a blue back buffer.
fn over_blue(
    layer: &mut UiLayer,
    test: &TestSwapchain,
    aim: Aim,
    input: &DepthInput,
) -> Vec<Pixel> {
    fill_back_buffer(test, &vec![BLUE; (WIDTH * HEIGHT) as usize]);
    draw(layer, test, aim, input);
    back_buffer_pixels(test)
}

// A pointer at column 10 of 64 puts each half's cursor at columns 5 to 20 and 37 to 52 with no
// move, one at column 50 at columns 25 to 40 and 57 to 72, each clipped to its half. Each cursor
// starts at the pointer's row 28, below the rows the UI's two rows blend in. The upper UI row covers
// the back buffer's rows 0 to 15 with red in columns 10 and 42 with no move.

/// The pointer's row, where the UI is clear and the scene is far.
const POINTER_ROW: i32 = 28;

/// A back buffer row the cursor covers.
const CURSOR_ROW: usize = 30;

/// A pointer row over the UI's opaque upper row, at column 10 a green texel.
const UI_ROW: i32 = 4;

/// A wall 1.8 m away as a share of the test camera's far plane. The ray reaches it 5 texels right
/// of the pointer at the full eye distance, and 3 texels right at the eye scale of the scene 0.5 m
/// away.
const WALL_SHARE: f32 = 1.8 / FAR_M;

/// The frames after which the newest pointer result comes from a read started in them: one more
/// than the reads that can wait for the GPU at once.
const FRESH_FRAMES: u32 = 4;

#[test]
fn a_cursor_over_the_scene_sits_at_its_depth_in_each_half_while_the_ui_keeps_its_columns() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene(&test);
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let (near_left, near_right) =
        (aim(&test_cursor, 10, POINTER_ROW), aim(&test_cursor, 50, POINTER_ROW));
    let start = Instant::now();
    let frames = frames_until_scene(&mut layer, &test, near_left, start);

    let left_side = over_blue(&mut layer, &test, near_left, &input(start, later(frames, 1)));
    let right_side = over_blue(&mut layer, &test, near_right, &input(start, later(frames, 2)));

    // The far scene gives the hit 0: no move in the left half, and the depth offset's 1.6 columns,
    // rounded, in the right half.
    let moves = Moves { left: 0, right: 2 };
    assert_eq!(layer.cursor_report(), CursorReport { over: Over::Scene, moves });
    assert_eq!(at(&left_side, 2, [10, 11]), [GREEN, RED], "the left half's UI moves 1 right");
    assert_eq!(at(&left_side, 2, [41, 42]), [RED, GREEN], "the right half's UI moves 1 left");
    let left_cursor = at(&left_side, CURSOR_ROW, [4, 5, 20, 21]);
    assert_eq!(left_cursor, [BLUE, BLACK, BLACK, BLUE], "the left half's cursor stays");
    let right_cursor = at(&left_side, CURSOR_ROW, [38, 39, 54, 55]);
    assert_eq!(right_cursor, [BLUE, BLACK, BLACK, BLUE], "the right half's cursor moves 2 right");
    // Columns 32 and 33 are the move's black columns.
    let left_edge = at(&right_side, CURSOR_ROW, [24, 25, 31, 34]);
    assert_eq!(left_edge, [BLUE, BLACK, BLACK, BLUE], "the left cursor ends at its half's edge");
    let right_edge = at(&right_side, CURSOR_ROW, [58, 59, 63]);
    assert_eq!(right_edge, [BLUE, BLACK, BLACK], "the right cursor ends at the frame's edge");
}

// A WARP device offers no failure of the read's create, draw or map calls that leaves the device
// usable, so the test hands the cursor depth the error a failed map returns.
#[test]
fn a_failed_pointer_read_moves_the_cursor_with_the_ui_and_leaves_the_ui_depth_running() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene(&test);
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let pointer = aim(&test_cursor, 10, POINTER_ROW);
    let start = Instant::now();
    let frames = frames_until_scene(&mut layer, &test, pointer, start);
    let failed = PointerError { step: PointerStep::Map, code: DXGI_ERROR_DEVICE_HUNG };

    // Each frame's draw returns no fault, which would switch the UI layer off.
    let (pixels, log) = logged("cursor-depth-read-failed", || {
        layer.depth.cursor.stop(failed);
        for frame in 1..5 {
            draw(&mut layer, &test, pointer, &input(start, later(frames, frame)));
            flush(&test);
        }
        over_blue(&mut layer, &test, pointer, &input(start, later(frames, 5)))
    });

    assert_eq!(layer.depth_report().columns, 1, "the UI depth runs on");
    let moves = Moves { left: 1, right: -1 };
    assert_eq!(layer.cursor_report(), CursorReport { over: Over::NoResult, moves });
    let lines: Vec<&str> = log.lines().filter(|line| line.contains("read failed")).collect();
    let [line] = lines.as_slice() else { panic!("one line for the session: {log}") };
    assert!(line.contains("mapping a staging texture of the pointer read"), "{line}");
    let left_cursor = at(&pixels, CURSOR_ROW, [5, 6, 21, 22]);
    assert_eq!(left_cursor, [BLUE, BLACK, BLACK, BLUE], "the left half's cursor moves 1 right");
    let right_cursor = at(&pixels, CURSOR_ROW, [35, 36, 51, 52]);
    assert_eq!(right_cursor, [BLUE, BLACK, BLACK, BLUE], "the right half's cursor moves 1 left");
}

// A WARP device offers no failure of the read's create, draw or map calls that leaves the device
// usable, so the test hands the UI depth the error a failed map returns.
#[test]
fn a_failed_nearest_read_leaves_the_pointer_read_running() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene(&test);
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let over_scene = aim(&test_cursor, 10, POINTER_ROW);
    let over_ui = aim(&test_cursor, 10, UI_ROW);
    let start = Instant::now();
    let frames = frames_until_scene(&mut layer, &test, over_scene, start);
    let failed = NearestError { step: NearestStep::Map, code: DXGI_ERROR_DEVICE_HUNG };

    layer.depth.stop(failed);
    for frame in 1..=u32::try_from(MAX_FRAMES).expect("a test frame count") {
        draw(&mut layer, &test, over_ui, &input(start, later(frames, frame)));
        if layer.cursor_report().over == Over::Ui {
            break;
        }
        flush(&test);
    }

    assert_eq!(layer.depth_report().columns, 0, "the UI depth stopped");
    let report = CursorReport { over: Over::Ui, moves: Moves::ZERO };
    assert_eq!(layer.cursor_report(), report, "a pointer read after the stop");
}

#[test]
fn a_pointer_read_started_before_a_flat_frame_is_not_followed_after_it() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene(&test);
    let health = running();
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let pointer = aim(&test_cursor, 10, POINTER_ROW);
    let start = Instant::now();
    let frames = frames_until_scene(&mut layer, &test, pointer, start);
    // The direct draws named no next layout, so this frame runs in a new layer's flat layout.
    let flat = input(start, later(frames, 1));
    input_frame(&mut layer, &health, &test.swapchain, Layout::MainOnly, &flat);
    for _ in 0..MAX_FRAMES {
        flush(&test);
    }

    draw(&mut layer, &test, pointer, &input(start, later(frames, 2)));

    assert_eq!(layer.depth_report().columns, 0, "the flat frame reset the UI's columns");
    let report = CursorReport { over: Over::NoResult, moves: Moves::ZERO };
    assert_eq!(layer.cursor_report(), report);
}

#[test]
fn the_pointer_read_probes_at_the_eye_scale_the_eye_cameras_rendered_the_frame_at() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene_below(&test, WALL_SHARE);
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let pointer = aim(&test_cursor, 10, POINTER_ROW);
    let start = Instant::now();
    let frames = frames_until_scene(&mut layer, &test, pointer, start);
    // No Present call hands the eye scale over, so the eye cameras render at the full eye distance
    // while the UI depth's eye scale in use falls.
    for frame in 1..=FRESH_FRAMES {
        draw(&mut layer, &test, pointer, &input(start, later(frames, frame)));
        flush(&test);
    }
    let at_full = layer.cursor_report();
    let in_use = layer.depth_report().eye_scale;

    // Each Present call hands the eye scale over before the frame's draw, as present.rs does.
    let mut frame = later(frames, FRESH_FRAMES);
    for _ in 0..MAX_FRAMES {
        frame = later(frame, 1);
        layer.hand_eye_scale();
        draw(&mut layer, &test, pointer, &input(start, frame));
        if layer.cursor_report() != at_full {
            break;
        }
        flush(&test);
    }

    assert!(in_use < EyeScale::FULL, "{in_use:?}");
    // 5/256 of a 32-column half is 0.63 columns, and the move's 2 columns less those are 1.38.
    let full = CursorReport { over: Over::Scene, moves: Moves { left: 1, right: 1 } };
    assert_eq!(at_full, full, "at the full eye distance");
    // 3/256 of a 32-column half is 0.38 columns, and the move's 2 columns less those are 1.63.
    let fallen = CursorReport { over: Over::Scene, moves: Moves { left: 0, right: 2 } };
    assert_eq!(layer.cursor_report(), fallen, "at the eye scale handed over");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_frame_that_takes_a_pointer_result_and_starts_a_pointer_read_makes_no_heap_allocation() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let scene = scene(&test);
    let mut layer = scene.game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let pointer = aim(&test_cursor, 10, 20);
    let start = Instant::now();
    // The first frame makes the layer draw, the cursor's texture and both reads.
    draw(&mut layer, &test, pointer, &input(start, 0));
    flush(&test);

    let mut count = 0;
    for frame in 1..=u32::try_from(MAX_FRAMES).expect("a test frame count") {
        let guard = arm_alloc_guard().expect("arm the allocation guard");
        draw(&mut layer, &test, pointer, &input(start, frame));
        count += guard.count();
        drop(guard);
        if layer.cursor_report().over == Over::Scene {
            break;
        }
        flush(&test);
    }

    assert_eq!(count, 0, "a side by side frame with the cursor depth allocated");
    assert_eq!(layer.cursor_report().over, Over::Scene, "a guarded frame took a result");
}
