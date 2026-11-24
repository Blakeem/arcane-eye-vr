//! The pointer's input to the Larian launcher's window, whose picture the screen quad shows during
//! the start wait.

use launch::mouse::Button;
use launch::turn::TurnKey;
use openxr::Vector2f;
use xr::input::{ButtonState, HandInput};

use super::{Action, Run, Sent, Sight, Window, aiming, centered, closed, stick};

/// A run whose input goes to the launcher window.
fn on_launcher() -> Run {
    Run { window: Window::Launcher, ..Run::new() }
}

/// The actions sent other than moves.
fn clicks(run: &Run) -> Vec<Action> {
    run.sent.iter().copied().filter(|sent| !matches!(sent, Action::Move(_))).collect()
}

#[test]
fn on_the_launcher_select_presses_and_releases_the_left_button() {
    let mut run = on_launcher();
    run.right(centered());
    run.right(HandInput { select: 1.0, ..centered() });
    run.right(centered());
    assert_eq!(clicks(&run), [Action::Press(Button::Left), Action::Release(Button::Left)]);
    assert!(run.moves() >= 1, "the pointer moved to the place: {:?}", run.sent);
}

#[test]
fn on_the_launcher_a_select_pressed_in_a_frame_whose_move_was_refused_presses_nothing() {
    // A covering window refuses the move, so the pointer still lies at the place of the move before.
    let mut run = on_launcher();
    run.right(centered());
    let covered = aiming(0.5, 0.0);
    run.move_result = Sent::Failed;
    run.right(HandInput { select: 1.0, ..covered });
    run.move_result = Sent::Done;
    run.right(HandInput { select: 1.0, ..covered });
    assert_eq!(run.moves(), 3, "the refused move was sent again: {:?}", run.sent);
    assert!(clicks(&run).is_empty(), "{:?}", run.sent);
    run.right(covered);
    run.right(HandInput { select: 1.0, ..covered });
    assert_eq!(clicks(&run), [Action::Press(Button::Left)], "a press after the move reached it");
}

#[test]
fn on_the_launcher_secondary_and_scroll_send_nothing() {
    let mut run = on_launcher();
    let up = Vector2f { x: 0.0, y: 1.0 };
    for right in [
        centered(),
        HandInput { secondary: ButtonState::Pressed, ..centered() },
        centered(),
        HandInput { scroll: up, ..centered() },
        HandInput { scroll: Vector2f { x: 0.0, y: -1.0 }, ..centered() },
        stick(0.8, centered()),
        stick(-0.8, centered()),
    ] {
        for _ in 0..30 {
            run.right(right);
        }
    }
    assert!(clicks(&run).is_empty(), "{:?}", run.sent);
}

#[test]
fn the_held_left_button_is_released_once_when_the_launcher_window_closes() {
    let mut run = on_launcher();
    run.hold_left();
    // A closed window leaves the status panel, so the screen shows no quad and the game takes the
    // input again.
    run.window = Window::Game;
    let status = Sight { surface: None, ..closed() };
    let held = HandInput { select: 1.0, ..centered() };
    run.frame(HandInput::default(), held, status);
    run.frame(HandInput::default(), held, status);
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
}

#[test]
fn the_held_left_button_is_released_once_when_the_game_s_frame_replaces_the_launcher() {
    let mut run = on_launcher();
    run.hold_left();
    run.window = Window::Game;
    let held = HandInput { select: 1.0, ..centered() };
    run.right(held);
    run.right(held);
    assert_eq!(run.count(Action::Release(Button::Left)), 1);
    assert_eq!(run.count(Action::Press(Button::Left)), 1, "the held trigger pressed nothing");
}

#[test]
fn a_turn_key_held_on_the_game_is_released_when_the_launcher_shows() {
    let mut run = Run::new();
    run.right(stick(0.8, centered()));
    run.window = Window::Launcher;
    run.right(stick(0.8, centered()));
    run.right(stick(0.8, centered()));
    let turns = [Action::PressTurn(TurnKey::Right), Action::ReleaseTurn(TurnKey::Right)];
    assert_eq!(run.turns(), turns);
}
