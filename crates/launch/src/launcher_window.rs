//! The Larian launcher's window: its search, the copy of its client area from the screen and the
//! mouse input sent to it.
//!
//! The launcher window is the visible top-level window that is not minimized, has the largest
//! client area that is not empty and belongs to a process whose program file is named
//! `LariLauncher.exe`. An empty client area holds no picture to copy and no place to click. Windows
//! gives a mouse button to the window under the pointer, so a move and a press first check that the
//! window at their target point has the launcher window's root owner and send nothing otherwise.
//! The root owner ends the chain of parents and owners, so the launcher's own dialogs and popups,
//! which the copy shows, take the input too. A move's target point is the place it moves to, and a
//! press's target point is the pointer's position. A release goes out through `mouse.rs`,
//! whichever window is there. The copy of a hidden or minimized window fails, since the screen
//! shows other windows at its client area. The copy makes the window topmost without activating
//! it, as `window_copy.rs` does, so no other window covers it, and the end of that order places it
//! below every other window. The headset session's frame loop and its copy thread call these
//! steps, so the order change is posted to the launcher's thread, and a copy may read the screen
//! before a later frame composes the new order.

use std::path::PathBuf;
use std::ptr;

use windows::Win32::Foundation::HWND;

use crate::mouse::{self, Button, ClientShares, Event, MouseError};
use crate::raw::cursor::PhysicalPixels;
use crate::raw::mouse as raw_mouse;
use crate::raw::window_copy as raw_copy;
use crate::window_copy::{Delivery, WindowCopyError, WindowPicture};

/// The program file of the Larian launcher, which Steam starts for the game.
pub(crate) const PROGRAM: &str = "LariLauncher.exe";

/// How a top-level window shows on the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Showing {
    Visible,
    Minimized,
    Hidden,
}

/// A top-level window as the search reads it. `program` is `None` for a window whose process
/// cannot be opened and for a window that is not [`Showing::Visible`], whose program the search
/// does not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate<W> {
    pub(crate) window: W,
    pub(crate) showing: Showing,
    pub(crate) program: Option<PathBuf>,
    /// The client area's width and height in pixels.
    pub(crate) area: (i32, i32),
}

/// The launcher window found by a search, and its client area's width and height in pixels at the
/// search. Two values are equal when they name the same window, whatever size each search read.
#[derive(Clone, Debug)]
pub struct LauncherWindow {
    /// The window handle's value, which names the window on every thread, so the frame loop hands
    /// the window to the copy thread.
    window: usize,
    size: (i32, i32),
}

impl PartialEq for LauncherWindow {
    fn eq(&self, other: &Self) -> bool {
        self.window == other.window
    }
}

impl Eq for LauncherWindow {}

/// Finds the launcher window of a process of the program file `LariLauncher.exe`.
pub fn find() -> Option<LauncherWindow> {
    find_of(PROGRAM)
}

/// Finds the launcher window of a process of the program file `program`.
pub(crate) fn find_of(program: &str) -> Option<LauncherWindow> {
    // Physical pixels, so the client areas of windows on displays of different scales compare.
    let _physical = PhysicalPixels::try_start();
    let candidates = raw_copy::candidates();
    let chosen = choose(&candidates, program)?;
    Some(LauncherWindow { window: chosen.window.0.expose_provenance(), size: chosen.area })
}

/// The candidate that is [`Showing::Visible`], belongs to a process of the program file `program`
/// in any case and has the largest client area that is not empty, the first of equal areas.
pub(crate) fn choose<'a, W>(
    candidates: &'a [Candidate<W>],
    program: &str,
) -> Option<&'a Candidate<W>> {
    let pixels = |candidate: &Candidate<W>| {
        let (width, height) = candidate.area;
        i64::from(width).saturating_mul(i64::from(height))
    };
    let mut chosen: Option<&Candidate<W>> = None;
    for candidate in candidates {
        let (width, height) = candidate.area;
        let has_area = width > 0 && height > 0;
        let of_program =
            candidate.program.as_deref().is_some_and(|file| mouse::is_named(file, program));
        let larger = chosen.is_none_or(|best| pixels(candidate) > pixels(best));
        if candidate.showing == Showing::Visible && has_area && of_program && larger {
            chosen = Some(candidate);
        }
    }
    chosen
}

impl LauncherWindow {
    /// The client area's width and height in physical pixels when the search found the window.
    pub const fn size(&self) -> (i32, i32) {
        self.size
    }

    /// The window's handle.
    fn handle(&self) -> HWND {
        HWND(ptr::with_exposed_provenance_mut(self.window))
    }

    /// Makes the window topmost without activating it and copies its client area from the screen.
    ///
    /// # Errors
    ///
    /// Returns [`WindowCopyError::Hidden`] for a window that is hidden, minimized or closed, whose
    /// client area on the screen shows other windows, [`WindowCopyError::Awareness`] when the thread
    /// could not take physical pixels, and the failed call otherwise, such as the topmost post or
    /// the screen copy.
    pub fn copy(&self) -> Result<WindowPicture, WindowCopyError> {
        let window = self.handle();
        let title = raw_copy::window_title(window);
        if raw_copy::showing(window) != Showing::Visible {
            return Err(WindowCopyError::Hidden { title });
        }
        let Some(_physical) = PhysicalPixels::try_start() else {
            return Err(WindowCopyError::Awareness);
        };
        raw_copy::make_topmost(window, &title, Delivery::Post)?;
        raw_copy::copy_from_screen(window, &title)
    }

    /// Ends the topmost order that [`LauncherWindow::copy`] gave the window and places it below
    /// every other window, so the game's window lies above it.
    ///
    /// # Errors
    ///
    /// Returns [`WindowCopyError::EndTopmost`] when the call failed, such as for a window that
    /// closed.
    pub fn end_topmost(&self) -> Result<(), WindowCopyError> {
        let window = self.handle();
        let title = raw_copy::window_title(window);
        raw_copy::end_topmost(window, &title, Delivery::Post)
    }

    /// Moves the mouse pointer to `place` in the window's client area.
    ///
    /// # Errors
    ///
    /// Returns [`MouseError::Covered`] when a window of another root owner holds the place,
    /// [`MouseError::WindowGone`] when the window closed, [`MouseError::Place`] when a share lies
    /// outside 0 to 1, [`MouseError::Awareness`] when the thread could not take physical pixels,
    /// and the failed step otherwise.
    pub fn move_to(&self, place: ClientShares) -> Result<(), MouseError> {
        self.move_with(place, raw_mouse::send)
    }

    /// Sends the move to `place` with `send_event` while the window at that place has the root
    /// owner of this window.
    pub(crate) fn move_with(
        &self,
        place: ClientShares,
        send_event: impl FnOnce(Event) -> Result<(), MouseError>,
    ) -> Result<(), MouseError> {
        let Some(_physical) = PhysicalPixels::try_start() else {
            return Err(MouseError::Awareness);
        };
        let owner = self.root_owner()?;
        let pixel = mouse::place_pixel(self.handle(), place)?;
        holds(owner, pixel)?;
        send_event(mouse::move_event_at(pixel)?)
    }

    /// Presses `button` where the mouse pointer is.
    ///
    /// # Errors
    ///
    /// Returns [`MouseError::Covered`] when a window of another root owner holds the pointer's
    /// position, [`MouseError::WindowGone`] when the window closed, and the failed step otherwise.
    pub fn press(&self, button: Button) -> Result<(), MouseError> {
        self.press_with(button, raw_mouse::cursor_position, raw_mouse::send)
    }

    /// Sends the press of `button` with `send_event` while the window at the pointer's position,
    /// which `cursor` reads, has the root owner of this window.
    pub(crate) fn press_with(
        &self,
        button: Button,
        cursor: impl FnOnce() -> Result<(i32, i32), MouseError>,
        send_event: impl FnOnce(Event) -> Result<(), MouseError>,
    ) -> Result<(), MouseError> {
        let Some(_physical) = PhysicalPixels::try_start() else {
            return Err(MouseError::Awareness);
        };
        let owner = self.root_owner()?;
        holds(owner, cursor()?)?;
        send_event(Event::Press(button))
    }

    /// The window's root owner, the end of its chain of parents and owners. A window that closed
    /// has none and refuses with [`MouseError::WindowGone`].
    fn root_owner(&self) -> Result<HWND, MouseError> {
        raw_mouse::root_owner(self.handle()).ok_or(MouseError::WindowGone)
    }
}

/// Refuses with [`MouseError::Covered`] unless the window at the desktop point `pixel` has the root
/// owner `owner`, so it is the launcher window, one of its child windows or a window it owns, such
/// as a dialog or a list's popup.
fn holds(owner: HWND, pixel: (i32, i32)) -> Result<(), MouseError> {
    if raw_mouse::root_owner_at(pixel) == Some(owner) {
        return Ok(());
    }
    let (x, y) = pixel;
    Err(MouseError::Covered { x, y })
}

#[cfg(test)]
mod tests;
