//! The mouse calls: the foreground window of a program, the root owner of a window and of the
//! window at a desktop point, the pointer's position, the virtual desktop's rectangle and the input
//! sent through `SendInput`.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOTOWNER, GetAncestor, GetCursorPos, GetSystemMetrics, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WHEEL_DELTA, WindowFromPoint,
};

use super::cursor::foreground_window;
use super::window_copy::program_of;
use crate::mouse::{Button, Desktop, Event, MouseError, is_named};

/// The size of `INPUT` as `SendInput` takes it, for a mouse input and a keyboard input alike.
pub(super) const INPUT_SIZE: i32 = 40;
const _: () = assert!(INPUT_SIZE.cast_unsigned() as usize == size_of::<INPUT>());

/// The wheel turn of one notch, as a wheel input's `mouseData` counts it.
const NOTCH: i32 = 120;
const _: () = assert!(NOTCH.cast_unsigned() == WHEEL_DELTA);

/// The foreground window when its process's program file is named `program`.
pub(crate) fn foreground_window_of_program(program: &str) -> Option<HWND> {
    let window = foreground_window()?;
    program_of(window).is_some_and(|file| is_named(&file, program)).then_some(window)
}

/// The root owner of the visible window that `WindowFromPoint` finds at the desktop point `point`,
/// or `None` when no window holds it.
pub(crate) fn root_owner_at(point: (i32, i32)) -> Option<HWND> {
    let (x, y) = point;
    // SAFETY: WindowFromPoint takes the point by value and has no pointer arguments.
    let window = unsafe { WindowFromPoint(POINT { x, y }) };
    if window.is_invalid() {
        return None;
    }
    root_owner(window)
}

/// The window at the end of the chain of parents and owners of `window`, which is `window` itself
/// for a top-level window that no window owns. `None` for a window gone since the caller read it.
pub(crate) fn root_owner(window: HWND) -> Option<HWND> {
    // SAFETY: GetAncestor has no pointer arguments, and a window gone since the read gives null.
    let root = unsafe { GetAncestor(window, GA_ROOTOWNER) };
    (!root.is_invalid()).then_some(root)
}

/// The mouse pointer's position on the desktop, in the thread's DPI awareness.
pub(crate) fn cursor_position() -> Result<(i32, i32), MouseError> {
    let mut point = POINT::default();
    // SAFETY: `point` is a writable local.
    match unsafe { GetCursorPos(&raw mut point) } {
        Ok(()) => Ok((point.x, point.y)),
        Err(source) => Err(MouseError::Cursor(source)),
    }
}

/// The virtual desktop. A failed read gives 0 for its value, which the coordinates refuse.
pub(crate) fn virtual_desktop() -> Desktop {
    // SAFETY: GetSystemMetrics has no pointer arguments.
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments.
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments.
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    // SAFETY: GetSystemMetrics has no pointer arguments.
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    Desktop { left, top, width, height }
}

/// Puts `event` into the system's mouse input stream.
pub(crate) fn send(event: Event) -> Result<(), MouseError> {
    let inputs = [input(event)];
    // SAFETY: `inputs` holds one mouse input of `INPUT_SIZE` bytes.
    let sent = unsafe { SendInput(&inputs, INPUT_SIZE) };
    if sent == 1 {
        return Ok(());
    }
    Err(MouseError::Send(windows::core::Error::from_thread()))
}

/// The `INPUT` of `event`.
fn input(event: Event) -> INPUT {
    let (dx, dy, data, flags) = match event {
        Event::Move { x, y } => {
            (x, y, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
        }
        Event::Press(button) => (0, 0, 0, pressed(button)),
        Event::Release(button) => (0, 0, 0, released(button)),
        Event::Wheel(notches) => {
            let delta = i32::from(notches).saturating_mul(NOTCH);
            (0, 0, delta.cast_unsigned(), MOUSEEVENTF_WHEEL)
        }
    };
    let mouse = MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    INPUT { r#type: INPUT_MOUSE, Anonymous: INPUT_0 { mi: mouse } }
}

const fn pressed(button: Button) -> MOUSE_EVENT_FLAGS {
    match button {
        Button::Left => MOUSEEVENTF_LEFTDOWN,
        Button::Right => MOUSEEVENTF_RIGHTDOWN,
    }
}

const fn released(button: Button) -> MOUSE_EVENT_FLAGS {
    match button {
        Button::Left => MOUSEEVENTF_LEFTUP,
        Button::Right => MOUSEEVENTF_RIGHTUP,
    }
}

#[cfg(test)]
mod launcher_tests;
#[cfg(test)]
mod tests;
