//! The drawn cursor after the swapchain's device changes.

use super::super::{Game, HEIGHT, WIDTH};
use super::{ALL_BLACK, BLACK, BLUE, aim, fill_blue};
use crate::raw::test_pixels::back_buffer_pixels;
use crate::raw::test_support::{TestCursor, TestSwapchain};
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::test_support::cursor_frame;

#[test]
fn a_cursor_drawn_on_one_device_is_drawn_again_on_the_next_device_with_the_same_handle() {
    let (first, second) = (TestSwapchain::new(WIDTH, HEIGHT), TestSwapchain::new(WIDTH, HEIGHT));
    fill_blue(&second);
    let game = Game::new(&first);
    let mut layer = game.layer();
    let test_cursor = TestCursor::monochrome([0, 0], &ALL_BLACK.0, &ALL_BLACK.1);
    let aimed = Some(aim(&test_cursor, 10, 4));

    for test in [&first, &second] {
        cursor_frame(&mut layer, test, WindowsCursor::Hidden, aimed);
    }

    let pixels = back_buffer_pixels(&second);
    // Column 10 of 64 puts the left half's cursor at column 5.
    assert_eq!(pixels[4 * 64 + 5], BLACK, "the second device's back buffer holds the cursor");
    assert_eq!(pixels[4 * 64 + 4], BLUE, "the cursor starts at its hotspot's column");
}
