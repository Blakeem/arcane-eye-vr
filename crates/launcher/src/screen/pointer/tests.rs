//! The game's pointer across frames of controller input: the place it points at, the pointing
//! hand, its moves, presses, releases and wheel turns.

use std::time::Duration;

use device_profile::profile::Hand;
use launch::mouse::Button;
use openxr::{Posef, Quaternionf, Vector2f, Vector3f};
use xr::input::{ButtonState, HandInput, Input};
use xr::quad::{self, ScreenQuad};

use super::model::{Action, Model, PanelAim, Sent, Sight, Update};
use crate::screen::controls::{ControllerWatch, Frame};
use crate::screen::geometry::Shares;

/// The time between two frames at 90 hertz.
const FRAME: Duration = Duration::from_nanos(11_111_111);

/// The screen 2.5 meters ahead, 3.0 meters wide and 1.6875 meters high.
fn screen() -> ScreenQuad {
    quad::screen(2.5, 3.0, (3840, 2160)).expect("a frame")
}

/// A hand whose `aim` pose sits at `x`, `y` on the head's plane and points along -Z, so its ray
/// meets the screen at the same `x` and `y`.
fn aiming(x: f32, y: f32) -> HandInput {
    let aim = Posef { orientation: Quaternionf::IDENTITY, position: Vector3f { x, y, z: 0.0 } };
    HandInput { aim: Some(aim), ..HandInput::default() }
}

/// The screen with the menu closed.
fn closed() -> Sight {
    Sight { menu: None, panel: PanelAim::Missed, screen: Some(screen()) }
}

/// A hand that aims at the screen's center.
fn centered() -> HandInput {
    aiming(0.0, 0.0)
}

/// A hand that aims beside the screen.
fn beside() -> HandInput {
    aiming(2.0, 0.0)
}

/// A pointer, the controllers' watch, every action sent and what each send returns.
struct Run {
    model: Model,
    watch: ControllerWatch,
    sent: Vec<Action>,
    result: Sent,
}

impl Run {
    fn new() -> Self {
        Self {
            model: Model::new(),
            watch: ControllerWatch::default(),
            sent: Vec::new(),
            result: Sent::Done,
        }
    }

    /// Runs one frame of the hands `left` and `right` with `sight`.
    fn frame(&mut self, left: HandInput, right: HandInput, sight: Sight) -> Update {
        let input = Input { left, right, head: None };
        let (left_buttons, right_buttons) = self.watch.update(&input, |_| {});
        let frame = Frame { input, left: left_buttons, right: right_buttons };
        let (sent, result) = (&mut self.sent, self.result);
        self.model.update(&frame, sight, FRAME, &mut |action| {
            sent.push(action);
            result
        })
    }

    /// Runs one frame of the right hand `right` on the closed menu's screen.
    fn right(&mut self, right: HandInput) -> Update {
        self.frame(HandInput::default(), right, closed())
    }

    /// Ends the session.
    fn finish(&mut self) {
        let (sent, result) = (&mut self.sent, self.result);
        self.model.release_all(&mut |action| {
            sent.push(action);
            result
        });
    }

    fn count(&self, action: Action) -> usize {
        self.sent.iter().filter(|sent| **sent == action).count()
    }

    fn moves(&self) -> usize {
        self.sent.iter().filter(|sent| matches!(sent, Action::Move(_))).count()
    }

    fn notches(&self) -> i32 {
        let turns = self.sent.iter().filter_map(|sent| match sent {
            Action::Wheel(notches) => Some(i32::from(*notches)),
            Action::Move(_) | Action::Press(_) | Action::Release(_) => None,
        });
        turns.sum()
    }

    /// Presses the right hand's `select` over the screen's center, so the pointer holds the left
    /// button.
    fn hold_left(&mut self) {
        self.right(centered());
        self.right(HandInput { select: 1.0, ..centered() });
        assert_eq!(self.count(Action::Press(Button::Left)), 1, "the left button is held");
    }
}

#[test]
fn a_ray_on_the_screen_gives_its_place_as_shares_from_the_top_left_corner() {
    let size = screen().size;
    assert_eq!((size.width, size.height), (3.0, 1.6875));
    let mut run = Run::new();
    let center = run.right(centered()).aimed.expect("a hit").hit.shares;
    assert_eq!(center, Shares { x: 0.5, y: 0.5 });
    let shares = run.right(aiming(-0.75, 0.42)).aimed.expect("a hit").hit.shares;
    assert!((shares.x - 0.25).abs() < 0.001 && (shares.y - 0.2511).abs() < 0.001, "{shares:?}");
}

#[test]
fn a_ray_that_meets_the_menu_s_panel_or_misses_the_screen_acts_on_nothing() {
    let mut run = Run::new();
    let on_panel = Sight { menu: Some(Hand::Left), panel: PanelAim::Met, ..closed() };
    let pressed = HandInput { select: 1.0, ..centered() };
    assert_eq!(run.frame(HandInput::default(), pressed, on_panel).aimed, None);
    assert_eq!(run.right(beside()).aimed, None);
    let hidden = Sight { screen: None, ..closed() };
    assert_eq!(run.frame(HandInput::default(), centered(), hidden).aimed, None);
    assert!(run.sent.is_empty(), "{:?}", run.sent);
}

#[test]
fn the_pointing_hand_is_the_right_hand_then_the_last_to_press_select_and_never_the_menu_hand() {
    let mut run = Run::new();
    assert_eq!(run.right(centered()).changed, None, "the right hand points at the start");
    let left_press = HandInput { select: 1.0, ..centered() };
    let pressed = run.frame(left_press, HandInput::default(), closed()).changed;
    assert_eq!(pressed, Some(Hand::Left), "a press of the left hand's select");
    let menu_on_left = Sight { menu: Some(Hand::Left), ..closed() };
    let opened = run.frame(HandInput::default(), HandInput::default(), menu_on_left).changed;
    assert_eq!(opened, Some(Hand::Right), "the menu opened on the left hand");
    let menu_press = HandInput { select: 1.0, ..centered() };
    assert_eq!(run.frame(menu_press, HandInput::default(), menu_on_left).changed, None);
    let closing = run.frame(HandInput::default(), HandInput::default(), closed()).changed;
    assert_eq!(closing, Some(Hand::Left), "the menu hand's press left the last press unchanged");
}

#[test]
fn a_controller_at_rest_moves_the_pointer_once() {
    let mut run = Run::new();
    for _ in 0..100 {
        run.right(centered());
    }
    assert_eq!(run.sent, [Action::Move(Shares { x: 0.5, y: 0.5 })]);
}

#[test]
fn a_place_0_0004_from_the_last_move_sends_nothing_and_one_0_001_away_moves_the_pointer() {
    // On a screen 3 meters wide, 0.0012 meters are 0.0004 of its width and 0.003 meters 0.001.
    let mut run = Run::new();
    run.right(centered());
    for _ in 0..200 {
        run.right(aiming(0.0012, 0.0));
    }
    assert_eq!(run.moves(), 1, "0.0004 away");
    for _ in 0..200 {
        run.right(aiming(0.003, 0.0));
    }
    assert!(run.moves() > 1, "0.001 away");
}

#[test]
fn a_failed_move_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.result = Sent::Failed;
    run.right(centered());
    run.result = Sent::Done;
    run.right(centered());
    run.right(centered());
    assert_eq!(run.moves(), 2);
}

#[test]
fn select_secondary_and_middle_press_and_release_the_left_right_and_middle_button() {
    let mut run = Run::new();
    run.right(centered());
    run.right(HandInput { select: 1.0, ..centered() });
    run.right(HandInput { select: 0.4, ..centered() });
    run.right(HandInput { secondary: ButtonState::Pressed, ..centered() });
    run.right(centered());
    run.right(HandInput { middle: ButtonState::Pressed, ..centered() });
    run.right(centered());
    let buttons: Vec<Action> =
        run.sent.iter().copied().filter(|sent| !matches!(sent, Action::Move(_))).collect();
    assert_eq!(
        buttons,
        [
            Action::Press(Button::Left),
            Action::Release(Button::Left),
            Action::Press(Button::Right),
            Action::Release(Button::Right),
            Action::Press(Button::Middle),
            Action::Release(Button::Middle),
        ]
    );
}

#[test]
fn a_select_held_while_the_ray_enters_the_screen_presses_nothing() {
    let mut run = Run::new();
    run.right(beside());
    run.right(HandInput { select: 1.0, ..beside() });
    run.right(HandInput { select: 1.0, ..centered() });
    assert_eq!(run.count(Action::Press(Button::Left)), 0);
}

#[test]
fn the_left_button_is_released_once_when_the_ray_leaves_the_screen() {
    let mut run = Run::new();
    run.hold_left();
    run.right(HandInput { select: 1.0, ..beside() });
    run.right(HandInput { select: 1.0, ..beside() });
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn the_left_button_is_released_once_when_the_pointing_hand_changes() {
    let mut run = Run::new();
    run.hold_left();
    let held = HandInput { select: 1.0, ..centered() };
    run.frame(HandInput { select: 1.0, ..HandInput::default() }, held, closed());
    run.frame(HandInput { select: 1.0, ..HandInput::default() }, held, closed());
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn the_left_button_is_released_once_when_the_aim_pose_is_lost() {
    let mut run = Run::new();
    run.hold_left();
    run.right(HandInput { select: 1.0, ..HandInput::default() });
    run.right(HandInput { select: 1.0, ..HandInput::default() });
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn the_left_button_is_released_once_when_the_menu_opens() {
    let mut run = Run::new();
    run.hold_left();
    let menu_on_left = Sight { menu: Some(Hand::Left), ..closed() };
    let held = HandInput { select: 1.0, ..centered() };
    run.frame(HandInput::default(), held, menu_on_left);
    run.frame(HandInput::default(), held, menu_on_left);
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn the_left_button_is_released_once_at_the_session_s_end() {
    let mut run = Run::new();
    run.hold_left();
    run.finish();
    run.finish();
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn a_failed_release_is_sent_again_in_the_next_frame() {
    let mut run = Run::new();
    run.hold_left();
    run.result = Sent::Failed;
    run.right(HandInput { select: 1.0, ..HandInput::default() });
    run.result = Sent::Done;
    run.right(HandInput { select: 1.0, ..HandInput::default() });
    run.right(HandInput { select: 1.0, ..HandInput::default() });
    assert_eq!(run.count(Action::Release(Button::Left)), 2);
}

#[test]
fn the_wheel_turns_12_notches_up_in_a_second_and_never_while_middle_is_pressed() {
    let up = Vector2f { x: 0.0, y: 1.0 };
    let turned = |hand: HandInput| {
        let mut run = Run::new();
        for _ in 0..91 {
            run.right(hand);
        }
        run.notches()
    };
    let notches = turned(HandInput { scroll: up, ..centered() });
    assert!((notches - 12).abs() <= 1, "{notches}");
    let middle = HandInput { scroll: up, middle: ButtonState::Pressed, ..centered() };
    assert_eq!(turned(middle), 0, "middle pressed");
    assert_eq!(turned(HandInput { scroll: up, ..beside() }), 0, "a ray beside the screen");
}
