//! The status panel: one line of text centered on a panel of [`STATUS_WIDTH`] by
//! [`STATUS_HEIGHT`] pixels, in the colors of the menu's title.

use gfx::canvas::{Align, Canvas, CanvasError, Line};
use gfx::layer::Rect;

use crate::painter::{BACKGROUND, TITLE_COLOR};

/// The width of the status panel in pixels, four times its height.
pub const STATUS_WIDTH: u16 = 1024;

/// The height of the status panel in pixels.
pub const STATUS_HEIGHT: u16 = 256;

const TEXT_HEIGHT: u16 = 72;

/// The entire status panel.
const PANEL: Rect = Rect { x: 0, y: 0, width: STATUS_WIDTH, height: STATUS_HEIGHT };

/// The pixels of a status panel that shows `text`, red, green, blue and alpha, rows top first.
///
/// # Errors
///
/// Returns the canvas's failed GDI call.
pub fn paint(text: &str) -> Result<Vec<u8>, CanvasError> {
    let mut canvas = Canvas::new(STATUS_WIDTH, STATUS_HEIGHT)?;
    canvas.fill(PANEL, BACKGROUND)?;
    let line = Line { text, height: TEXT_HEIGHT, color: TITLE_COLOR, align: Align::Center };
    canvas.draw_text(PANEL, &line)?;
    Ok(canvas.pixels()?.to_vec())
}

#[cfg(test)]
mod tests {
    use super::{STATUS_HEIGHT, STATUS_WIDTH, paint};

    /// The background as red, green, blue and alpha.
    const BACKGROUND: [u8; 4] = [24, 26, 32, 255];

    #[test]
    fn a_status_panel_holds_its_text_in_the_middle_on_the_background() {
        let pixels = paint("Starting Baldur's Gate 3").expect("the status panel");
        let width = usize::from(STATUS_WIDTH);
        assert_eq!(pixels.len(), width * usize::from(STATUS_HEIGHT) * 4);
        let pixels = pixels.as_chunks::<4>().0;
        let row = |y: u16| &pixels[usize::from(y) * width..(usize::from(y) + 1) * width];
        assert!(row(0).iter().all(|pixel| *pixel == BACKGROUND), "the top row is background");
        let middle = row(STATUS_HEIGHT / 2);
        assert!(middle.iter().any(|pixel| *pixel != BACKGROUND), "the text crosses the middle row");
        assert_eq!(middle[0], BACKGROUND, "the text is centered, so the left edge is background");
        assert!(pixels.iter().all(|pixel| pixel[3] == 255), "every pixel is opaque");
    }

    #[test]
    fn an_empty_status_panel_is_background_only() {
        let pixels = paint("").expect("the status panel");
        assert!(pixels.as_chunks::<4>().0.iter().all(|pixel| *pixel == BACKGROUND));
    }
}
