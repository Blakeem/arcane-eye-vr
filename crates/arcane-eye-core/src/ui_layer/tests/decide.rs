//! The UI layer's decisions for each pair of picture and UI enable byte state, and the halves.

use gfx::layer::Rect;

use crate::ui_layer::decide::{Draw, Picture, Seen, Step, Write, halves, step};

#[test]
fn a_side_by_side_frame_with_the_games_ui_clears_the_byte_and_draws_nothing() {
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::DrawnByGame)),
        Step { draw: Draw::Nothing, write: Write::Clear }
    );
}

#[test]
fn a_side_by_side_frame_the_game_left_to_the_core_gets_the_ui_in_each_half() {
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::LeftToCore)),
        Step { draw: Draw::Ui, write: Write::Keep }
    );
}

#[test]
fn a_flat_frame_with_the_games_ui_becomes_a_pair_with_it_and_clears_the_byte() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::DrawnByGame)),
        Step { draw: Draw::Pair, write: Write::Clear }
    );
}

#[test]
fn a_flat_frame_the_game_left_to_the_core_becomes_a_pair_with_the_ui_and_keeps_the_byte_at_0() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::LeftToCore)),
        Step { draw: Draw::PairThenUi, write: Write::Keep }
    );
}

#[test]
fn a_ui_the_game_switched_off_gets_no_draw_and_no_write_and_a_flat_frame_still_becomes_a_pair() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::OffByGame)),
        Step { draw: Draw::Pair, write: Write::Keep }
    );
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::OffByGame)),
        Step { draw: Draw::Nothing, write: Write::Keep }
    );
}

#[test]
fn a_frame_without_a_composite_stage_gets_no_ui_or_write_and_a_flat_frame_still_becomes_a_pair() {
    assert_eq!(step(Picture::Flat, None), Step { draw: Draw::Pair, write: Write::Keep });
    assert_eq!(step(Picture::SideBySide, None), Step { draw: Draw::Nothing, write: Write::Keep });
}

#[test]
fn each_draw_names_whether_it_makes_a_pair_and_whether_it_draws_the_ui() {
    let parts = |draw: Draw| (draw.makes_pair(), draw.draws_ui());
    assert_eq!(parts(Draw::Nothing), (false, false));
    assert_eq!(parts(Draw::Ui), (false, true));
    assert_eq!(parts(Draw::Pair), (true, false));
    assert_eq!(parts(Draw::PairThenUi), (true, true));
}

#[test]
fn the_halves_split_the_width_and_an_odd_column_goes_to_the_right_half() {
    assert_eq!(
        halves(3840, 2160),
        Some([
            Rect { x: 0, y: 0, width: 1920, height: 2160 },
            Rect { x: 1920, y: 0, width: 1920, height: 2160 },
        ])
    );
    assert_eq!(
        halves(1281, 720),
        Some([
            Rect { x: 0, y: 0, width: 640, height: 720 },
            Rect { x: 640, y: 0, width: 641, height: 720 },
        ])
    );
}

#[test]
fn a_size_without_a_pixel_or_past_a_rectangles_fields_holds_no_halves() {
    assert_eq!(halves(0, 2160), None);
    assert_eq!(halves(3840, 0), None);
    assert_eq!(halves(65_536, 2160), None);
    assert_eq!(halves(3840, 65_536), None);
}
