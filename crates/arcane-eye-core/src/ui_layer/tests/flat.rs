//! The UI layer once a feature of the stereo pair failed: the game gets its UI enable byte and its
//! cursor display counter back, and the layer changes no pixel of the back buffer.

use diag::health::{Feature, Health};

use super::{GREEN, Game, HEIGHT, RED, SIDE_BY_SIDE, WIDTH, eighths, frame, running};
use crate::eye_view::Layout;
use crate::pair::TEST_PAIR;
use crate::raw::test_pixels::{Pixel, back_buffer_pixels, fill_back_buffer};
use crate::raw::test_support::{TestCursor, TestSwapchain, display_count};
use crate::ui_layer::cursor::Aim;
use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::cursor::place::{Pointer, spot};
use crate::ui_layer::test_support::cursor_frame;

const BLUE: Pixel = [0, 0, 255, 255];
const WHITE: Pixel = [255, 255, 255, 255];

/// A back buffer whose left half is blue and whose right half is white, which a pair copy of a flat
/// frame and a UI draw would both change.
fn halves_picture() -> Vec<Pixel> {
    let half = usize::try_from(WIDTH / 2).expect("a half width");
    let row = [vec![BLUE; half], vec![WHITE; half]].concat();
    let height = usize::try_from(HEIGHT).expect("a height");
    row.repeat(height)
}

/// An aim of `cursor` at a pointer inside the test window's client area.
fn aim(cursor: &TestCursor) -> Aim {
    let width = i32::try_from(WIDTH).expect("a client width");
    let height = i32::try_from(HEIGHT).expect("a client height");
    let pointer = Pointer { x: 10, y: 4, width, height };
    Aim { handle: cursor.handle(), spot: spot(pointer).expect("a pointer inside") }
}

/// A registry with the UI layer and `feature` running.
fn running_with(feature: Feature) -> Health {
    let health = running();
    health.install_ok(feature);
    health
}

#[test]
fn a_failure_of_each_pair_feature_gives_the_game_its_values_back_and_changes_no_pixel() {
    for feature in TEST_PAIR {
        let test = TestSwapchain::new(WIDTH, HEIGHT);
        let game = Game::new(&test);
        let health = running_with(feature);
        let mut layer = game.layer();
        let cursor = TestCursor::monochrome([0, 0], &[0; 128], &[0; 128]);
        let before = display_count();
        frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
        frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
        assert_eq!(eighths(&test), [RED, GREEN, RED, GREEN], "{feature:?}: the UI in each half");
        cursor_frame(&mut layer, &test, WindowsCursor::Hidden, Some(aim(&cursor)));
        assert_eq!((game.ui_enabled(), display_count()), (0, before - 1), "{feature:?}");

        health.check_failed(feature);
        fill_back_buffer(&test, &halves_picture());
        frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
        frame(&mut layer, &health, &test.swapchain, Layout::MainOnly);

        assert_eq!(game.ui_enabled(), 1, "{feature:?}: the game draws its UI again");
        assert_eq!(display_count(), before, "{feature:?}: Windows shows its cursor again");
        assert!(back_buffer_pixels(&test) == halves_picture(), "{feature:?}: no pixel changed");
    }
}

#[test]
fn a_release_gives_the_game_its_ui_enable_byte_and_cursor_display_counter_back_once() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    let mut layer = game.layer();
    let cursor = TestCursor::monochrome([0, 0], &[0; 128], &[0; 128]);
    let before = display_count();
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
    cursor_frame(&mut layer, &test, WindowsCursor::Hidden, Some(aim(&cursor)));
    assert_eq!((game.ui_enabled(), display_count()), (0, before - 1));

    layer.release();
    layer.release();

    assert_eq!(game.ui_enabled(), 1, "the game draws its UI again");
    assert_eq!(display_count(), before, "one counter call for two releases");
}

#[test]
fn a_failed_install_of_a_pair_feature_leaves_the_layer_drawing() {
    let test = TestSwapchain::new(WIDTH, HEIGHT);
    let game = Game::new(&test);
    let health = running();
    health.install_failed(Feature::ShadowCache);
    let mut layer = game.layer();

    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);
    frame(&mut layer, &health, &test.swapchain, SIDE_BY_SIDE);

    assert_eq!(eighths(&test), [RED, GREEN, RED, GREEN]);
    assert_eq!(game.ui_enabled(), 0);
}
