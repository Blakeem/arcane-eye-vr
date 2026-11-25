//! Tests of the camera key input and of its foreground check. None of them sends input.

use super::keyboard;
use crate::camera_key::{CameraKey, CameraKeyError, KeyEvent, PanKey, Stroke, TurnKey, send};

/// A program file name that no process has.
const NO_PROGRAM: &str = "arcane-eye-launch-test-no-program.exe";

/// `KEYEVENTF_SCANCODE` (0x8).
const SCAN_CODE: u32 = 0x8;

/// `KEYEVENTF_EXTENDEDKEY` (0x1).
const EXTENDED: u32 = 0x1;

/// `KEYEVENTF_KEYUP` (0x2).
const KEY_UP: u32 = 0x2;

/// Fails the test unless `key` goes out as `scan_code` with the flags `flags` on its press and with
/// `KEYEVENTF_KEYUP` added on its release.
fn assert_input(key: CameraKey, scan_code: u16, flags: u32) {
    let down = keyboard(KeyEvent { key, stroke: Stroke::Down });
    assert_eq!(down.wScan, scan_code, "{key:?} down");
    assert_eq!(down.dwFlags.0, flags, "{key:?} down");
    let up = keyboard(KeyEvent { key, stroke: Stroke::Up });
    assert_eq!(up.wScan, scan_code, "{key:?} up");
    assert_eq!(up.dwFlags.0, flags | KEY_UP, "{key:?} up");
}

#[test]
fn each_turn_key_goes_out_as_its_extended_scan_code_with_the_key_up_flag_only_on_the_release() {
    for (key, scan_code) in [(TurnKey::Left, 0x53_u16), (TurnKey::Right, 0x4F)] {
        assert_input(CameraKey::Turn(key), scan_code, SCAN_CODE | EXTENDED);
    }
}

#[test]
fn each_pan_key_goes_out_as_its_scan_code_without_the_extended_key_flag() {
    let keys = [
        (PanKey::Forward, 0x11_u16),
        (PanKey::Backward, 0x1F),
        (PanKey::Left, 0x1E),
        (PanKey::Right, 0x20),
    ];
    for (key, scan_code) in keys {
        assert_input(CameraKey::Pan(key), scan_code, SCAN_CODE);
    }
}

#[test]
fn a_press_refuses_a_foreground_window_of_another_program_and_sends_nothing() {
    for key in [CameraKey::Turn(TurnKey::Left), CameraKey::Pan(PanKey::Forward)] {
        let mut sent = 0_u32;
        let result = send(NO_PROGRAM, KeyEvent { key, stroke: Stroke::Down }, |_| {
            sent = sent.saturating_add(1);
            Ok(())
        });
        let refused = matches!(result, Err(CameraKeyError::NotForeground { program: NO_PROGRAM }));
        assert!(refused, "{key:?}: {result:?}");
        assert_eq!(sent, 0, "{key:?} sent input");
    }
}

#[test]
fn a_release_goes_out_while_the_foreground_window_is_another_program_s() {
    for key in [CameraKey::Turn(TurnKey::Right), CameraKey::Pan(PanKey::Backward)] {
        let mut sent = Vec::new();
        let release = KeyEvent { key, stroke: Stroke::Up };
        let result = send(NO_PROGRAM, release, |event| {
            sent.push(event);
            Ok(())
        });
        assert!(result.is_ok(), "{key:?}: {result:?}");
        assert_eq!(sent, [release], "{key:?}");
    }
}
