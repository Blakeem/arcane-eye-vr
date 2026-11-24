//! The camera turn keys the pointer holds from the `scroll` control's left and right.

use device_profile::profile::Hand;
use launch::turn::TurnKey;
use openxr::Vector2f;
use xr::input::HandInput;

use super::{Action, PanelAim, Run, Sent, Sight, beside, centered, closed, stick};

/// The frames of one second at 90 hertz.
const SECOND: usize = 90;

#[test]
fn a_stick_at_0_8_presses_the_right_turn_key_once_and_holds_it() {
    let mut run = Run::new();
    for _ in 0..10 {
        run.right(stick(0.8, centered()));
    }
    assert_eq!(run.turns(), [Action::PressTurn(TurnKey::Right)]);
}

#[test]
fn a_diagonal_stick_turns_the_camera_and_the_wheel_at_once() {
    let mut run = Run::new();
    let diagonal = HandInput { scroll: Vector2f { x: 0.8, y: 0.4 }, ..centered() };
    for _ in 0..SECOND {
        run.right(diagonal);
    }
    assert_eq!(run.turns(), [Action::PressTurn(TurnKey::Right)]);
    // A scroll of 0.4 turns (0.4 - 0.25) / 0.75 of 12 notches per second, 2.4 in a second.
    assert_eq!(run.notches(), 2);
}

#[test]
fn a_stick_from_left_to_right_releases_the_left_key_and_presses_the_right_key_in_one_frame() {
    let mut run = Run::new();
    run.right(stick(-0.8, centered()));
    assert_eq!(run.turns(), [Action::PressTurn(TurnKey::Left)]);
    run.right(stick(0.8, centered()));
    let expected = [
        Action::PressTurn(TurnKey::Left),
        Action::ReleaseTurn(TurnKey::Left),
        Action::PressTurn(TurnKey::Right),
    ];
    assert_eq!(run.turns(), expected);
}

#[test]
fn the_turn_key_is_pressed_while_the_ray_misses_the_screen_or_the_aim_pose_is_lost() {
    for hand in [beside(), HandInput::default()] {
        let mut run = Run::new();
        assert_eq!(run.right(stick(0.8, hand)).aimed, None, "{hand:?}");
        assert_eq!(run.turns(), [Action::PressTurn(TurnKey::Right)], "{hand:?}");
    }
}

#[test]
fn the_held_turn_key_is_released_when_the_pointing_hand_changes_and_pressed_again_after() {
    let mut run = Run::new();
    run.right(stick(0.8, centered()));
    let left_press = HandInput { select: 1.0, ..stick(0.8, centered()) };
    run.frame(left_press, stick(0.8, centered()), closed());
    run.frame(left_press, stick(0.8, centered()), closed());
    let expected = [
        Action::PressTurn(TurnKey::Right),
        Action::ReleaseTurn(TurnKey::Right),
        Action::PressTurn(TurnKey::Right),
    ];
    assert_eq!(run.turns(), expected);
}

#[test]
fn the_held_turn_key_is_released_once_while_the_ray_meets_the_menu_s_panel() {
    let mut run = Run::new();
    run.right(stick(0.8, centered()));
    let on_panel = Sight { menu: Some(Hand::Left), panel: PanelAim::Met, ..closed() };
    run.frame(HandInput::default(), stick(0.8, centered()), on_panel);
    run.frame(HandInput::default(), stick(0.8, centered()), on_panel);
    let expected = [Action::PressTurn(TurnKey::Right), Action::ReleaseTurn(TurnKey::Right)];
    assert_eq!(run.turns(), expected);
}

#[test]
fn the_held_turn_key_is_released_once_when_the_screen_quad_no_longer_shows() {
    let mut run = Run::new();
    run.right(stick(0.8, centered()));
    let hidden = Sight { surface: None, ..closed() };
    run.frame(HandInput::default(), stick(0.8, centered()), hidden);
    run.frame(HandInput::default(), stick(0.8, centered()), hidden);
    let expected = [Action::PressTurn(TurnKey::Right), Action::ReleaseTurn(TurnKey::Right)];
    assert_eq!(run.turns(), expected);
}

#[test]
fn the_held_turn_key_is_released_once_at_the_session_s_end() {
    let mut run = Run::new();
    run.right(stick(-0.8, centered()));
    run.finish();
    run.finish();
    let expected = [Action::PressTurn(TurnKey::Left), Action::ReleaseTurn(TurnKey::Left)];
    assert_eq!(run.turns(), expected);
}

#[test]
fn a_refused_turn_key_press_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.result = Sent::Failed;
    run.right(stick(0.8, centered()));
    run.result = Sent::Done;
    run.right(stick(0.8, centered()));
    run.right(stick(0.8, centered()));
    assert_eq!(run.count(Action::PressTurn(TurnKey::Right)), 2);
}

#[test]
fn a_failed_turn_key_release_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.right(stick(0.8, centered()));
    run.result = Sent::Failed;
    run.right(centered());
    run.result = Sent::Done;
    run.right(centered());
    run.right(centered());
    assert_eq!(run.count(Action::ReleaseTurn(TurnKey::Right)), 2);
}
