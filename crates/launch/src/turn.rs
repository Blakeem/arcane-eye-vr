//! The game's camera turn keys: the press and the release of the key that turns the camera left
//! and of the key that turns it right, sent to the game window through `SendInput`, as a keyboard
//! sends them.
//!
//! Each key goes out as its scan code with the extended-key flag, the input a measured trial
//! turned the camera with, so the keyboard layout does not change it. Windows gives input to the
//! foreground window, so a press first checks that the foreground window belongs to a process of
//! the game's program file and sends nothing otherwise. A release goes out whichever window is in
//! the foreground, since a press holds its key down in the system's input state and only the
//! release ends it.

use crate::game_dir::EXE_NAME;
use crate::raw::mouse as raw_mouse;
use crate::raw::turn as raw_turn;

/// A key of the game's default bindings that turns its camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnKey {
    /// The Delete key.
    Left,
    /// The End key.
    Right,
}

impl TurnKey {
    /// The key's scan code, sent with the extended-key flag.
    pub(crate) const fn scan_code(self) -> u16 {
        match self {
            Self::Left => 0x53,
            Self::Right => 0x4F,
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
    pub(crate) key: TurnKey,
    pub(crate) stroke: Stroke,
}

/// Why a turn key input was not sent.
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error(
        "the turn key needs the game window in the foreground, and the foreground window is not a \
         window of {program}"
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
/// Returns [`TurnError::NotForeground`] when the foreground window is not the game's, and
/// [`TurnError::Send`] when Windows refused the input.
pub fn press(key: TurnKey) -> Result<(), TurnError> {
    send(EXE_NAME, KeyEvent { key, stroke: Stroke::Down }, raw_turn::send)
}

/// Lets `key` up, whichever window is in the foreground.
///
/// # Errors
///
/// Returns [`TurnError::Send`] when Windows refused the input.
pub fn release(key: TurnKey) -> Result<(), TurnError> {
    send(EXE_NAME, KeyEvent { key, stroke: Stroke::Up }, raw_turn::send)
}

/// Sends `event` with `send_event`. A press goes out only while the foreground window belongs to a
/// process whose program file is named `program`, and a release goes out always.
pub(crate) fn send(
    program: &'static str,
    event: KeyEvent,
    send_event: impl FnOnce(KeyEvent) -> Result<(), TurnError>,
) -> Result<(), TurnError> {
    if event.stroke == Stroke::Down && raw_mouse::foreground_window_of_program(program).is_none() {
        return Err(TurnError::NotForeground { program });
    }
    send_event(event)
}
