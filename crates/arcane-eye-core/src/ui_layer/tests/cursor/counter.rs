//! The cursor display counter of the thread that runs the UI layer: lowered under a drawn cursor,
//! and the game's value wherever the core draws none.

use std::thread;

use diag::health::Health;
use game_bindings::state::arm_alloc_guard;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use super::super::{Game, HEIGHT, SIDE_BY_SIDE, UiLayer, WIDTH, frame, running, stop};
use super::{ALL_BLACK, BLACK, aim, fill_blue};
use crate::logged;
use crate::raw::test_pixels::back_buffer_pixels;
use crate::raw::test_support::{TestCursor, TestSwapchain, display_count};
use crate::ui_layer::Target;
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::decide::Draw;
use crate::ui_layer::test_support::{cursor_frame, draw_step, target_parts};

#[test]
fn a_drawn_cursor_lowers_the_windows_thread_counter_once_and_a_stopped_layer_raises_it() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));
    let before = display_count();

    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    let first = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    let second = display_count();
    stop(&mut layer, &health, &test);
    let stopped = display_count();
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);

    assert_eq!([first, second], [before - 1, before - 1], "one call while the feature runs");
    assert_eq!(stopped, before, "the game's value after the feature stops");
    assert_eq!(display_count(), before, "one call after the feature stops");
}

#[test]
fn a_frame_with_no_pointer_in_the_client_area_shows_the_windows_cursor() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));
    let before = display_count();

    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    let hidden = display_count();
    // The test window is hidden, so the frame reads no pointer in its client area.
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
    let unaimed = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, None);

    assert_eq!(hidden, before - 1, "the drawn cursor hides the Windows cursor");
    assert_eq!(unaimed, before, "a frame whose pointer read finds no client area");
    assert_eq!(display_count(), before, "a frame with no aim, as for a pointer on a border");
}

#[test]
fn a_skipped_cursor_shows_the_windows_cursor_until_its_handle_changes() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let destroyed = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let stale = Some(aim(&destroyed, 10, 4));
    drop(destroyed);
    let before = display_count();

    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, Some(aim(&test_cursor, 10, 4)));
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, stale);
    let failed = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, stale);
    let held = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, Some(aim(&test_cursor, 10, 4)));

    assert_eq!(failed, before, "the frame whose cursor read failed");
    assert_eq!(held, before, "the frame that skips the held handle");
    assert_eq!(display_count(), before - 1, "the changed handle is drawn again");
}

#[test]
fn a_drawn_cursor_in_a_frame_whose_step_shows_the_windows_cursor_raises_a_lowered_counter() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));
    let before = display_count();

    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    fill_blue(&test);
    cursor_frame(&mut layer, &test, WindowsCursor::Shown, aimed);
    let shown = display_count();
    // Column 10 of 64 puts the left half's cursor at column 5.
    let drawn = back_buffer_pixels(&test)[4 * 64 + 5];
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);

    assert_eq!(shown, before, "the game's UI across both halves takes the Windows cursor");
    assert_eq!(drawn, BLACK, "the cursor is drawn in the frame that shows the Windows cursor");
    assert_eq!(display_count(), before - 1, "the next frame with the UI in each half");
}

#[test]
fn a_frame_the_frame_handler_skips_after_a_hidden_frame_gives_the_counter_the_games_value_back() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));
    let before = display_count();

    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    layer.on_skipped_frame();
    let skipped = display_count();
    layer.on_skipped_frame();
    let second_skip = display_count();
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);

    assert_eq!([skipped, second_skip], [before, before], "one call for two skipped frames");
    assert_eq!(display_count(), before - 1, "the next drawn cursor hides it again");
}

#[test]
#[cfg_attr(not(debug_assertions), ignore = "the counting allocator exists only in debug builds")]
fn a_cursor_frame_after_a_skipped_frame_makes_no_heap_allocation() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, aimed);
    layer.on_skipped_frame();
    let (size, devices, window) = target_parts(&test);
    let target = Target { swapchain: &test.swapchain, devices: &devices, size, window };

    let guard = arm_alloc_guard().expect("arm the allocation guard");
    let drawn = draw_step(&mut layer, Draw::Nothing, WindowsCursor::Hidden, aimed, &target);
    let count = guard.count();
    drop(guard);

    assert_eq!(drawn, Ok(()));
    assert_eq!(count, 0, "the skipped frame kept the cursor's texture");
}

/// The log of three side by side frames of `layer` on the calling thread, and the thread's cursor
/// display counter before and after them.
fn three_frames(
    layer: &mut UiLayer,
    health: &Health,
    swapchain: &IDXGISwapChain,
) -> (String, [i32; 2]) {
    let before = display_count();
    let ((), log) = logged("cursor-other-thread", || {
        for _ in 0..3 {
            frame(layer, health, swapchain, SIDE_BY_SIDE);
        }
    });
    (log, [before, display_count()])
}

#[test]
fn a_layer_on_a_thread_that_does_not_own_the_window_leaves_the_counters_and_logs_both_threads() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    let (swapchain, before) = (&test.swapchain, display_count());

    let (log, counts) = thread::scope(|scope| {
        let frames = scope.spawn(|| three_frames(&mut layer, &health, swapchain));
        frames.join().expect("the frame thread ends")
    });

    assert_eq!(counts[0], counts[1], "the frame thread's counter");
    assert_eq!(display_count(), before, "the window thread's counter");
    let lines: Vec<&str> = log.lines().filter(|line| line.contains("another thread")).collect();
    let [line] = lines.as_slice() else { panic!("one line for three frames: {log}") };
    assert!(line.contains("caller=") && line.contains("window_thread="), "{line}");
}
