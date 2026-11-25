//! The camera key call: the key input sent through `SendInput`.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput, VIRTUAL_KEY,
};

use super::mouse::INPUT_SIZE;
use crate::camera_key::{CameraKey, CameraKeyError, KeyEvent, Stroke};

/// Puts `event` into the system's keyboard input stream.
pub(crate) fn send(event: KeyEvent) -> Result<(), CameraKeyError> {
    let inputs = [INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: keyboard(event) } }];
    // SAFETY: `inputs` holds one keyboard input of `INPUT_SIZE` bytes.
    let sent = unsafe { SendInput(&inputs, INPUT_SIZE) };
    if sent == 1 {
        return Ok(());
    }
    Err(CameraKeyError::Send(windows::core::Error::from_thread()))
}

/// The keyboard input of `event`. `KEYEVENTF_SCANCODE` makes Windows ignore the virtual key.
fn keyboard(event: KeyEvent) -> KEYBDINPUT {
    let stroke = match event.stroke {
        Stroke::Down => KEYBD_EVENT_FLAGS(0),
        Stroke::Up => KEYEVENTF_KEYUP,
    };
    // Delete and End sit in the extended key set, and W, A, S and D do not.
    let set = match event.key {
        CameraKey::Turn(_) => KEYEVENTF_EXTENDEDKEY,
        CameraKey::Pan(_) => KEYBD_EVENT_FLAGS(0),
    };
    KEYBDINPUT {
        wVk: VIRTUAL_KEY(0),
        wScan: event.key.scan_code(),
        dwFlags: KEYEVENTF_SCANCODE | set | stroke,
        time: 0,
        dwExtraInfo: 0,
    }
}

#[cfg(test)]
mod tests;
