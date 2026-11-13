//! The launch app's unsafe edge: the message box that shows the player a failure.

#![expect(unsafe_code, reason = "the message box call is a Windows call")]

use tracing::warn;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
use windows::core::{HSTRING, PCWSTR, w};

const MESSAGE_TITLE: PCWSTR = w!("Arcane Eye VR");

/// Shows `text` in an error message box and waits until the player closes it.
pub(crate) fn show_error(text: &str) {
    let text = HSTRING::from(text);
    // SAFETY: both strings are NUL-terminated wide strings that outlive the call, and the box has
    // no owner window.
    let pressed = unsafe { MessageBoxW(None, &text, MESSAGE_TITLE, MB_OK | MB_ICONERROR) };
    if pressed.0 == 0 {
        let error = windows::core::Error::from_thread();
        warn!(error = %error, "the error message box failed to show");
    }
}
