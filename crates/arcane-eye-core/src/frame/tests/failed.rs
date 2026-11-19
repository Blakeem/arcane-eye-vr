//! Tests of the frame handler after a caught panic latched `present-hook` off: the eye camera's frame
//! ends and the UI layer stops each frame, until a panic stops that for the session.

use std::cell::Cell;

use super::log_lines;
use crate::frame::{FrameState, on_failed_present_call};
use crate::raw::test_support::{TestCursor, TestSwapchain, display_count};
use crate::ui_layer::cursor::Aim;
use crate::ui_layer::cursor::place::{Pointer, spot};
use crate::ui_layer::test_support::hidden_cursor_layer;
use crate::vr_view::VrView;

/// A frame state whose UI layer hid the Windows cursor under a drawn cursor in `test`'s back buffer.
fn hidden_cursor_state(test: &TestSwapchain, cursor: &TestCursor) -> FrameState {
    let pointer = Pointer { x: 10, y: 4, width: 64, height: 32 };
    let aim = Aim { handle: cursor.handle(), spot: spot(pointer).expect("a pointer inside") };
    FrameState::new(None, Some(hidden_cursor_layer(test, aim)), VrView::default())
}

#[test]
fn each_failed_present_call_ends_the_camera_frame_and_gives_the_counter_the_games_value_back() {
    let test = TestSwapchain::new(64, 32);
    let masks = [0_u8; 128];
    let cursor = TestCursor::monochrome([0, 0], &masks, &masks);
    let before = display_count();
    let mut state = hidden_cursor_state(&test, &cursor);
    let hidden = display_count();
    let camera_ends = Cell::new(0);

    for _ in 0..2 {
        on_failed_present_call(&mut state, || camera_ends.set(camera_ends.get() + 1));
    }

    assert_eq!(hidden, before - 1, "the drawn cursor hid the Windows cursor");
    assert_eq!(display_count(), before, "one counter call for two frames");
    assert_eq!(camera_ends.get(), 2, "each frame ends the eye camera's frame");
}

#[test]
fn a_panic_in_a_failed_present_call_stops_it_for_the_session_with_one_log_line() {
    let test = TestSwapchain::new(64, 32);
    let masks = [0_u8; 128];
    let cursor = TestCursor::monochrome([0, 0], &masks, &masks);
    let mut state = hidden_cursor_state(&test, &cursor);
    let hidden = display_count();
    let camera_ends = Cell::new(0);

    let lines = log_lines("failed-present-panic", || {
        on_failed_present_call(&mut state, || panic!("the eye camera's frame end panicked"));
        on_failed_present_call(&mut state, || camera_ends.set(camera_ends.get() + 1));
    });
    let after_panic = display_count();
    let layer = state.ui_layer.as_mut().expect("the UI layer");
    layer.release();

    assert_eq!(after_panic, hidden, "the state stays as the panic left it");
    assert_eq!(camera_ends.get(), 0, "a stopped give-back runs nothing");
    let stops: Vec<&String> = lines.iter().filter(|line| line.contains("panicked, so")).collect();
    let [line] = stops.as_slice() else { panic!("one stop line: {lines:#?}") };
    assert!(line.contains(" WARN "), "{line}");
}
