//! The mouse cursor's move to a position in the client area of the game window.
//!
//! Windows moves the cursor of the input desktop only, so the game window must be in the
//! foreground. Positions are physical pixels of the client area, the pixels the game renders.

use std::fmt;

use tracing::info;

use crate::raw::cursor::{self as raw_cursor, AreaFailure, PhysicalPixels};

/// A position in a window's client area, in whole pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPoint {
    pub x: u32,
    pub y: u32,
}

impl fmt::Display for ClientPoint {
    /// The position as a command line writes it: `<x>,<y>`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{},{}", self.x, self.y)
    }
}

/// A window's client area on the screen, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClientArea {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// Why the cursor did not move.
#[derive(Debug, thiserror::Error)]
pub enum CursorError {
    #[error(
        "the cursor position needs the game window in the foreground, and the foreground window is \
         not a window of the game process {pid}"
    )]
    NotForeground { pid: u32 },
    #[error(
        "the cursor position {point} lies outside the {width}x{height} client area of the game window"
    )]
    Outside { point: ClientPoint, width: i32, height: i32 },
    #[error("could not read the client area of the game window")]
    ClientArea(#[source] windows::core::Error),
    #[error("could not map the client area of the game window to the screen")]
    ToScreen,
    #[error("could not move the cursor to the screen position {x},{y}")]
    Move { x: i32, y: i32, source: windows::core::Error },
}

impl From<AreaFailure> for CursorError {
    fn from(failure: AreaFailure) -> Self {
        match failure {
            AreaFailure::Rect(source) => Self::ClientArea(source),
            AreaFailure::ToScreen => Self::ToScreen,
        }
    }
}

/// Moves the cursor to `point` in the client area of the foreground window, which must belong to
/// process `pid`.
///
/// # Errors
///
/// Returns [`CursorError::NotForeground`] when the foreground window belongs to another process,
/// [`CursorError::Outside`] when `point` lies outside the client area, and the failed call
/// otherwise.
pub fn place(pid: u32, point: ClientPoint) -> Result<(), CursorError> {
    let _physical = PhysicalPixels::start();
    let Some(window) = raw_cursor::foreground_window_of(pid) else {
        return Err(CursorError::NotForeground { pid });
    };
    let area = raw_cursor::client_area(window)?;
    let (x, y) = screen_point(area, point)?;
    raw_cursor::move_cursor(x, y)?;
    info!(pid, point = %point, x, y, "moved the cursor in the game window");
    Ok(())
}

/// The screen position of `point` in `area`, once `point` lies inside it.
pub(crate) fn screen_point(
    area: ClientArea,
    point: ClientPoint,
) -> Result<(i32, i32), CursorError> {
    let outside = || CursorError::Outside { point, width: area.width, height: area.height };
    let x = i32::try_from(point.x).ok().filter(|x| *x < area.width).ok_or_else(outside)?;
    let y = i32::try_from(point.y).ok().filter(|y| *y < area.height).ok_or_else(outside)?;
    let screen_x = area.left.checked_add(x).ok_or_else(outside)?;
    let screen_y = area.top.checked_add(y).ok_or_else(outside)?;
    Ok((screen_x, screen_y))
}

#[cfg(test)]
mod tests {
    use super::{ClientArea, ClientPoint, CursorError, screen_point};

    const AREA: ClientArea = ClientArea { left: -1920, top: 40, width: 3840, height: 2160 };

    #[test]
    fn a_point_inside_the_client_area_moves_by_the_area_s_screen_corner() {
        assert_eq!(screen_point(AREA, ClientPoint { x: 0, y: 0 }).expect("inside"), (-1920, 40));
        let corner = ClientPoint { x: 3839, y: 2159 };
        assert_eq!(screen_point(AREA, corner).expect("inside"), (1919, 2199));
    }

    #[test]
    fn a_point_on_or_past_the_right_or_bottom_edge_lies_outside_the_client_area() {
        for (x, y) in [(3840, 0), (0, 2160), (u32::MAX, 5), (5, u32::MAX)] {
            let point = ClientPoint { x, y };
            let error = screen_point(AREA, point).expect_err("outside");
            let named = matches!(
                error,
                CursorError::Outside { point: named, width: 3840, height: 2160 } if named == point
            );
            assert!(named, "{x},{y}: {error:?}");
        }
        let message = screen_point(AREA, ClientPoint { x: 3840, y: 7 }).expect_err("outside");
        assert_eq!(
            message.to_string(),
            "the cursor position 3840,7 lies outside the 3840x2160 client area of the game window"
        );
    }

    #[test]
    fn an_empty_client_area_holds_no_point() {
        let empty = ClientArea { left: 0, top: 0, width: 0, height: 0 };
        let error = screen_point(empty, ClientPoint { x: 0, y: 0 }).expect_err("outside");
        assert!(matches!(error, CursorError::Outside { width: 0, height: 0, .. }), "{error:?}");
    }
}
