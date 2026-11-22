//! Tests of the turn key input and of its foreground check. None of them sends input.

use super::keyboard;
use crate::turn::{KeyEvent, Stroke, TurnError, TurnKey, send};

/// A program file name that no process has.
const NO_PROGRAM: &str = "arcane-eye-launch-test-no-program.exe";

/// `KEYEVENTF_EXTENDEDKEY` (0x1) with `KEYEVENTF_SCANCODE` (0x8).
const DOWN_FLAGS: u32 = 0x9;

/// The flags of the press with `KEYEVENTF_KEYUP` (0x2).
const UP_FLAGS: u32 = 0xB;

#[test]
fn each_turn_key_goes_out_as_its_extended_scan_code_with_the_key_up_flag_only_on_the_release() {
    for (key, scan_code) in [(TurnKey::Left, 0x53_u16), (TurnKey::Right, 0x4F)] {
        let down = keyboard(KeyEvent { key, stroke: Stroke::Down });
        assert_eq!(down.wScan, scan_code, "{key:?} down");
        assert_eq!(down.dwFlags.0, DOWN_FLAGS, "{key:?} down");
        let up = keyboard(KeyEvent { key, stroke: Stroke::Up });
        assert_eq!(up.wScan, scan_code, "{key:?} up");
        assert_eq!(up.dwFlags.0, UP_FLAGS, "{key:?} up");
    }
}

#[test]
fn a_press_refuses_a_foreground_window_of_another_program_and_sends_nothing() {
    for key in [TurnKey::Left, TurnKey::Right] {
        let mut sent = 0_u32;
        let result = send(NO_PROGRAM, KeyEvent { key, stroke: Stroke::Down }, |_| {
            sent = sent.saturating_add(1);
            Ok(())
        });
        let refused = matches!(result, Err(TurnError::NotForeground { program: NO_PROGRAM }));
        assert!(refused, "{key:?}: {result:?}");
        assert_eq!(sent, 0, "{key:?} sent input");
    }
}

#[test]
fn a_release_goes_out_while_the_foreground_window_is_another_program_s() {
    let mut sent = Vec::new();
    let release = KeyEvent { key: TurnKey::Right, stroke: Stroke::Up };
    let result = send(NO_PROGRAM, release, |event| {
        sent.push(event);
        Ok(())
    });
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(sent, [release]);
}
