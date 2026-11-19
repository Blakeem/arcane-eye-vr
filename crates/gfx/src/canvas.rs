//! The pixel canvas: pixels in memory into which its caller fills rectangles and draws lines of
//! text, given out as red, green, blue and alpha bytes with every alpha 255.
//!
//! GDI draws the text into a DIB section, whose pixels are blue, green, red and an unused byte, so
//! the canvas converts its pixels when its caller reads them after a change.

use crate::layer::Rect;
use crate::raw::canvas::Surface;

/// The bytes of one pixel, in the DIB section and in the pixels the canvas gives out.
pub(crate) const PIXEL_BYTES: usize = 4;

/// A color with no transparency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// Where a line of text sits across its rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A line of text and its look. `height` is the height of its characters in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line<'a> {
    pub text: &'a str,
    pub height: u16,
    pub color: Rgb,
    pub align: Align,
}

/// A GDI call of the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GdiCall {
    CreateCompatibleDc,
    CreateDibSection,
    SelectObject,
    SetBkMode,
    GetStockObject,
    SetTextColor,
    DrawText,
    GdiFlush,
}

impl GdiCall {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CreateCompatibleDc => "CreateCompatibleDC",
            Self::CreateDibSection => "CreateDIBSection",
            Self::SelectObject => "SelectObject",
            Self::SetBkMode => "SetBkMode",
            Self::GetStockObject => "GetStockObject",
            Self::SetTextColor => "SetTextColor",
            Self::DrawText => "DrawTextW",
            Self::GdiFlush => "GdiFlush",
        }
    }
}

/// A canvas that could not be made or drawn into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CanvasError {
    #[error("a canvas of {width} by {height} pixels has an empty side")]
    Empty { width: u16, height: u16 },
    #[error("the canvas's GDI call {} failed", .0.as_str())]
    Gdi(GdiCall),
}

/// Whether the red, green, blue and alpha copy holds the canvas's last drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RgbaCopy {
    Current,
    Stale,
}

/// A canvas of pixels in memory, black when it is made.
pub struct Canvas {
    surface: Surface,
    width: u16,
    rgba: Vec<u8>,
    copy: RgbaCopy,
}

impl Canvas {
    /// Makes a black canvas of `width` by `height` pixels.
    ///
    /// # Errors
    ///
    /// Returns [`CanvasError::Empty`] for a side of 0 pixels, or the GDI call that failed.
    pub fn new(width: u16, height: u16) -> Result<Self, CanvasError> {
        if width == 0 || height == 0 {
            return Err(CanvasError::Empty { width, height });
        }
        let mut surface = Surface::create(width, height)?;
        surface.bits_mut()?.fill(0);
        let rgba = vec![0; byte_len(width, height)];
        Ok(Self { surface, width, rgba, copy: RgbaCopy::Stale })
    }

    /// Fills the part of `rect` that lies on the canvas with `color`.
    ///
    /// # Errors
    ///
    /// Returns [`GdiCall::GdiFlush`] when a GDI call this thread batched before failed.
    pub fn fill(&mut self, rect: Rect, color: Rgb) -> Result<(), CanvasError> {
        fill_bgrx(self.surface.bits_mut()?, self.width, rect, color);
        self.copy = RgbaCopy::Stale;
        Ok(())
    }

    /// Draws `text` as one line in `rect`, vertically centered. No pixel outside `rect` changes, and
    /// a line longer than `rect` ends with an ellipsis. An empty text or a height of 0 draws
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns the GDI call that failed.
    pub fn draw_text(&mut self, rect: Rect, text: &Line<'_>) -> Result<(), CanvasError> {
        if text.text.is_empty() || text.height == 0 || rect.width == 0 || rect.height == 0 {
            return Ok(());
        }
        self.copy = RgbaCopy::Stale;
        self.surface.draw_text(rect, text)
    }

    /// The canvas's pixels as red, green, blue and alpha bytes, rows top first.
    ///
    /// # Errors
    ///
    /// Returns [`GdiCall::GdiFlush`] when a GDI call this thread batched before failed.
    pub fn pixels(&mut self) -> Result<&[u8], CanvasError> {
        if self.copy == RgbaCopy::Stale {
            bgrx_to_rgba(self.surface.bits_mut()?, &mut self.rgba);
            self.copy = RgbaCopy::Current;
        }
        Ok(&self.rgba)
    }
}

/// The bytes of a `width` by `height` canvas. A `u16` side keeps the product far inside `usize`
/// on a 64-bit target.
pub(crate) fn byte_len(width: u16, height: u16) -> usize {
    usize::from(width).saturating_mul(usize::from(height)).saturating_mul(PIXEL_BYTES)
}

/// Fills the part of `rect` inside `bgrx`, the DIB pixels of a canvas `width` pixels wide.
fn fill_bgrx(bgrx: &mut [u8], width: u16, rect: Rect, color: Rgb) {
    let row_bytes = usize::from(width).saturating_mul(PIXEL_BYTES);
    if row_bytes == 0 {
        return;
    }
    let left = usize::from(rect.x.min(width));
    let right = usize::from(rect.x.saturating_add(rect.width).min(width));
    let span = left.saturating_mul(PIXEL_BYTES)..right.saturating_mul(PIXEL_BYTES);
    let rows = bgrx.chunks_exact_mut(row_bytes).skip(usize::from(rect.y));
    for row in rows.take(usize::from(rect.height)) {
        let Some(pixels) = row.get_mut(span.clone()) else {
            continue;
        };
        pixels.as_chunks_mut::<PIXEL_BYTES>().0.fill([color.blue, color.green, color.red, u8::MAX]);
    }
}

/// Copies `bgrx`, DIB pixels, into `rgba` as red, green, blue and an alpha of 255.
fn bgrx_to_rgba(bgrx: &[u8], rgba: &mut [u8]) {
    let pairs = bgrx.as_chunks::<PIXEL_BYTES>().0.iter();
    for (&[blue, green, red, _], target) in pairs.zip(rgba.as_chunks_mut::<PIXEL_BYTES>().0) {
        *target = [red, green, blue, u8::MAX];
    }
}

#[cfg(test)]
mod tests {
    use super::{Rect, Rgb, bgrx_to_rgba, fill_bgrx};

    const GREEN: Rgb = Rgb { red: 10, green: 200, blue: 30 };

    #[test]
    fn a_fill_writes_its_color_as_blue_green_red_inside_the_rect_only() {
        let mut bgrx = vec![0_u8; 4 * 3 * 4];
        let rect = Rect { x: 1, y: 1, width: 2, height: 1 };

        fill_bgrx(&mut bgrx, 4, rect, GREEN);

        let filled: Vec<usize> = (0..12).filter(|pixel| bgrx[pixel * 4] != 0).collect();
        assert_eq!(filled, [5, 6]);
        assert_eq!(&bgrx[20..24], &[30, 200, 10, 255]);
    }

    #[test]
    fn a_fill_past_the_right_and_bottom_edges_is_clipped_to_the_canvas() {
        let mut bgrx = vec![0_u8; 3 * 2 * 4];
        let rect = Rect { x: 2, y: 1, width: 10, height: 10 };

        fill_bgrx(&mut bgrx, 3, rect, GREEN);

        let filled: Vec<usize> = (0..6).filter(|pixel| bgrx[pixel * 4] != 0).collect();
        assert_eq!(filled, [5]);
    }

    #[test]
    fn a_fill_wholly_outside_the_canvas_changes_nothing() {
        let mut bgrx = vec![0_u8; 3 * 2 * 4];

        fill_bgrx(&mut bgrx, 3, Rect { x: 3, y: 0, width: 2, height: 2 }, GREEN);
        fill_bgrx(&mut bgrx, 3, Rect { x: 0, y: 2, width: 2, height: 2 }, GREEN);

        assert!(bgrx.iter().all(|&byte| byte == 0));
    }

    #[test]
    fn a_conversion_swaps_blue_and_red_and_sets_every_alpha_to_255() {
        let bgrx = [1, 2, 3, 0, 4, 5, 6, 77];
        let mut rgba = [0_u8; 8];

        bgrx_to_rgba(&bgrx, &mut rgba);

        assert_eq!(rgba, [3, 2, 1, 255, 6, 5, 4, 255]);
    }
}
