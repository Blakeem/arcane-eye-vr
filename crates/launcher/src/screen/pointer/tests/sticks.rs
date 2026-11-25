//! The camera keys and the wheel of the two sticks across the pointing hand, the menu, the panel,
//! the layers and the session's end: the left stick pans, and the right stick turns and is the
//! wheel.

use device_profile::profile::Hand;
use launch::camera_key::{CameraKey, PanKey, TurnKey};
use openxr::Vector2f;
use xr::input::HandInput;

use super::{Action, PanelAim, Run, Sent, Sight, beside, centered, closed, stick};

/// The frames of one second at 90 hertz.
const SECOND: usize = 90;

const A: CameraKey = CameraKey::Pan(PanKey::Left);
const D: CameraKey = CameraKey::Pan(PanKey::Right);
const W: CameraKey = CameraKey::Pan(PanKey::Forward);
const DELETE: CameraKey = CameraKey::Turn(TurnKey::Left);
const END: CameraKey = CameraKey::Turn(TurnKey::Right);

/// `hand` with its `scroll` control at `x`, `y`.
fn scroll(x: f32, y: f32, hand: HandInput) -> HandInput {
    HandInput { scroll: Vector2f { x, y }, ..hand }
}

/// `hand` with its `select` pulled, so it becomes the pointing hand.
fn selecting(hand: HandInput) -> HandInput {
    HandInput { select: 1.0, ..hand }
}

#[test]
fn a_right_stick_at_0_8_presses_the_right_turn_key_once_and_holds_it() {
    let mut run = Run::new();
    for _ in 0..10 {
        run.right(stick(0.8, centered()));
    }
    assert_eq!(run.keys(), [Action::PressKey(END)]);
}

#[test]
fn a_diagonal_right_stick_turns_the_camera_and_the_wheel_at_once() {
    let mut run = Run::new();
    for _ in 0..SECOND {
        run.right(scroll(0.8, 0.4, centered()));
    }
    assert_eq!(run.keys(), [Action::PressKey(END)]);
    // A scroll of 0.4 turns (0.4 - 0.25) / 0.75 of 12 notches per second, 2.4 in a second.
    assert_eq!(run.notches(), 2);
}

#[test]
fn a_right_stick_from_left_to_right_releases_delete_and_presses_end_in_one_frame() {
    let mut run = Run::new();
    run.right(stick(-0.8, centered()));
    assert_eq!(run.keys(), [Action::PressKey(DELETE)]);
    run.right(stick(0.8, centered()));
    let expected = [Action::PressKey(DELETE), Action::ReleaseKey(DELETE), Action::PressKey(END)];
    assert_eq!(run.keys(), expected);
}

#[test]
fn the_left_stick_pans_while_the_right_hand_points_and_while_the_left_hand_points() {
    let mut run = Run::new();
    run.frame(scroll(0.8, 0.8, centered()), centered(), closed());
    assert_eq!(run.keys(), [Action::PressKey(D), Action::PressKey(W)], "the right hand points");
    let changed = run.frame(selecting(scroll(0.8, 0.8, centered())), centered(), closed());
    assert_eq!(changed.changed, Some(Hand::Left));
    run.frame(selecting(scroll(-0.8, 0.8, centered())), centered(), closed());
    let expected =
        [Action::PressKey(D), Action::PressKey(W), Action::ReleaseKey(D), Action::PressKey(A)];
    assert_eq!(run.keys(), expected, "the left hand points");
}

#[test]
fn the_right_stick_turns_and_is_the_wheel_while_the_left_hand_points() {
    let mut run = Run::new();
    let left = selecting(centered());
    assert_eq!(run.frame(left, centered(), closed()).changed, Some(Hand::Left));
    for _ in 0..91 {
        run.frame(left, scroll(-0.8, 1.0, centered()), closed());
    }
    assert_eq!(run.keys(), [Action::PressKey(DELETE)]);
    assert!((run.notches() - 12).abs() <= 1, "{} notches in a second", run.notches());
}

#[test]
fn a_change_of_the_pointing_hand_keeps_each_held_key() {
    let mut run = Run::new();
    run.frame(scroll(0.8, 0.0, centered()), stick(0.8, centered()), closed());
    run.frame(selecting(scroll(0.8, 0.0, centered())), stick(0.8, centered()), closed());
    run.frame(scroll(0.8, 0.0, centered()), selecting(stick(0.8, centered())), closed());
    assert_eq!(run.keys(), [Action::PressKey(D), Action::PressKey(END)]);
}

#[test]
fn a_stick_presses_its_key_while_the_ray_misses_the_screen_or_the_aim_pose_is_lost() {
    for hand in [beside(), HandInput::default()] {
        let mut run = Run::new();
        assert_eq!(run.frame(stick(0.8, hand), stick(0.8, hand), closed()).aimed, None);
        assert_eq!(run.keys(), [Action::PressKey(D), Action::PressKey(END)], "{hand:?}");
    }
}

#[test]
fn the_menu_on_a_hand_stops_that_hand_s_stick_and_not_the_other() {
    let mut run = Run::new();
    let sticks = (scroll(0.8, 0.0, centered()), scroll(0.8, 1.0, centered()));
    run.frame(sticks.0, sticks.1, closed());
    let menu_on_right = Sight { menu: Some(Hand::Right), ..closed() };
    for _ in 0..SECOND {
        run.frame(sticks.0, sticks.1, menu_on_right);
    }
    assert_eq!(run.keys(), [Action::PressKey(D), Action::PressKey(END), Action::ReleaseKey(END)]);
    assert_eq!(run.notches(), 0, "the menu hand's wheel");
    let menu_on_left = Sight { menu: Some(Hand::Left), ..closed() };
    run.frame(sticks.0, sticks.1, menu_on_left);
    let expected = [
        Action::PressKey(D),
        Action::PressKey(END),
        Action::ReleaseKey(END),
        Action::ReleaseKey(D),
        Action::PressKey(END),
    ];
    assert_eq!(run.keys(), expected);
}

#[test]
fn a_ray_on_the_panel_stops_the_pointing_hand_s_stick_once_and_not_the_other() {
    let mut run = Run::new();
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    let on_panel = Sight { menu: Some(Hand::Left), panel: PanelAim::Met, ..closed() };
    run.frame(stick(0.8, centered()), stick(0.8, centered()), on_panel);
    run.frame(stick(0.8, centered()), stick(0.8, centered()), on_panel);
    let expected = [
        Action::PressKey(D),
        Action::PressKey(END),
        Action::ReleaseKey(D),
        Action::ReleaseKey(END),
    ];
    assert_eq!(run.keys(), expected, "the menu stops the left stick and the panel the right");
    let mut pointing_right = Run::new();
    let ray_on_panel = Sight { panel: PanelAim::Met, ..closed() };
    pointing_right.frame(stick(0.8, centered()), stick(0.8, centered()), ray_on_panel);
    assert_eq!(pointing_right.keys(), [Action::PressKey(D)], "the left stick acts");
}

#[test]
fn each_held_key_is_released_once_when_the_screen_quad_no_longer_shows() {
    let mut run = Run::new();
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    let hidden = Sight { surface: None, ..closed() };
    run.frame(stick(0.8, centered()), stick(0.8, centered()), hidden);
    run.frame(stick(0.8, centered()), stick(0.8, centered()), hidden);
    let expected = [
        Action::PressKey(D),
        Action::PressKey(END),
        Action::ReleaseKey(D),
        Action::ReleaseKey(END),
    ];
    assert_eq!(run.keys(), expected);
}

#[test]
fn each_held_key_is_released_once_at_the_session_s_end() {
    let mut run = Run::new();
    run.frame(scroll(-0.8, 0.8, centered()), stick(-0.8, centered()), closed());
    run.finish();
    run.finish();
    let expected = [
        Action::PressKey(A),
        Action::PressKey(W),
        Action::PressKey(DELETE),
        Action::ReleaseKey(A),
        Action::ReleaseKey(W),
        Action::ReleaseKey(DELETE),
    ];
    assert_eq!(run.keys(), expected);
}

#[test]
fn a_refused_key_press_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.result = Sent::Failed;
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    run.result = Sent::Done;
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    assert_eq!(run.count(Action::PressKey(D)), 2);
    assert_eq!(run.count(Action::PressKey(END)), 2);
}

#[test]
fn a_failed_key_release_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.frame(stick(0.8, centered()), stick(0.8, centered()), closed());
    run.result = Sent::Failed;
    run.frame(centered(), centered(), closed());
    run.result = Sent::Done;
    run.frame(centered(), centered(), closed());
    run.frame(centered(), centered(), closed());
    assert_eq!(run.count(Action::ReleaseKey(D)), 2);
    assert_eq!(run.count(Action::ReleaseKey(END)), 2);
}
