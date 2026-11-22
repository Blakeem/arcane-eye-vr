//! The depth map's decisions: what a frame's map shows from what the UI layer drew into the frame,
//! the UI disparity of the UI's columns and the crossed disparity of the distance texture's unit.

use crate::pair::PairState;
use crate::ui_layer::decide::{Draw, Seen};

/// The draw of a frame that ran to its end and who drew the frame's UI, as its UI enable byte
/// showed it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drawn {
    pub(crate) draw: Draw,
    pub(crate) seen: Option<Seen>,
}

/// Whether a frame's map holds the UI texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiShown {
    Drawn,
    NotDrawn,
}

/// What a frame's map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Content {
    /// No cell, for a frame that holds the game's UI, a frame whose draw did not run to its end and
    /// a frame of a flat pair.
    Empty,
    /// The pair the core made from a flat picture.
    Flat(UiShown),
    /// A frame the game rendered side by side.
    Rendered(UiShown),
}

/// What the map of a frame of the stereo pair `pair` shows, whose UI layer drew `drawn`, or `None`
/// when its draw did not run to its end. A side by side frame with no draw and a pair with no UI
/// draw hold the scene alone when the game switched its UI off. Each holds the game's UI when the
/// game drew it or no composite stage was found, so its map holds no cell.
pub(crate) const fn content(pair: PairState, drawn: Option<Drawn>) -> Content {
    let Some(Drawn { draw, seen }) = drawn else { return Content::Empty };
    if pair.is_flat() {
        return Content::Empty;
    }
    match draw {
        Draw::Nothing => match seen {
            Some(Seen::OffByGame) => Content::Rendered(UiShown::NotDrawn),
            Some(Seen::DrawnByGame | Seen::LeftToCore) | None => Content::Empty,
        },
        Draw::Pair => match seen {
            Some(Seen::OffByGame) => Content::Flat(UiShown::NotDrawn),
            Some(Seen::DrawnByGame | Seen::LeftToCore) | None => Content::Empty,
        },
        Draw::PairThenUi => Content::Flat(UiShown::Drawn),
        Draw::Move(_) => Content::Rendered(UiShown::NotDrawn),
        Draw::Ui | Draw::MoveThenUi(_) => Content::Rendered(UiShown::Drawn),
    }
}

/// The crossed disparity between the UI in the left half and in the right half of a UI drawn
/// `columns` to the right in the left half of `half_width` columns, and as far to the left in
/// the right half. 0 for a half of no column.
pub(crate) fn ui_disparity(columns: u16, half_width: u16) -> f32 {
    if half_width == 0 {
        return 0.0;
    }
    2.0 * f32::from(columns) / f32::from(half_width)
}

/// The crossed disparity `lens * eye_m / (2 * z)` of a scene at the distance texture's value 1,
/// which is the far plane `far_m`, at the eye distance in use `eye_m`. `None` when it is not finite
/// and above 0, as for a camera with no far plane.
pub(crate) fn crossed_per_unit(lens: f32, eye_m: f32, far_m: f32) -> Option<f32> {
    let crossed = lens * eye_m / (2.0 * far_m);
    (crossed.is_finite() && crossed > 0.0).then_some(crossed)
}

#[cfg(test)]
mod tests {
    use diag::health::Feature;
    use settings::player::DepthOffsetPercent;

    use super::{Content, Drawn, UiShown, content, crossed_per_unit, ui_disparity};
    use crate::pair::PairState;
    use crate::ui_layer::decide::{Draw, Seen, depth_shift};
    use crate::ui_layer::depth_offset::DepthOffset;

    /// What the map shows of a frame of the pair `pair` whose draw `draw` ran to its end and whose
    /// UI enable byte showed `seen`.
    const fn shown(pair: PairState, draw: Draw, seen: Seen) -> Content {
        content(pair, Some(Drawn { draw, seen: Some(seen) }))
    }

    #[test]
    fn a_frame_that_holds_the_game_ui_or_did_not_draw_or_a_flat_pair_holds_no_cell() {
        let percent = DepthOffsetPercent::try_from(1.5).expect("a depth offset in range");
        let shift = depth_shift(2560, 1600, DepthOffset::of_setting(percent)).expect("a move");
        let (running, flat) = (PairState::Running, PairState::Flat { cause: Feature::UiLayer });
        let no_stage = |draw| content(running, Some(Drawn { draw, seen: None }));

        assert_eq!(shown(running, Draw::Nothing, Seen::DrawnByGame), Content::Empty);
        assert_eq!(no_stage(Draw::Nothing), Content::Empty, "no composite stage");
        assert_eq!(shown(running, Draw::Pair, Seen::DrawnByGame), Content::Empty, "the game's UI");
        assert_eq!(no_stage(Draw::Pair), Content::Empty, "a flat picture with no composite stage");
        assert_eq!(content(running, None), Content::Empty, "a draw that did not end");
        for draw in [Draw::Ui, Draw::Move(shift), Draw::Pair, Draw::PairThenUi] {
            assert_eq!(shown(flat, draw, Seen::LeftToCore), Content::Empty, "{draw:?}");
        }
    }

    #[test]
    fn a_frame_maps_the_scene_it_shows_and_the_ui_it_drew() {
        let percent = DepthOffsetPercent::try_from(1.5).expect("a depth offset in range");
        let shift = depth_shift(2560, 1600, DepthOffset::of_setting(percent)).expect("a move");
        let running = PairState::Running;
        let (ui, scene) = (UiShown::Drawn, UiShown::NotDrawn);

        let unmoved = shown(running, Draw::Nothing, Seen::OffByGame);
        assert_eq!(unmoved, Content::Rendered(scene), "a UI the game switched off and no move");
        assert_eq!(shown(running, Draw::Move(shift), Seen::OffByGame), unmoved);
        assert_eq!(shown(running, Draw::Pair, Seen::OffByGame), Content::Flat(scene));
        assert_eq!(shown(running, Draw::PairThenUi, Seen::LeftToCore), Content::Flat(ui));
        assert_eq!(shown(running, Draw::Ui, Seen::LeftToCore), Content::Rendered(ui));
        assert_eq!(
            shown(running, Draw::MoveThenUi(shift), Seen::LeftToCore),
            Content::Rendered(ui)
        );
    }

    #[test]
    fn the_ui_disparity_is_twice_the_columns_the_ui_moves_in_each_half() {
        assert!((ui_disparity(13, 1280) - 0.020_312_5).abs() < 1e-7);
        assert!(ui_disparity(0, 1280).abs() < f32::EPSILON);
        assert!(ui_disparity(13, 0).abs() < f32::EPSILON, "a half of no column");
    }

    #[test]
    fn the_crossed_disparity_of_the_distance_unit_needs_a_far_plane_and_an_eye_distance() {
        let crossed = crossed_per_unit(1.080_552_4, 0.065, 100.0).expect("a disparity");

        assert!((crossed - 0.000_351_18).abs() < 1e-8, "{crossed}");
        assert_eq!(crossed_per_unit(1.080_552_4, 0.065, 0.0), None, "no far plane");
        assert_eq!(crossed_per_unit(f32::NAN, 0.065, 100.0), None);
        assert_eq!(crossed_per_unit(1.080_552_4, 0.0, 100.0), None, "no eye distance");
    }
}
