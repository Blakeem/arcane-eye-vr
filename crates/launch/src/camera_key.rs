//! The game's camera keys: the press and the release of the keys that turn and pan the camera,
//! sent to the game window through `SendInput`, as a keyboard sends them.
//!
//! Each key goes out as its scan code, so the keyboard layout does not change it. A turn key also
//! carries the extended-key flag, the input a measured trial turned the camera with, since Delete
//! and End sit in the extended key set. Windows gives input to the foreground window, so a press
//! first checks that the foreground window belongs to a process of the game's program file and
//! sends nothing otherwise. A release goes out whichever window is in the foreground, since a press
//! holds its key down in the system's input state and only the release ends it.

use crate::game_dir::EXE_NAME;
use crate::raw::camera_key as raw_camera_key;
use crate::raw::mouse as raw_mouse;

/// A key of the game's default bindings that turns its camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnKey {
    /// The Delete key.
    Left,
    /// The End key.
    Right,
}

/// A key of the game's default bindings that pans its camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanKey {
    /// The W key.
    Forward,
    /// The S key.
    Backward,
    /// The A key.
    Left,
    /// The D key.
    Right,
}

/// A key of the game's default bindings that moves its camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraKey {
    Turn(TurnKey),
    Pan(PanKey),
}

impl CameraKey {
    /// The key's set 1 scan code, without the extended-key prefix.
    pub(crate) const fn scan_code(self) -> u16 {
        match self {
            Self::Turn(TurnKey::Left) => 0x53,
            Self::Turn(TurnKey::Right) => 0x4F,
            Self::Pan(PanKey::Forward) => 0x11,
            Self::Pan(PanKey::Backward) => 0x1F,
            Self::Pan(PanKey::Left) => 0x1E,
            Self::Pan(PanKey::Right) => 0x20,
        }
    }
}

/// Whether a key input presses its key down or lets it up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stroke {
    Down,
    Up,
}

/// One key input as `SendInput` takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub(crate) key: CameraKey,
    pub(crate) stroke: Stroke,
}

/// Why a camera key input was not sent.
#[derive(Debug, thiserror::Error)]
pub enum CameraKeyError {
    #[error(
        "the camera key needs the game window in the foreground, and the foreground window is not \
         a window of {program}"
    )]
    NotForeground { program: &'static str },
    #[error(
        "Windows refused the key input. A game that runs as administrator refuses the input of a \
         launch app that does not"
    )]
    Send(#[source] windows::core::Error),
}

/// Presses `key` down in the game window.
///
/// # Errors
///
/// Returns [`CameraKeyError::NotForeground`] when the foreground window is not the game's, and
/// [`CameraKeyError::Send`] when Windows refused the input.
pub fn press(key: CameraKey) -> Result<(), CameraKeyError> {
    send(EXE_NAME, KeyEvent { key, stroke: Stroke::Down }, raw_camera_key::send)
}

/// Lets `key` up, whichever window is in the foreground.
///
/// # Errors
///
/// Returns [`CameraKeyError::Send`] when Windows refused the input.
pub fn release(key: CameraKey) -> Result<(), CameraKeyError> {
    send(EXE_NAME, KeyEvent { key, stroke: Stroke::Up }, raw_camera_key::send)
}

/// Sends `event` with `send_event`. A press goes out only while the foreground window belongs to a
/// process whose program file is named `program`, and a release goes out always.
pub(crate) fn send(
    program: &'static str,
    event: KeyEvent,
    send_event: impl FnOnce(KeyEvent) -> Result<(), CameraKeyError>,
) -> Result<(), CameraKeyError> {
    if event.stroke == Stroke::Down && raw_mouse::foreground_window_of_program(program).is_none() {
        return Err(CameraKeyError::NotForeground { program });
    }
    send_event(event)
}
