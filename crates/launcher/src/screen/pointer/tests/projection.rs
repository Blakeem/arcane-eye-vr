//! The pointer's input while the projection picture shows the game: its place where the ray meets
//! the picture, its far point without a depth map, its buttons, wheel and camera keys, and the
//! releases at a change between the screen quad and the projection picture.

use ipc_layout::viewer_header::depth_map::{Cell, Grid, InterfaceArea, SCENE};
use launch::camera_key::{CameraKey, PanKey, TurnKey};
use launch::mouse::Button;
use openxr::Vector2f;
use xr::input::{ButtonState, HandInput};

use super::{Action, Run, Sight, Surface, aiming, centered, closed, stick};
use crate::screen::depth::DepthView;
use crate::screen::geometry::Shares;
use crate::screen::geometry::test_support::lens_views;

/// A world frame's scene 5 m in front of the eye views of `lens_views` in each cell of a map of
/// 160 by 90 cells.
fn scene() -> Vec<Cell> {
    vec![[-0.008_086, SCENE]; 14_400]
}

/// The projection picture of `lens_views` with the map of `cells`, or with no map for `None`, and
/// the menu closed.
fn projection(cells: Option<&[Cell]>) -> Sight<'_> {
    let grid = Grid::new(160, 90).expect("a grid");
    let area = InterfaceArea::FULL;
    let map = cells.map(|cells| DepthView { grid, depth_offset: 0.0, cells, area });
    Sight { surface: Some(Surface::Projection(lens_views(), map)), ..closed() }
}

/// Fails the test unless `run` sent exactly one move, to a place within 0.002 of (0.5, 0.5).
fn assert_moved_to_the_middle(run: &Run) {
    let [Action::Move(Shares { x, y })] = run.sent.as_slice() else {
        panic!("one move: {:?}", run.sent);
    };
    assert!((x - 0.5).abs() < 0.002 && (y - 0.5).abs() < 0.002, "moved to ({x}, {y})");
}

#[test]
fn a_ray_into_the_projection_picture_moves_the_pointer_where_it_meets_the_scene() {
    let cells = scene();
    let mut run = Run::new();
    let aimed = run.frame(HandInput::default(), centered(), projection(Some(&cells)));
    let point = aimed.aimed.expect("a meeting").point;
    assert!((point.z + 5.0).abs() < 0.05 && point.x.abs() < 0.01, "the laser ends at {point:?}");
    assert_moved_to_the_middle(&run);
}

#[test]
fn without_a_map_the_laser_ends_200_m_along_the_ray_at_the_far_point_s_mouse_place() {
    let mut run = Run::new();
    let aimed = run.frame(HandInput::default(), centered(), projection(None));
    let point = aimed.aimed.expect("a far point").point;
    assert!((point.z + 200.0).abs() < 0.01, "the laser ends at {point:?}");
    assert_moved_to_the_middle(&run);
}

#[test]
fn in_the_projection_picture_select_secondary_and_scroll_click_and_turn_the_wheel() {
    let cells = scene();
    let sight = projection(Some(&cells));
    let mut run = Run::new();
    let up = Vector2f { x: 0.0, y: 1.0 };
    for right in [
        centered(),
        HandInput { select: 1.0, ..centered() },
        centered(),
        HandInput { secondary: ButtonState::Pressed, ..centered() },
        centered(),
    ] {
        run.frame(HandInput::default(), right, sight);
    }
    let clicks: Vec<Action> =
        run.sent.iter().copied().filter(|sent| !matches!(sent, Action::Move(_))).collect();
    let expected = [
        Action::Press(Button::Left),
        Action::Release(Button::Left),
        Action::Press(Button::Right),
        Action::Release(Button::Right),
    ];
    assert_eq!(clicks, expected);
    for _ in 0..91 {
        run.frame(HandInput::default(), HandInput { scroll: up, ..centered() }, sight);
    }
    assert!((run.notches() - 12).abs() <= 1, "{} notches in a second", run.notches());
}

#[test]
fn a_ray_beside_the_projection_picture_releases_the_held_left_button_once() {
    let cells = scene();
    let sight = projection(Some(&cells));
    let mut run = Run::new();
    run.frame(HandInput::default(), centered(), sight);
    run.frame(HandInput::default(), HandInput { select: 1.0, ..centered() }, sight);
    assert_eq!(run.count(Action::Press(Button::Left)), 1, "the left button is held");
    // A ray from 200 m to the right along -Z has a place on the picture for the left eye only past
    // the walk's end, where it lies 252 m ahead.
    let held = HandInput { select: 1.0, ..aiming(200.0, 0.0) };
    assert_eq!(run.frame(HandInput::default(), held, sight).aimed, None, "no meeting");
    run.frame(HandInput::default(), held, sight);
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn a_frame_that_shows_the_game_in_the_projection_picture_pans_and_turns_the_camera() {
    let mut run = Run::new();
    for _ in 0..10 {
        run.frame(stick(-0.8, centered()), stick(0.8, centered()), projection(None));
    }
    let expected = [
        Action::PressKey(CameraKey::Pan(PanKey::Left)),
        Action::PressKey(CameraKey::Turn(TurnKey::Right)),
    ];
    assert_eq!(run.keys(), expected);
}

#[test]
fn a_change_from_the_screen_quad_to_the_projection_picture_releases_the_held_button_once() {
    let mut run = Run::new();
    run.hold_left();
    let held = HandInput { select: 1.0, ..centered() };
    run.frame(HandInput::default(), held, projection(None));
    run.frame(HandInput::default(), held, projection(None));
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
    assert_eq!(run.count(Action::Press(Button::Left)), 1, "the held trigger pressed nothing");
}

#[test]
fn a_change_between_the_layers_releases_each_held_key_and_presses_it_again_after() {
    let mut run = Run::new();
    let (left, right) = (stick(-0.8, centered()), stick(0.8, centered()));
    run.frame(left, right, projection(None));
    run.frame(left, right, closed());
    run.frame(left, right, closed());
    let (pan, turn) = (CameraKey::Pan(PanKey::Left), CameraKey::Turn(TurnKey::Right));
    let expected = [
        Action::PressKey(pan),
        Action::PressKey(turn),
        Action::ReleaseKey(pan),
        Action::ReleaseKey(turn),
        Action::PressKey(pan),
        Action::PressKey(turn),
    ];
    assert_eq!(run.keys(), expected);
}
