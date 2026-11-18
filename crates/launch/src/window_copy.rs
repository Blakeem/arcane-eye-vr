//! The copy of another process's top-level window from the screen. The window is found by its title
//! and by the name of its process's program file.
//!
//! A screen copy reads the desktop of the calling thread, so the window must be on that desktop.
//! The window is made topmost without activation before the copy, so no other window covers it and
//! the keyboard focus stays where it was. The window stays topmost after the copy, until it closes.
//! A window lowered after each copy is raised again before the next, and the desktop may compose
//! that order on a later frame than the copy reads (untested). The copy is the window's client area
//! in physical pixels, without a frame.

use std::path::Path;

use tracing::info;

use crate::raw::cursor::PhysicalPixels;
use crate::raw::window_copy as raw_copy;

/// The bytes of one pixel of the copied bitmap: blue, green, red and an unused byte.
pub(crate) const BITMAP_PIXEL_BYTES: usize = 4;

/// A window's picture: rows top first, 3 bytes per pixel in the order red, green, blue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPicture {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

/// Why a found window was not copied.
#[derive(Debug, thiserror::Error)]
pub enum WindowCopyError {
    #[error("could not make the window \"{title}\" topmost")]
    Topmost { title: String, source: windows::core::Error },
    #[error("could not read the client area of the window \"{title}\"")]
    Rect { title: String, source: windows::core::Error },
    #[error(
        "the window \"{title}\" is {width}x{height} pixels, so it holds no pixel to copy. A \
         minimized window has a client area of 0x0, so restore the window if it is minimized"
    )]
    Empty { title: String, width: i32, height: i32 },
    #[error("could not copy the window \"{title}\" from the screen, since {call} failed")]
    Screen { title: String, call: &'static str },
    #[error("could not copy the window \"{title}\" from the screen")]
    Blit { title: String, source: windows::core::Error },
}

/// Finds the first visible top-level window titled `title` of a process whose program file is
/// named `process` with any extension and in any case, makes it topmost without activating it and
/// copies its client area from the screen.
///
/// # Errors
///
/// Returns the failed call of a window that was found but not copied. No such window is `Ok(None)`.
pub fn copy(process: &str, title: &str) -> Result<Option<WindowPicture>, WindowCopyError> {
    let _physical = PhysicalPixels::start();
    let Some(window) = raw_copy::find(title, |program| is_program(program, process)) else {
        return Ok(None);
    };
    raw_copy::make_topmost(window, title)?;
    let picture = raw_copy::copy_from_screen(window, title)?;
    let (width, height) = (picture.width, picture.height);
    info!(process, title, width, height, "copied a window from the screen");
    Ok(Some(picture))
}

/// Whether the program file `program` is named `process` with any extension and in any case.
pub(crate) fn is_program(program: &Path, process: &str) -> bool {
    program.file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case(process))
}

/// The red, green and blue bytes of each blue, green, red and unused byte of `bitmap`.
pub(crate) fn bitmap_to_rgb(bitmap: &[u8]) -> Vec<u8> {
    let (pixels, _no_remainder) = bitmap.as_chunks::<BITMAP_PIXEL_BYTES>();
    let mut rgb = Vec::with_capacity(pixels.len().saturating_mul(3));
    for &[blue, green, red, _unused] in pixels {
        rgb.extend([red, green, blue]);
    }
    rgb
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{bitmap_to_rgb, is_program};

    #[test]
    fn a_program_file_matches_its_name_without_extension_in_any_case() {
        let program = Path::new(r"C:\Steam\steamapps\common\SteamVR\bin\win64\vrcompositor.exe");
        assert!(is_program(program, "vrcompositor"));
        assert!(is_program(program, "VRCompositor"));
        assert!(is_program(Path::new("vrcompositor"), "vrcompositor"));
        assert!(!is_program(program, "vrcompositor.exe"));
        assert!(!is_program(program, "vrcompositor2"));
        assert!(!is_program(Path::new(r"C:\vrcompositor\vrserver.exe"), "vrcompositor"));
        assert!(!is_program(Path::new(""), "vrcompositor"));
    }

    #[test]
    fn each_bitmap_pixel_becomes_red_green_blue_and_loses_its_unused_byte() {
        let bitmap = [30, 20, 10, 255, 60, 50, 40, 0];
        assert_eq!(bitmap_to_rgb(&bitmap), [10, 20, 30, 40, 50, 60]);
        assert_eq!(bitmap_to_rgb(&[]), [0_u8; 0]);
    }
}
