//! The move of a side by side frame's right half, read from the back buffer of a test game's frames.

use game_bindings::offsets::COMPOSITE_STAGE_UI_ENABLED;

use super::{Game, HEIGHT, RED, SIDE_BY_SIDE, WIDTH, depth, frame, offset_frame, running};
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, fill_back_buffer};
use crate::raw::test_support::TestSwapchain;

const BLACK: Pixel = [0, 0, 0, 255];
const CLEAR: Pixel = [0, 0, 0, 0];

/// The columns of the back buffer's left half.
const HALF: usize = 32;

/// 5 percent of the 32-column half, 1.6 columns, rounds to 2.
const MOVED: usize = 2;

/// The first column the move cuts at the frame's right edge: the 64-column width less the move.
const CUT: usize = 62;

/// One row whose column `x` holds red `x` and full green, so no column is black.
fn columns() -> Vec<Pixel> {
    (0..u8::try_from(WIDTH).expect("a test width")).map(|x| [x, 255, 0, 255]).collect()
}

/// The back buffer filled with `row` in every row.
fn rows(row: &[Pixel]) -> Vec<Pixel> {
    row.repeat(HEIGHT as usize)
}

#[test]
fn a_side_by_side_frame_moves_its_right_half_right_with_black_in_the_uncovered_columns() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let health = running();
    let mut layer = game.layer();
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
    let picture = columns();

    fill_back_buffer(&test, &rows(&picture));
    offset_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, depth(5.0));

    let row = [&picture[..HALF], &[BLACK; MOVED][..], &picture[HALF..CUT]].concat();
    assert_eq!(back_buffer_pixels(&test), rows(&row), "the left half unchanged in every row");
}

#[test]
fn the_ui_drawn_after_the_move_stays_at_its_place_in_the_right_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    // A UI texture as wide as a half: red in its left 16 columns and clear in the rest.
    let ui = [[RED; 16], [CLEAR; 16]].concat();
    let game = Game::showing(&test, 32, &ui);
    let health = running();
    let mut layer = game.layer();
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
    let picture = columns();

    fill_back_buffer(&test, &rows(&picture));
    offset_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, depth(5.0));

    // Column 48, the right half's first column the UI leaves clear, shows column 46 moved by 2.
    let red = [RED; 16];
    let row = [&red[..], &picture[16..HALF], &red[..], &picture[46..CUT]].concat();
    assert_eq!(
        back_buffer_pixels(&test),
        rows(&row),
        "red over the black columns, not beside them"
    );
}

#[test]
fn an_offset_that_rounds_to_0_columns_makes_no_copy_and_no_draw() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.stage.set(COMPOSITE_STAGE_UI_ENABLED, 0_u8);
    let health = running();
    let picture = rows(&columns());

    for percent in [0.0, 1.5] {
        let mut layer = game.layer();
        layer.next = SIDE_BY_SIDE;
        fill_back_buffer(&test, &picture);
        offset_frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE, depth(percent));

        assert!(layer.layer.is_none(), "{percent} percent created the layer draw");
        assert_eq!(back_buffer_pixels(&test), picture, "{percent} percent changed the frame");
    }
}
