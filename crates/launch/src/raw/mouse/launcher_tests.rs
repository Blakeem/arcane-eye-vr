//! Tests of the launcher window's search, its move and press checks, its copy and its order
//! against stand-in child processes that own one window or a window and a popup it owns. No test
//! sends input. Each move and press goes to a recording stand-in of `SendInput`.

use std::io::{self, Write as _};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, IsWindowVisible, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetWindowPos, WINDOW_EX_STYLE, WINDOW_STYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_POPUP, WS_VISIBLE, WindowFromPoint,
};

use crate::cursor::ClientArea;
use crate::launcher_window::{self, LauncherWindow};
use crate::mouse::{Button, ClientShares, Event, MouseError};
use crate::raw::cursor::{PhysicalPixels, client_area};
use crate::raw::tests::stand_in::{
    CLOSE_TIMEOUT, HIDDEN, OWNED_TITLE_END, StandIn, WindowStyles, close_on_request, exe_name,
    run_stand_in, run_stand_in_owning,
};
use crate::raw::top_level_windows;
use crate::raw::window_copy::window_title;
use crate::raw::window_copy_tests::{input_desktop_visible, is_topmost};
use crate::window_copy::WindowCopyError;

/// The libtest names of the tests that a child process runs as a stand-in for the launcher.
const VISIBLE_STAND_IN: &str = "raw::mouse::launcher_tests::stand_in_visible_launcher_window";
const HIDDEN_STAND_IN: &str = "raw::mouse::launcher_tests::stand_in_hidden_launcher_window";
const OWNER_STAND_IN: &str = "raw::mouse::launcher_tests::stand_in_launcher_window_with_a_popup";

/// The class name of the stand-in whose window owns a popup, which titles its main window.
const OWNER_CLASS: &str = "arcane-eye-launch-test-owner-launcher";

/// The styles of a visible stand-in window that takes no focus and stays off the taskbar.
const VISIBLE: WindowStyles = (
    WINDOW_STYLE(WS_POPUP.0 | WS_VISIBLE.0),
    WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
);

/// The screen rectangle of each test's launcher stand-in, apart from every other one, since the
/// tests run at once. `OTHER` covers no part of `PRESSED`, `POPUP` lies over `OWNER`, and the
/// covering stand-in of the move test takes the rectangle `MOVED` too.
const FOUND: ClientArea = ClientArea { left: 300, top: 300, width: 160, height: 120 };
const WENT_HIDDEN: ClientArea = ClientArea { left: 600, top: 300, width: 160, height: 120 };
const OWNER: ClientArea = ClientArea { left: 900, top: 300, width: 160, height: 120 };
const POPUP: ClientArea = ClientArea { left: 980, top: 360, width: 60, height: 40 };
const PRESSED: ClientArea = ClientArea { left: 300, top: 460, width: 160, height: 120 };
const OTHER: ClientArea = ClientArea { left: 600, top: 460, width: 160, height: 120 };
const ORDERED: ClientArea = ClientArea { left: 900, top: 460, width: 160, height: 120 };
const MOVED: ClientArea = ClientArea { left: 300, top: 620, width: 160, height: 120 };
const GONE: ClientArea = ClientArea { left: 600, top: 620, width: 160, height: 120 };

/// The middle of a client area, the place each move test asks for.
const MIDDLE: ClientShares = ClientShares { x: 0.5, y: 0.5 };

/// The distance of the point a press checks from the top left corner of a client area, in pixels.
const INSIDE: i32 = 10;

/// The time between two reads of an order change that the stand-in's thread takes later.
const POLL: Duration = Duration::from_millis(20);

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_visible_launcher_window() {
    run_stand_in("arcane-eye-launch-test-visible-launcher", Some(close_on_request), VISIBLE);
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_hidden_launcher_window() {
    run_stand_in("arcane-eye-launch-test-hidden-launcher", Some(close_on_request), HIDDEN);
}

#[test]
#[ignore = "runs only as the child process of another test"]
fn stand_in_launcher_window_with_a_popup() {
    run_stand_in_owning(OWNER_CLASS, Some(close_on_request), VISIBLE, Some(VISIBLE));
}

/// Whether `window` is visible.
fn visible(window: HWND) -> bool {
    // SAFETY: IsWindowVisible has no pointer arguments.
    unsafe { IsWindowVisible(window) }.as_bool()
}

/// The visible window of `stand_in` whose title `is_title` accepts.
fn window_of(stand_in: &StandIn, is_title: impl Fn(&str) -> bool) -> HWND {
    let windows = top_level_windows(Some(stand_in.child.id()));
    let mut found = windows.into_iter().filter(|&window| visible(window));
    found.find(|&window| is_title(&window_title(window))).expect("the stand-in's visible window")
}

/// Places `window` at `rect` above every window that is not topmost and returns the desktop point
/// `INSIDE` pixels into it.
fn place_topmost(window: HWND, rect: ClientArea) -> (i32, i32) {
    let ClientArea { left, top, width, height } = rect;
    // SAFETY: SetWindowPos has no pointer arguments, and the stand-in's window lives while it runs.
    unsafe { SetWindowPos(window, Some(HWND_TOPMOST), left, top, width, height, SWP_NOACTIVATE) }
        .expect("place the stand-in's window");
    // The corner read back, since a scaled display may place a window of an unaware process apart
    // from the rectangle in physical pixels.
    let area = client_area(window).expect("the stand-in's client area");
    let inside = |side: i32| side.checked_add(INSIDE).expect("a point on the desktop");
    (inside(area.left), inside(area.top))
}

/// Starts the visible stand-in of `test_name`, places its window at `rect` above every window that
/// is not topmost and returns the stand-in, its window and the desktop point `INSIDE` pixels into
/// the window.
fn placed_stand_in(test_name: &str, rect: ClientArea) -> (StandIn, HWND, (i32, i32)) {
    let stand_in = StandIn::start(VISIBLE_STAND_IN, test_name);
    let window = window_of(&stand_in, |_| true);
    let point = place_topmost(window, rect);
    (stand_in, window, point)
}

/// Whether `holds` turned true before [`CLOSE_TIMEOUT`] passed.
fn turns_true(holds: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    while !holds() {
        if started.elapsed() > CLOSE_TIMEOUT {
            return false;
        }
        // thread::sleep is banned for game threads. Parking waits the same, and an early wake polls
        // once more.
        thread::park_timeout(POLL);
    }
    true
}

#[test]
fn a_visible_window_is_found_by_its_program_file_s_name() {
    let _physical = PhysicalPixels::start();
    let (_stand_in, _window, _point) = placed_stand_in("launcher-found", FOUND);
    let found =
        launcher_window::find_of(&exe_name("launcher-found")).expect("the stand-in's window");
    let (width, height) = found.size();
    assert!(width > 0 && height > 0, "the placed window's client area is {width}x{height}");
    assert_eq!(launcher_window::find_of(&exe_name("launcher-not-started")), None);
}

#[test]
fn a_hidden_window_is_not_found() {
    let _stand_in = StandIn::start(HIDDEN_STAND_IN, "launcher-hidden");
    assert_eq!(launcher_window::find_of(&exe_name("launcher-hidden")), None);
}

/// Presses the left button of `launcher` with the pointer at `point` and returns the result and
/// the events that reached the recording stand-in of `SendInput`.
fn press_at(launcher: &LauncherWindow, point: (i32, i32)) -> (Result<(), MouseError>, Vec<Event>) {
    let mut sent = Vec::new();
    let result = launcher.press_with(
        Button::Left,
        || Ok(point),
        |event| {
            sent.push(event);
            Ok(())
        },
    );
    (result, sent)
}

#[test]
fn a_press_is_refused_with_no_input_when_another_window_lies_at_the_pointer_s_position() {
    let _physical = PhysicalPixels::start();
    let (_launcher_stand_in, _launcher, on_launcher) = placed_stand_in("launcher-press", PRESSED);
    let (_other_stand_in, _other, on_other) = placed_stand_in("launcher-press-other", OTHER);
    let launcher =
        launcher_window::find_of(&exe_name("launcher-press")).expect("the stand-in's window");
    let (covered, refused_events) = press_at(&launcher, on_other);
    assert!(matches!(covered, Err(MouseError::Covered { .. })), "{covered:?}");
    assert!(refused_events.is_empty(), "a refused press sent {refused_events:?}");
    let (pressed, pressed_events) = press_at(&launcher, on_launcher);
    assert!(pressed.is_ok(), "{pressed:?}");
    assert_eq!(pressed_events, [Event::Press(Button::Left)]);
}

/// Moves the pointer to `place` of `launcher` and returns the result and the events that reached
/// the recording stand-in of `SendInput`.
fn move_in(launcher: &LauncherWindow, place: ClientShares) -> (Result<(), MouseError>, Vec<Event>) {
    let mut sent = Vec::new();
    let result = launcher.move_with(place, |event| {
        sent.push(event);
        Ok(())
    });
    (result, sent)
}

#[test]
fn a_move_is_refused_with_no_input_when_another_window_covers_its_place() {
    let _physical = PhysicalPixels::start();
    let (_launcher_stand_in, _launcher, _point) = placed_stand_in("launcher-move", MOVED);
    let launcher =
        launcher_window::find_of(&exe_name("launcher-move")).expect("the stand-in's window");
    let (moved, moved_events) = move_in(&launcher, MIDDLE);
    assert!(moved.is_ok(), "{moved:?}");
    assert!(matches!(moved_events.as_slice(), [Event::Move { .. }]), "{moved_events:?}");
    let (_cover_stand_in, _cover, _on_cover) = placed_stand_in("launcher-move-cover", MOVED);
    let (covered, covered_events) = move_in(&launcher, MIDDLE);
    assert!(matches!(covered, Err(MouseError::Covered { .. })), "{covered:?}");
    assert!(covered_events.is_empty(), "a refused move sent {covered_events:?}");
}

#[test]
fn a_move_or_a_press_on_a_window_that_closed_is_refused_as_gone_with_no_input() {
    let _physical = PhysicalPixels::start();
    let (stand_in, _window, _point) = placed_stand_in("launcher-gone", GONE);
    let launcher =
        launcher_window::find_of(&exe_name("launcher-gone")).expect("the stand-in's window");
    // The drop ends the stand-in and waits for it, so its window is destroyed.
    drop(stand_in);
    let (pressed, press_events) = press_at(&launcher, (0, 0));
    assert!(matches!(pressed, Err(MouseError::WindowGone)), "{pressed:?}");
    assert!(press_events.is_empty(), "a refused press sent {press_events:?}");
    let (moved, move_events) = move_in(&launcher, MIDDLE);
    assert!(matches!(moved, Err(MouseError::WindowGone)), "{moved:?}");
    assert!(move_events.is_empty(), "a refused move sent {move_events:?}");
}

#[test]
fn a_press_on_a_popup_that_the_launcher_window_owns_is_sent() {
    let _physical = PhysicalPixels::start();
    let stand_in = StandIn::start(OWNER_STAND_IN, "launcher-popup");
    let main = window_of(&stand_in, |title| title == OWNER_CLASS);
    let popup = window_of(&stand_in, |title| title.ends_with(OWNED_TITLE_END));
    place_topmost(main, OWNER);
    let on_popup = place_topmost(popup, POPUP);
    let (x, y) = on_popup;
    // SAFETY: WindowFromPoint takes the point by value and has no pointer arguments.
    let at_point = unsafe { WindowFromPoint(POINT { x, y }) };
    assert_eq!(at_point, popup, "the popup lies at the pointer's position");
    let launcher =
        launcher_window::find_of(&exe_name("launcher-popup")).expect("the stand-in's window");
    let (pressed, pressed_events) = press_at(&launcher, on_popup);
    assert!(pressed.is_ok(), "{pressed:?}");
    assert_eq!(pressed_events, [Event::Press(Button::Left)]);
}

/// The place of `window` in the order of the top-level windows, 0 for the topmost.
fn order_of(window: HWND) -> Option<usize> {
    top_level_windows(None).into_iter().position(|listed| listed == window)
}

#[test]
fn the_end_of_the_topmost_order_places_the_window_below_others_and_a_copy_makes_it_topmost() {
    if !input_desktop_visible() {
        let reason = "skipped: this session has no visible input desktop to copy a window from";
        writeln!(io::stderr(), "{reason}").expect("log the skip");
        return;
    }
    let _physical = PhysicalPixels::start();
    let (_stand_in, window, _point) = placed_stand_in("launcher-order", ORDERED);
    let other_stand_in = StandIn::start(VISIBLE_STAND_IN, "launcher-order-other");
    let other = window_of(&other_stand_in, |_| true);
    let launcher =
        launcher_window::find_of(&exe_name("launcher-order")).expect("the stand-in's window");
    launcher.end_topmost().expect("end the topmost order");
    let below_other = || {
        let (launcher_place, other_place) = (order_of(window), order_of(other));
        !is_topmost(window)
            && launcher_place.zip(other_place).is_some_and(|(lower, upper)| lower > upper)
    };
    assert!(turns_true(below_other), "the window stays topmost or above the other window");
    launcher.copy().expect("copy the stand-in's window");
    assert!(turns_true(|| is_topmost(window)), "the copied window is not topmost");
}

#[test]
fn the_copy_of_a_window_that_went_hidden_fails_as_hidden() {
    let _physical = PhysicalPixels::start();
    let (_stand_in, window, _point) = placed_stand_in("launcher-went-hidden", WENT_HIDDEN);
    let launcher =
        launcher_window::find_of(&exe_name("launcher-went-hidden")).expect("the stand-in's window");
    let flags = SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE;
    // SAFETY: SetWindowPos has no pointer arguments, and the stand-in's window lives while it runs.
    unsafe { SetWindowPos(window, None, 0, 0, 0, 0, flags) }.expect("hide the stand-in's window");
    let copy = launcher.copy();
    assert!(matches!(copy, Err(WindowCopyError::Hidden { .. })), "{copy:?}");
}
