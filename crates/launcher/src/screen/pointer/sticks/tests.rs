//! Tests of the keys and the wheel turns of the two sticks, with each hand's stick acting or not.

use device_profile::profile::Hand;
use launch::camera_key::{CameraKey, PanKey, TurnKey};
use openxr::Vector2f;
use xr::input::{HandInput, Input};

use super::{Stick, Sticks};
use crate::screen::pointer::model::{Action, Sent};

/// The time of one frame at 90 hertz, in seconds.
const FRAME: f32 = 0.011;

const A: CameraKey = CameraKey::Pan(PanKey::Left);
const D: CameraKey = CameraKey::Pan(PanKey::Right);
const W: CameraKey = CameraKey::Pan(PanKey::Forward);
const S: CameraKey = CameraKey::Pan(PanKey::Backward);
const DELETE: CameraKey = CameraKey::Turn(TurnKey::Left);
const END: CameraKey = CameraKey::Turn(TurnKey::Right);

/// The input of a frame with the left stick at `left` and the right stick at `right`.
fn sticks_at(left: (f32, f32), right: (f32, f32)) -> Input {
    let hand = |(x, y): (f32, f32)| HandInput { scroll: Vector2f { x, y }, ..HandInput::default() };
    Input { left: hand(left), right: hand(right), head: None }
}

/// The sticks, every action sent and what each send returns.
struct Run {
    sticks: Sticks,
    sent: Vec<Action>,
    result: Sent,
}

impl Run {
    fn new() -> Self {
        Self { sticks: Sticks::default(), sent: Vec::new(), result: Sent::Done }
    }

    /// Runs one frame of `input` with both sticks acting.
    fn acting(&mut self, input: Input) {
        self.frame(input, |_| Stick::Acts);
    }

    /// Runs one frame of `input` with each hand's stick acting as `stick` says.
    fn frame(&mut self, input: Input, stick: impl Fn(Hand) -> Stick) {
        let (sent, result) = (&mut self.sent, self.result);
        self.sticks.update(&input, stick, FRAME, &mut |action| {
            sent.push(action);
            result
        });
    }

    fn finish(&mut self) {
        let (sent, result) = (&mut self.sent, self.result);
        self.sticks.release_all(&mut |action| {
            sent.push(action);
            result
        });
    }

    fn notches(&self) -> i32 {
        let turns = self.sent.iter().filter_map(|sent| match sent {
            Action::Wheel(notches) => Some(i32::from(*notches)),
            Action::Move(_)
            | Action::Press(_)
            | Action::Release(_)
            | Action::PressKey(_)
            | Action::ReleaseKey(_) => None,
        });
        turns.sum()
    }
}

#[test]
fn the_left_stick_s_left_and_right_press_past_0_5_release_within_0_3_and_cross_in_one_frame() {
    let mut run = Run::new();
    for x in [0.4, 0.6, 0.4, 0.2, -0.6, -0.4, 0.6] {
        run.acting(sticks_at((x, 0.0), (0.0, 0.0)));
    }
    let expected = [
        Action::PressKey(D),
        Action::ReleaseKey(D),
        Action::PressKey(A),
        Action::ReleaseKey(A),
        Action::PressKey(D),
    ];
    assert_eq!(run.sent, expected);
}

#[test]
fn the_left_stick_s_up_and_down_press_past_0_5_release_within_0_3_and_cross_in_one_frame() {
    let mut run = Run::new();
    for y in [0.4, 0.6, 0.4, 0.2, -0.6, -0.4, 0.6] {
        run.acting(sticks_at((0.0, y), (0.0, 0.0)));
    }
    let expected = [
        Action::PressKey(W),
        Action::ReleaseKey(W),
        Action::PressKey(S),
        Action::ReleaseKey(S),
        Action::PressKey(W),
    ];
    assert_eq!(run.sent, expected);
}

#[test]
fn a_diagonal_left_stick_holds_a_key_on_each_axis_at_once() {
    let mut run = Run::new();
    for _ in 0..10 {
        run.acting(sticks_at((-0.8, -0.8), (0.0, 0.0)));
    }
    run.acting(sticks_at((0.0, 0.0), (0.0, 0.0)));
    let expected =
        [Action::PressKey(A), Action::PressKey(S), Action::ReleaseKey(A), Action::ReleaseKey(S)];
    assert_eq!(run.sent, expected);
}

#[test]
fn a_value_that_is_not_finite_releases_the_held_key_and_presses_none() {
    let mut run = Run::new();
    run.acting(sticks_at((0.8, 0.8), (0.8, 0.0)));
    run.acting(sticks_at((f32::NAN, f32::INFINITY), (f32::NEG_INFINITY, f32::NAN)));
    let expected = [
        Action::PressKey(D),
        Action::PressKey(W),
        Action::PressKey(END),
        Action::ReleaseKey(D),
        Action::ReleaseKey(W),
        Action::ReleaseKey(END),
    ];
    assert_eq!(run.sent, expected);
}

#[test]
fn the_right_stick_holds_a_turn_key_and_turns_the_wheel_at_once() {
    let mut run = Run::new();
    for _ in 0..91 {
        run.acting(sticks_at((0.0, 0.0), (-0.8, 1.0)));
    }
    let keys: Vec<Action> =
        run.sent.iter().copied().filter(|sent| !matches!(sent, Action::Wheel(_))).collect();
    assert_eq!(keys, [Action::PressKey(DELETE)]);
    assert!((run.notches() - 12).abs() <= 1, "{} notches in a second", run.notches());
}

#[test]
fn a_stopped_stick_releases_its_keys_once_and_turns_no_wheel_while_the_other_stick_acts() {
    let mut run = Run::new();
    let pushed = sticks_at((0.8, 0.8), (0.8, 1.0));
    run.acting(pushed);
    let left_only = |hand| if hand == Hand::Left { Stick::Acts } else { Stick::Stopped };
    run.sent.clear();
    for _ in 0..91 {
        run.frame(pushed, left_only);
    }
    assert_eq!(run.sent, [Action::ReleaseKey(END)]);
    let right_only = |hand| if hand == Hand::Right { Stick::Acts } else { Stick::Stopped };
    run.sent.clear();
    run.frame(pushed, right_only);
    assert_eq!(run.sent, [Action::ReleaseKey(D), Action::ReleaseKey(W), Action::PressKey(END)]);
}

#[test]
fn a_failed_release_stays_held_and_is_sent_again_and_a_refused_press_is_sent_again() {
    let mut run = Run::new();
    run.result = Sent::Failed;
    run.acting(sticks_at((0.8, 0.0), (0.0, 0.0)));
    run.result = Sent::Done;
    run.acting(sticks_at((0.8, 0.0), (0.0, 0.0)));
    run.result = Sent::Failed;
    run.acting(sticks_at((0.0, 0.0), (0.0, 0.0)));
    run.result = Sent::Done;
    run.acting(sticks_at((0.0, 0.0), (0.0, 0.0)));
    run.acting(sticks_at((0.0, 0.0), (0.0, 0.0)));
    let expected =
        [Action::PressKey(D), Action::PressKey(D), Action::ReleaseKey(D), Action::ReleaseKey(D)];
    assert_eq!(run.sent, expected);
}

#[test]
fn the_session_s_end_releases_each_held_key_once() {
    let mut run = Run::new();
    run.acting(sticks_at((-0.8, 0.8), (0.8, 0.0)));
    run.sent.clear();
    run.finish();
    run.finish();
    let expected = [Action::ReleaseKey(A), Action::ReleaseKey(W), Action::ReleaseKey(END)];
    assert_eq!(run.sent, expected);
}
