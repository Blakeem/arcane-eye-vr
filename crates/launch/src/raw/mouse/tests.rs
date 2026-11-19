//! Tests of the mouse calls that read windows. None of them sends input.

use super::foreground_window_of_program;
use crate::mouse::{Button, ClientShares, Event, MouseError, Request, send};

/// A program file name that no process has.
const NO_PROGRAM: &str = "arcane-eye-launch-test-no-program.exe";

#[test]
fn a_program_that_owns_no_window_has_no_foreground_window() {
    assert_eq!(foreground_window_of_program(NO_PROGRAM), None);
}

#[test]
fn a_move_a_press_and_a_wheel_turn_refuse_a_foreground_window_of_another_program_and_send_nothing()
{
    let requests = [
        Request::Move(ClientShares { x: 0.5, y: 0.5 }),
        Request::Press(Button::Left),
        Request::Wheel(1),
    ];
    for request in requests {
        let mut sent = 0_u32;
        let result = send(NO_PROGRAM, request, |_| {
            sent = sent.saturating_add(1);
            Ok(())
        });
        let refused = matches!(result, Err(MouseError::NotForeground { program: NO_PROGRAM }));
        assert!(refused, "{request:?}: {result:?}");
        assert_eq!(sent, 0, "{request:?} sent input");
    }
}

#[test]
fn a_release_goes_out_while_the_foreground_window_is_another_program_s() {
    let mut sent = Vec::new();
    let result = send(NO_PROGRAM, Request::Release(Button::Right), |event| {
        sent.push(event);
        Ok(())
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(sent, [Event::Release(Button::Right)]);
}
