//! Tests of the cursor calls that read windows. None of them moves the cursor.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP,
};
use windows::core::w;

use super::cursor::{PhysicalPixels, client_area, foreground_window_of};
use crate::cursor::ClientArea;

/// The screen rectangle of the test window. Each of the four numbers differs from the others, so a
/// swapped field shows.
const WINDOW: ClientArea = ClientArea { left: 100, top: 200, width: 320, height: 180 };

/// A hidden window of this thread. Dropping it destroys the window.
struct TestWindow(HWND);

impl TestWindow {
    /// A hidden popup window, which has no border, so its client area is its entire rectangle.
    fn create(rect: ClientArea) -> Self {
        // SAFETY: STATIC is a system class, and every pointer argument is a live string or null.
        let window = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("arcane-eye-launch-test-cursor"),
                WS_POPUP,
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                None,
                None,
                None,
                None,
            )
        };
        Self(window.expect("create the test window"))
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // Windows destroys a window that this call leaves when its thread exits.
        // SAFETY: this thread created the window, and only this drop destroys it.
        unsafe { DestroyWindow(self.0) }.unwrap_or_default();
    }
}

#[test]
fn the_client_area_of_a_borderless_window_is_its_screen_rectangle() {
    let _physical = PhysicalPixels::start();
    let window = TestWindow::create(WINDOW);
    assert_eq!(client_area(window.0).expect("read the client area"), WINDOW);
}

#[test]
fn a_process_id_that_owns_no_window_has_no_foreground_window() {
    // Windows gives process ids that are multiples of 4, so no process has this id.
    assert_eq!(foreground_window_of(u32::MAX), None);
}
