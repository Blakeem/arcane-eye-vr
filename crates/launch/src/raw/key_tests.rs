//! Tests of the key post against stand-in child processes that own one window and write each key
//! message the window gets to standard output.

use std::io::{self, BufRead as _, Write as _};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_KEYDOWN, WM_KEYUP, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP, WS_VISIBLE,
};

use super::tests::stand_in::{
    CLOSE_TIMEOUT, HIDDEN, StandIn, WindowStyles, close_on_request, run_stand_in,
};
use crate::key;
use crate::stop::{self, ProcessState};

/// The libtest names of the tests that a child process runs as a stand-in for the game.
const VISIBLE_STAND_IN: &str = "raw::key_tests::stand_in_visible_window_reports_keys";
const HIDDEN_STAND_IN: &str = "raw::key_tests::stand_in_hidden_window_reports_keys";

/// The text before the message, `wParam` and `lParam` of each key message a stand-in reports.
const KEY_LINE: &str = "key message:";

/// The styles of a visible stand-in window that takes no focus and stays off the taskbar.
const VISIBLE: WindowStyles = (
    WINDOW_STYLE(WS_POPUP.0 | WS_VISIBLE.0),
    WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
);

extern "system" fn report_keys(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message == WM_KEYDOWN || message == WM_KEYUP {
        let mut stdout = io::stdout();
        let line = format!("{KEY_LINE} {message:#06x} {:#x} {:#010x}", w.0, l.0);
        writeln!(stdout, "{line}").and_then(|()| stdout.flush()).expect("report the key");
        return LRESULT(0);
    }
    close_on_request(window, message, w, l)
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_visible_window_reports_keys() {
    run_stand_in("arcane-eye-launch-test-visible-keys", Some(report_keys), VISIBLE);
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_hidden_window_reports_keys() {
    run_stand_in("arcane-eye-launch-test-hidden-keys", Some(report_keys), HIDDEN);
}

/// Closes the stand-in and returns the key messages it reported. The close reaches the window after
/// every message posted before it, so the lines hold every key the window got.
fn keys_until_closed(stand_in: &mut StandIn) -> Vec<String> {
    let pid = stand_in.child.id();
    assert_eq!(stop::close(pid, CLOSE_TIMEOUT).expect("close the stand-in"), ProcessState::Exited);
    let lines = (&mut stand_in.stdout).lines().map_while(Result::ok);
    let keys = lines.filter_map(|line| line.split_once(KEY_LINE).map(|(_, key)| key.to_owned()));
    keys.collect()
}

#[test]
fn the_space_key_reaches_a_visible_window_as_a_key_down_then_a_key_up() {
    let mut stand_in = StandIn::start(VISIBLE_STAND_IN, "key-visible");
    assert_eq!(key::post_space(stand_in.child.id()), 1, "one visible window got the key");
    let keys = keys_until_closed(&mut stand_in);
    assert_eq!(keys, [" 0x0100 0x20 0x00390001", " 0x0101 0x20 0xc0390001"]);
}

#[test]
fn a_hidden_window_gets_no_key() {
    let mut stand_in = StandIn::start(HIDDEN_STAND_IN, "key-hidden");
    assert_eq!(key::post_space(stand_in.child.id()), 0, "a hidden window got the key");
    assert_eq!(keys_until_closed(&mut stand_in), [""; 0]);
}
