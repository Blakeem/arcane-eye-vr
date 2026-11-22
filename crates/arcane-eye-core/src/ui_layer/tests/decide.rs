//! The UI layer's decisions for each pair of picture and UI enable byte state, and the halves.

use gfx::layer::{Area, Placement, Rect};
use ipc_layout::viewer_header::screen::ParallelShare;
use settings::player::DepthOffsetPercent;

use crate::ui_layer::cursor::counter::WindowsCursor;
use crate::ui_layer::decide::{Draw, Picture, Seen, Shift, Step, Write, depth_shift, halves, step};
use crate::ui_layer::depth_offset::DepthOffset;

fn depth(percent: f32) -> DepthOffset {
    DepthOffset::of_setting(DepthOffsetPercent::try_from(percent).expect("an offset in range"))
}

/// The move of a 64 by 32 frame's right half by 2 columns.
const SHIFT: Shift = Shift {
    source: Rect { x: 32, y: 0, width: 30, height: 32 },
    picture: Placement {
        area: Area { x: 2.0, y: 0.0, width: 64.0, height: 32.0 },
        clip: Rect { x: 34, y: 0, width: 30, height: 32 },
    },
    uncovered: Placement {
        area: Area { x: 32.0, y: 0.0, width: 2.0, height: 32.0 },
        clip: Rect { x: 32, y: 0, width: 2, height: 32 },
    },
};

#[test]
fn a_side_by_side_frame_with_the_games_ui_clears_the_byte_keeps_the_windows_cursor_and_no_move() {
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::DrawnByGame), Some(SHIFT)),
        Step { draw: Draw::Nothing, write: Write::Clear, windows_cursor: WindowsCursor::Shown },
        "a move would move the game's UI"
    );
}

#[test]
fn a_side_by_side_frame_the_game_left_to_the_core_gets_the_move_then_the_ui_in_each_half() {
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::LeftToCore), Some(SHIFT)),
        Step {
            draw: Draw::MoveThenUi(SHIFT),
            write: Write::Keep,
            windows_cursor: WindowsCursor::Hidden
        }
    );
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::LeftToCore), None),
        Step { draw: Draw::Ui, write: Write::Keep, windows_cursor: WindowsCursor::Hidden },
        "a move of 0 columns"
    );
}

#[test]
fn a_flat_frame_with_the_games_ui_becomes_a_pair_with_it_and_clears_the_byte() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::DrawnByGame), Some(SHIFT)),
        Step { draw: Draw::Pair, write: Write::Clear, windows_cursor: WindowsCursor::Hidden }
    );
}

#[test]
fn a_flat_frame_the_game_left_to_the_core_becomes_a_pair_with_the_ui_and_keeps_the_byte_at_0() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::LeftToCore), Some(SHIFT)),
        Step { draw: Draw::PairThenUi, write: Write::Keep, windows_cursor: WindowsCursor::Hidden },
        "the two halves of a flat frame's pair are not moved"
    );
}

#[test]
fn a_ui_the_game_switched_off_gets_no_draw_and_no_write_and_a_flat_frame_still_becomes_a_pair() {
    assert_eq!(
        step(Picture::Flat, Some(Seen::OffByGame), Some(SHIFT)),
        Step { draw: Draw::Pair, write: Write::Keep, windows_cursor: WindowsCursor::Hidden }
    );
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::OffByGame), Some(SHIFT)),
        Step { draw: Draw::Move(SHIFT), write: Write::Keep, windows_cursor: WindowsCursor::Hidden }
    );
    assert_eq!(
        step(Picture::SideBySide, Some(Seen::OffByGame), None),
        Step { draw: Draw::Nothing, write: Write::Keep, windows_cursor: WindowsCursor::Hidden }
    );
}

#[test]
fn a_frame_without_a_composite_stage_gets_no_ui_or_write_and_a_flat_frame_still_becomes_a_pair() {
    assert_eq!(
        step(Picture::Flat, None, Some(SHIFT)),
        Step { draw: Draw::Pair, write: Write::Keep, windows_cursor: WindowsCursor::Hidden }
    );
    assert_eq!(
        step(Picture::SideBySide, None, Some(SHIFT)),
        Step { draw: Draw::Nothing, write: Write::Keep, windows_cursor: WindowsCursor::Shown },
        "the game's UI across the entire side by side frame keeps the Windows cursor and no move"
    );
}

#[test]
fn each_draw_names_whether_it_makes_a_pair_its_move_and_whether_it_draws_the_ui() {
    let parts = |draw: Draw| (draw.makes_pair(), draw.shift(), draw.draws_ui());
    assert_eq!(parts(Draw::Nothing), (false, None, false));
    assert_eq!(parts(Draw::Ui), (false, None, true));
    assert_eq!(parts(Draw::Move(SHIFT)), (false, Some(SHIFT), false));
    assert_eq!(parts(Draw::MoveThenUi(SHIFT)), (false, Some(SHIFT), true));
    assert_eq!(parts(Draw::Pair), (true, None, false));
    assert_eq!(parts(Draw::PairThenUi), (true, None, true));
}

#[test]
fn the_move_is_the_offset_of_the_right_halfs_width_rounded_to_entire_columns() {
    assert_eq!(depth_shift(64, 32, depth(5.0)), Some(SHIFT), "1.6 columns round to 2");
    let wide = depth_shift(3840, 2160, depth(1.5)).expect("28.8 columns round to 29");
    assert_eq!(wide.picture.area, Area { x: 29.0, y: 0.0, width: 3840.0, height: 2160.0 });
    assert_eq!(wide.uncovered.area, Area { x: 1920.0, y: 0.0, width: 29.0, height: 2160.0 });
    assert_eq!(wide.source, Rect { x: 1920, y: 0, width: 1891, height: 2160 });
    let odd = depth_shift(1281, 720, depth(5.0)).expect("5 percent of 641 columns rounds to 32");
    assert_eq!(odd.source, Rect { x: 640, y: 0, width: 609, height: 720 }, "641 less 32 kept");
    assert_eq!(
        odd.picture,
        Placement {
            area: Area { x: 32.0, y: 0.0, width: 1281.0, height: 720.0 },
            clip: Rect { x: 672, y: 0, width: 609, height: 720 },
        }
    );
    assert_eq!(
        odd.uncovered,
        Placement {
            area: Area { x: 640.0, y: 0.0, width: 32.0, height: 720.0 },
            clip: Rect { x: 640, y: 0, width: 32, height: 720 },
        }
    );
}

#[test]
fn the_moved_columns_read_the_source_and_the_two_clips_tile_the_right_half() {
    let shift = depth_shift(3840, 2160, depth(5.0)).expect("96 columns");

    let black = Rect { x: 1920, y: 0, width: 96, height: 2160 };
    assert_eq!(shift.uncovered.clip, black, "black from the right half's left edge");
    let moved = Rect { x: 2016, y: 0, width: 1824, height: 2160 };
    assert_eq!(shift.picture.clip, moved, "the moved columns from black's end to the frame's edge");
    let picture = Area { x: 96.0, y: 0.0, width: 3840.0, height: 2160.0 };
    assert_eq!(shift.picture.area, picture, "column 2016 reads column 1920");
    let read = Rect { x: 1920, y: 0, width: 1824, height: 2160 };
    assert_eq!(shift.source, read, "the columns 2016 to 3839 read");
}

#[test]
fn a_move_that_rounds_to_0_columns_or_a_size_without_halves_is_no_move() {
    assert_eq!(depth_shift(64, 32, depth(0.0)), None);
    assert_eq!(depth_shift(64, 32, depth(1.5)), None, "0.48 columns round to 0");
    assert_eq!(depth_shift(0, 32, depth(5.0)), None);
    assert_eq!(depth_shift(65_536, 32, depth(5.0)), None);
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

#[test]
fn a_screen_s_share_moves_the_right_half_and_with_no_screen_the_setting_does() {
    let screen = ParallelShare::of_screen(0.065, 3.0);
    let on_screen = DepthOffset::in_use(DepthOffsetPercent::default(), screen);
    let screen_shift = depth_shift(3840, 2160, on_screen).expect("41.6 columns round to 42");
    assert_eq!(screen_shift.uncovered.clip.width, 42);
    let setting = DepthOffset::in_use(DepthOffsetPercent::default(), None);
    let setting_shift = depth_shift(3840, 2160, setting).expect("28.8 columns round to 29");
    assert_eq!(setting_shift.uncovered.clip.width, 29);
}
