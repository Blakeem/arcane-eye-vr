//! The turn key call: the key input sent through `SendInput`.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput, VIRTUAL_KEY,
};

use super::mouse::INPUT_SIZE;
use crate::turn::{KeyEvent, Stroke, TurnError};

/// Puts `event` into the system's keyboard input stream.
pub(crate) fn send(event: KeyEvent) -> Result<(), TurnError> {
    let inputs = [INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: keyboard(event) } }];
    // SAFETY: `inputs` holds one keyboard input of `INPUT_SIZE` bytes.
    let sent = unsafe { SendInput(&inputs, INPUT_SIZE) };
    if sent == 1 {
        return Ok(());
    }
    Err(TurnError::Send(windows::core::Error::from_thread()))
}

/// The keyboard input of `event`. `KEYEVENTF_SCANCODE` makes Windows ignore the virtual key.
fn keyboard(event: KeyEvent) -> KEYBDINPUT {
    let stroke = match event.stroke {
        Stroke::Down => KEYBD_EVENT_FLAGS(0),
        Stroke::Up => KEYEVENTF_KEYUP,
    };
    KEYBDINPUT {
        wVk: VIRTUAL_KEY(0),
        wScan: event.key.scan_code(),
        dwFlags: KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY | stroke,
        time: 0,
        dwExtraInfo: 0,
    }
}

#[cfg(test)]
mod tests;
