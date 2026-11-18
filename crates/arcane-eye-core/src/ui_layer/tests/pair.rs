//! The pair made of a flat frame, read from the back buffer of a test game's frames.

use diag::health::Feature;
use game_bindings::offsets::PIPELINE_SPECIAL_VIEW;

use super::{Game, HEIGHT, RED, WIDTH, frame, running, stop};
use crate::eye_view::Layout;
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, fill_back_buffer};
use crate::raw::test_support::TestSwapchain;

const BLUE: Pixel = [0, 0, 255, 255];
const WHITE: Pixel = [255, 255, 255, 255];
const CLEAR: Pixel = [0, 0, 0, 0];

/// The pixel at column `x` and row `y` of the back buffer.
fn at(pixels: &[Pixel], x: usize, y: usize) -> Pixel {
    let index = y.checked_mul(WIDTH as usize).and_then(|row| row.checked_add(x));
    pixels[index.expect("a pixel index")]
}

/// A flat picture with a blue left half and a white right half.
fn blue_then_white() -> Vec<Pixel> {
    let row = [[BLUE; 32], [WHITE; 32]].concat();
    row.repeat(HEIGHT as usize)
}

/// Columns 8, 24, 40 and 56 of row 27, in the bottom quarter where the UI texture of the pair test
/// is clear, hold the entire flat picture in each half, blue then white, twice.
fn assert_pair(pixels: &[Pixel]) {
    let bottom = [8, 24, 40, 56].map(|x| at(pixels, x, 27));
    assert_eq!(bottom, [BLUE, WHITE, BLUE, WHITE], "the flat picture at half its width, twice");
}

#[test]
fn a_flat_frame_holds_its_picture_and_the_ui_in_each_half() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    // A red top left corner, clear elsewhere, so the picture shows through.
    let game = Game::showing(&test, 2, &[RED, CLEAR, CLEAR, CLEAR]);
    let health = running();
    let mut layer = game.layer();

    fill_back_buffer(&test, &blue_then_white());
    frame(&mut layer, &health, &test, Layout::MainOnly);
    let game_ui_frame = back_buffer_pixels(&test);
    fill_back_buffer(&test, &blue_then_white());
    frame(&mut layer, &health, &test, Layout::MainOnly);
    let core_ui_frame = back_buffer_pixels(&test);

    assert_pair(&game_ui_frame);
    assert_eq!(at(&game_ui_frame, 4, 4), BLUE, "the game's UI frame gets no second UI");
    assert_pair(&core_ui_frame);
    let corners = [4, 36].map(|x| at(&core_ui_frame, x, 4));
    assert_eq!(corners, [RED, RED], "the UI texture's corner in each half");
    assert_eq!(at(&core_ui_frame, 27, 4), WHITE, "the flat picture under the clear UI");
}

#[test]
fn a_flat_frame_without_a_composite_stage_becomes_a_pair() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    game.world.pipeline.set(PIPELINE_SPECIAL_VIEW, 0_usize);
    let health = running();
    let mut layer = game.layer();

    fill_back_buffer(&test, &blue_then_white());
    frame(&mut layer, &health, &test, Layout::MainOnly);

    assert_pair(&back_buffer_pixels(&test));
    assert!(health.is_running(Feature::UiLayer));
}

#[test]
fn a_stopped_layer_drops_its_layer_draw_and_presents_flat_frames_flat() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    fill_back_buffer(&test, &blue_then_white());
    frame(&mut layer, &health, &test, Layout::MainOnly);
    assert!(layer.layer.is_some(), "the pair's frame holds the layer draw");

    stop(&mut layer, &health, &test);
    fill_back_buffer(&test, &blue_then_white());
    frame(&mut layer, &health, &test, Layout::MainOnly);

    assert!(layer.layer.is_none(), "the copy of the back buffer is released");
    let bottom = [8, 24, 40, 56].map(|x| at(&back_buffer_pixels(&test), x, 27));
    assert_eq!(bottom, [BLUE, BLUE, WHITE, WHITE], "the flat picture as the game drew it");
}
