//! The game's mouse: pointer moves, button presses and releases and wheel turns sent to the game
//! window through `SendInput`, as a mouse sends them.
//!
//! Windows gives input to the foreground window, so a move, a press and a wheel turn first check
//! that the foreground window belongs to a process of the game's program file and send nothing
//! otherwise. A release goes out whichever window is in the foreground, since a press holds its
//! button down in the system's input state and only the release ends it. A move names a place in
//! the game window's client area as shares of its width and height. Windows takes an absolute move
//! as coordinates from 0 to 65535 across the virtual desktop.

use std::path::Path;

use windows::Win32::Foundation::HWND;

use crate::cursor::ClientArea;
use crate::game_dir::EXE_NAME;
use crate::raw::cursor::{self as raw_cursor, AreaFailure, PhysicalPixels};
use crate::raw::mouse as raw_mouse;

/// The largest absolute coordinate of a move, at the virtual desktop's right or bottom edge.
const ABSOLUTE_MAX: i64 = 65_535;

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// A place in a window's client area as shares of its width and its height from its top left
/// corner, each from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClientShares {
    pub x: f32,
    pub y: f32,
}

/// The virtual desktop, the rectangle that holds every monitor, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Desktop {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// One mouse input as the caller asks for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Request {
    Move(ClientShares),
    Press(Button),
    Release(Button),
    Wheel(i16),
}

/// One mouse input as `SendInput` takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    /// A move to the absolute coordinates `x`, `y` of the virtual desktop.
    Move {
        x: i32,
        y: i32,
    },
    Press(Button),
    Release(Button),
    /// A wheel turn by whole notches, forward (up) for a positive count.
    Wheel(i16),
}

/// Why a mouse input was not sent.
#[derive(Debug, thiserror::Error)]
pub enum MouseError {
    #[error(
        "the mouse input needs the game window in the foreground, and the foreground window is not \
         a window of {program}"
    )]
    NotForeground { program: &'static str },
    #[error("the place {x},{y} lies outside the shares 0 to 1 of the client area")]
    Place { x: f32, y: f32 },
    #[error("could not read the client area of the game window")]
    ClientArea(#[source] windows::core::Error),
    #[error("could not map the client area of the game window to the screen")]
    ToScreen,
    #[error("the client area of the game window is {width}x{height} pixels, so it holds no place")]
    EmptyArea { width: i32, height: i32 },
    #[error("the virtual desktop of {width}x{height} pixels gives no absolute mouse coordinates")]
    Desktop { width: i32, height: i32 },
    #[error(
        "the thread kept its DPI awareness, so the pointer's place on a scaled display is unknown"
    )]
    Awareness,
    #[error(
        "Windows refused the mouse input. A game that runs as administrator refuses the input of a \
         launch app that does not"
    )]
    Send(#[source] windows::core::Error),
}

impl From<AreaFailure> for MouseError {
    fn from(failure: AreaFailure) -> Self {
        match failure {
            AreaFailure::Rect(source) => Self::ClientArea(source),
            AreaFailure::ToScreen => Self::ToScreen,
        }
    }
}

/// Moves the game's mouse pointer to `place` in the client area of the game window.
///
/// # Errors
///
/// Returns [`MouseError::NotForeground`] when the foreground window is not the game's,
/// [`MouseError::Place`] when a share lies outside 0 to 1, [`MouseError::Awareness`] when the
/// thread could not take physical pixels, and the failed step otherwise.
pub fn move_to(place: ClientShares) -> Result<(), MouseError> {
    send(EXE_NAME, Request::Move(place), raw_mouse::send)
}

/// Presses `button` of the game's mouse.
///
/// # Errors
///
/// Returns [`MouseError::NotForeground`] when the foreground window is not the game's, and
/// [`MouseError::Send`] when Windows refused the input.
pub fn press(button: Button) -> Result<(), MouseError> {
    send(EXE_NAME, Request::Press(button), raw_mouse::send)
}

/// Releases `button` of the game's mouse, whichever window is in the foreground.
///
/// # Errors
///
/// Returns [`MouseError::Send`] when Windows refused the input.
pub fn release(button: Button) -> Result<(), MouseError> {
    send(EXE_NAME, Request::Release(button), raw_mouse::send)
}

/// Turns the game's mouse wheel by `notches`, up for a positive count.
///
/// # Errors
///
/// Returns [`MouseError::NotForeground`] when the foreground window is not the game's, and
/// [`MouseError::Send`] when Windows refused the input.
pub fn turn_wheel(notches: i16) -> Result<(), MouseError> {
    send(EXE_NAME, Request::Wheel(notches), raw_mouse::send)
}

/// Sends `request` with `send_event`. A move, a press or a wheel turn goes out only while the
/// foreground window belongs to a process whose program file is named `program`, and a release
/// goes out always.
pub(crate) fn send(
    program: &'static str,
    request: Request,
    send_event: impl FnOnce(Event) -> Result<(), MouseError>,
) -> Result<(), MouseError> {
    let event = match request {
        Request::Move(place) => move_event(game_window(program)?, place)?,
        Request::Press(button) => game_window(program).map(|_| Event::Press(button))?,
        Request::Release(button) => Event::Release(button),
        Request::Wheel(notches) => game_window(program).map(|_| Event::Wheel(notches))?,
    };
    send_event(event)
}

/// The foreground window when a process whose program file is named `program` owns it.
fn game_window(program: &'static str) -> Result<HWND, MouseError> {
    raw_mouse::foreground_window_of_program(program).ok_or(MouseError::NotForeground { program })
}

/// The move of the pointer to `place` in the client area of `window`.
fn move_event(window: HWND, place: ClientShares) -> Result<Event, MouseError> {
    // The frame loop calls this, so the failure goes to the caller's rate-limited warning.
    let Some(_physical) = PhysicalPixels::try_start() else {
        return Err(MouseError::Awareness);
    };
    let area = raw_cursor::client_area(window)?;
    let (x, y) = coordinates(raw_mouse::virtual_desktop(), screen_pixel(area, place)?)?;
    Ok(Event::Move { x, y })
}

/// Whether the program file `program` is named `name`, in any case.
pub(crate) fn is_named(program: &Path, name: &str) -> bool {
    program.file_name().is_some_and(|file| file.eq_ignore_ascii_case(name))
}

/// The screen pixel at `place` in `area`.
pub(crate) fn screen_pixel(
    area: ClientArea,
    place: ClientShares,
) -> Result<(i32, i32), MouseError> {
    let ClientShares { x, y } = place;
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Err(MouseError::Place { x, y });
    }
    let empty = || MouseError::EmptyArea { width: area.width, height: area.height };
    if area.width <= 0 || area.height <= 0 {
        return Err(empty());
    }
    let screen_x = area.left.checked_add(pixel_at(x, area.width)).ok_or_else(empty)?;
    let screen_y = area.top.checked_add(pixel_at(y, area.height)).ok_or_else(empty)?;
    Ok((screen_x, screen_y))
}

/// The pixel at `share` of a line `length` pixels long: the pixel whose span holds the share, and
/// the last pixel for a share of 1. A binary search finds it, since no lint-clean cast turns a float
/// into an integer.
fn pixel_at(share: f32, length: i32) -> i32 {
    let target = f64::from(share) * f64::from(length);
    let (mut low, mut high) = (0_i32, length);
    while high.saturating_sub(low) > 1 {
        let middle = low.midpoint(high);
        if f64::from(middle) <= target {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// The absolute coordinates of the screen pixel `pixel` on `desktop`. A pixel off the desktop gets
/// the coordinates of the nearest edge, where Windows keeps the pointer.
pub(crate) fn coordinates(desktop: Desktop, pixel: (i32, i32)) -> Result<(i32, i32), MouseError> {
    let refused = || MouseError::Desktop { width: desktop.width, height: desktop.height };
    let x = coordinate(pixel.0, desktop.left, desktop.width).ok_or_else(refused)?;
    let y = coordinate(pixel.1, desktop.top, desktop.height).ok_or_else(refused)?;
    Ok((x, y))
}

/// The coordinate of `pixel` on a desktop side that starts at `start` and is `size` pixels long,
/// or `None` for a side of fewer than 2 pixels.
fn coordinate(pixel: i32, start: i32, size: i32) -> Option<i32> {
    let span = i64::from(size).checked_sub(1).filter(|span| *span > 0)?;
    let offset = i64::from(pixel).checked_sub(i64::from(start))?;
    let scaled = offset.checked_mul(ABSOLUTE_MAX)?.checked_div(span)?;
    i32::try_from(scaled.clamp(0, ABSOLUTE_MAX)).ok()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ClientShares, Desktop, MouseError, coordinates, is_named, screen_pixel};
    use crate::cursor::ClientArea;

    const AREA: ClientArea = ClientArea { left: 100, top: 50, width: 1920, height: 1080 };
    const DESKTOP: Desktop = Desktop { left: 0, top: 0, width: 3840, height: 2160 };

    #[test]
    fn the_middle_of_a_client_area_gives_its_pixel_and_absolute_coordinates() {
        let pixel = screen_pixel(AREA, ClientShares { x: 0.5, y: 0.5 }).expect("a place");
        assert_eq!(pixel, (1060, 590));
        let (x, y) = coordinates(DESKTOP, pixel).expect("a desktop");
        // 1060 * 65535 / 3839 = 18095.6 and 590 * 65535 / 2159 = 17909.1.
        assert!((x - 18_095).abs() <= 1 && (y - 17_909).abs() <= 1, "{x}, {y}");
    }

    #[test]
    fn the_shares_0_and_1_give_the_first_and_the_last_pixel() {
        assert_eq!(
            screen_pixel(AREA, ClientShares { x: 0.0, y: 0.0 }).expect("a place"),
            (100, 50)
        );
        let last = screen_pixel(AREA, ClientShares { x: 1.0, y: 1.0 }).expect("a place");
        assert_eq!(last, (2019, 1129));
    }

    #[test]
    fn a_share_outside_0_to_1_or_not_a_number_is_refused() {
        for (x, y) in [(-0.1, 0.5), (0.5, 1.01), (f32::NAN, 0.5)] {
            let error = screen_pixel(AREA, ClientShares { x, y }).expect_err("outside");
            assert!(matches!(error, MouseError::Place { .. }), "{x}, {y}: {error:?}");
        }
    }

    #[test]
    fn an_empty_client_area_holds_no_place() {
        let empty = ClientArea { left: 0, top: 0, width: 0, height: 1080 };
        let error = screen_pixel(empty, ClientShares { x: 0.5, y: 0.5 }).expect_err("empty");
        assert!(matches!(error, MouseError::EmptyArea { width: 0, height: 1080 }), "{error:?}");
    }

    #[test]
    fn the_desktop_s_corners_give_0_and_65535_and_a_pixel_off_it_its_nearest_edge() {
        let desktop = Desktop { left: -1920, top: -200, width: 3840, height: 1280 };
        assert_eq!(coordinates(desktop, (-1920, -200)).expect("a desktop"), (0, 0));
        assert_eq!(coordinates(desktop, (1919, 1079)).expect("a desktop"), (65_535, 65_535));
        assert_eq!(coordinates(desktop, (-5000, 9000)).expect("a desktop"), (0, 65_535));
    }

    #[test]
    fn a_desktop_side_of_fewer_than_2_pixels_gives_no_coordinates() {
        let flat = Desktop { left: 0, top: 0, width: 1, height: 2160 };
        let error = coordinates(flat, (0, 0)).expect_err("too small");
        assert!(matches!(error, MouseError::Desktop { width: 1, height: 2160 }), "{error:?}");
    }

    #[test]
    fn a_program_file_matches_its_file_name_in_any_case() {
        let program = Path::new(r"C:\Games\Baldurs Gate 3\bin\bg3_dx11.exe");
        assert!(is_named(program, "bg3_dx11.exe"));
        assert!(is_named(program, "BG3_DX11.EXE"));
        assert!(!is_named(program, "bg3_dx11"));
        assert!(!is_named(Path::new(r"C:\Games\bg3_dx11.exe\bg3.exe"), "bg3_dx11.exe"));
        assert!(!is_named(Path::new(""), "bg3_dx11.exe"));
    }
}
