//! The GDI calls of the pixel canvas: its memory device context with a top-down 32-bit DIB section
//! selected, the font of a line of text and the text draw.
//!
//! Every object the canvas creates has an owner whose drop deletes it, so each path deletes it.

use std::slice;

use tracing::warn;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CLR_INVALID,
    CreateCompatibleDC, CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FW_NORMAL, GdiFlush,
    GetStockObject, HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::core::w;

use crate::canvas::{Align, CanvasError, GdiCall, Line, PIXEL_BYTES, Rgb, byte_len};
use crate::layer::Rect;

/// The size of `BITMAPINFOHEADER` as its `biSize` field holds it.
const HEADER_SIZE: u32 = 40;
const _: () = assert!(HEADER_SIZE as usize == size_of::<BITMAPINFOHEADER>());

/// The bits of one pixel of the DIB section.
const PIXEL_BITS: u16 = 32;
const _: () = assert!(PIXEL_BITS as usize == PIXEL_BYTES * 8);

/// The weight of regular text as `CreateFontW` takes it.
const REGULAR_WEIGHT: i32 = FW_NORMAL.0.cast_signed();

/// A memory device context with the canvas's DIB section selected and a transparent text
/// background. Its drop selects the context's first bitmap back, then the fields delete the
/// context and the DIB section in that order.
pub(crate) struct Surface {
    dc: MemoryDc,
    dib: Dib,
    before: HGDIOBJ,
}

impl Surface {
    /// Creates the memory device context and a `width` by `height` DIB section, rows top first.
    pub(crate) fn create(width: u16, height: u16) -> Result<Self, CanvasError> {
        let dc = MemoryDc::create()?;
        let dib = Dib::create(width, height)?;
        // SAFETY: both handles are live, and a DIB section can be selected into a memory context.
        let before = unsafe { SelectObject(dc.0, dib.bitmap.into()) };
        if before.is_invalid() {
            return Err(CanvasError::Gdi(GdiCall::SelectObject));
        }
        let surface = Self { dc, dib, before };
        // SAFETY: the device context is live, and TRANSPARENT is a valid mode.
        if unsafe { SetBkMode(surface.dc.0, TRANSPARENT) } == 0 {
            return Err(CanvasError::Gdi(GdiCall::SetBkMode));
        }
        Ok(surface)
    }

    /// The DIB section's pixels, blue, green, red and an unused byte, rows top first, after every
    /// GDI call of this thread has finished writing them.
    pub(crate) fn bits_mut(&mut self) -> Result<&mut [u8], CanvasError> {
        // SAFETY: GdiFlush has no arguments.
        if !unsafe { GdiFlush() }.as_bool() {
            return Err(CanvasError::Gdi(GdiCall::GdiFlush));
        }
        // SAFETY: CreateDIBSection allocated `len` bytes at `bits`, which live until the DIB section
        // is deleted, after `self`. The `&mut self` borrow keeps every other access out, and GDI
        // writes them only during this surface's own calls.
        Ok(unsafe { slice::from_raw_parts_mut(self.dib.bits, self.dib.len) })
    }

    /// Draws `text` in `rect` with GDI. The text is clipped to `rect`.
    pub(crate) fn draw_text(&mut self, rect: Rect, text: &Line<'_>) -> Result<(), CanvasError> {
        let font = Font::create(text.height)?;
        self.draw_text_in(rect, text, &font)
    }

    /// Draws `text` in `rect` with `font`, whose height wins over the text's.
    pub(crate) fn draw_text_in(
        &mut self,
        rect: Rect,
        text: &Line<'_>,
        font: &Font,
    ) -> Result<(), CanvasError> {
        let mut units: Vec<u16> = text.text.encode_utf16().collect();
        let mut bounds = gdi_rect(rect);
        let _selected = FontSelection::select(&self.dc, font)?;
        // SAFETY: the device context is live, and a color is a plain value.
        if unsafe { SetTextColor(self.dc.0, colorref(text.color)) } == COLORREF(CLR_INVALID) {
            return Err(CanvasError::Gdi(GdiCall::SetTextColor));
        }
        // SAFETY: the device context and its font are live, `units` holds the text with its length,
        // and `bounds` is a writable local. The format leaves out DT_MODIFYSTRING, so the call does
        // not write `units`.
        let drawn =
            unsafe { DrawTextW(self.dc.0, &mut units, &raw mut bounds, format(text.align)) };
        if drawn == 0 {
            return Err(CanvasError::Gdi(GdiCall::DrawText));
        }
        Ok(())
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: `before` is the object the DIB section replaced in the live device context.
        unsafe { SelectObject(self.dc.0, self.before) };
    }
}

/// The `RECT` of `rect`. A `u16` side keeps every edge inside `i32`.
fn gdi_rect(rect: Rect) -> RECT {
    let left = i32::from(rect.x);
    let top = i32::from(rect.y);
    RECT {
        left,
        top,
        right: left.saturating_add(i32::from(rect.width)),
        bottom: top.saturating_add(i32::from(rect.height)),
    }
}

/// The `COLORREF` of `color`: red in the low byte, then green, then blue.
fn colorref(color: Rgb) -> COLORREF {
    COLORREF(u32::from_le_bytes([color.red, color.green, color.blue, 0]))
}

/// One vertically centered line with no prefix processing, an ellipsis at a cut end and `align`.
fn format(align: Align) -> DRAW_TEXT_FORMAT {
    let across = match align {
        Align::Left => DT_LEFT,
        Align::Center => DT_CENTER,
        Align::Right => DT_RIGHT,
    };
    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | across
}

/// A memory device context like the screen's. Dropping it deletes it.
struct MemoryDc(HDC);

impl MemoryDc {
    fn create() -> Result<Self, CanvasError> {
        // SAFETY: no device context means a context like the screen's.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return Err(CanvasError::Gdi(GdiCall::CreateCompatibleDc));
        }
        Ok(Self(dc))
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: CreateCompatibleDC made the device context, and only this drop deletes it.
        if !unsafe { DeleteDC(self.0) }.as_bool() {
            warn!("a memory device context of a pixel canvas was not deleted");
        }
    }
}

/// A DIB section and its pixels' memory. Dropping it deletes both, after its selection ended.
struct Dib {
    bitmap: HBITMAP,
    bits: *mut u8,
    len: usize,
}

impl Dib {
    fn create(width: u16, height: u16) -> Result<Self, CanvasError> {
        let header = BITMAPINFOHEADER {
            biSize: HEADER_SIZE,
            biWidth: i32::from(width),
            // A negative height asks for the rows top first.
            biHeight: i32::from(height).saturating_neg(),
            biPlanes: 1,
            biBitCount: PIXEL_BITS,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        };
        let info = BITMAPINFO { bmiHeader: header, ..BITMAPINFO::default() };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` describes an uncompressed 32-bit bitmap and `bits` is a writable local.
        // DIB_RGB_COLORS with no section makes the call ignore the device context and allocate.
        let created = unsafe {
            CreateDIBSection(None, &raw const info, DIB_RGB_COLORS, &raw mut bits, None, 0)
        };
        let Ok(bitmap) = created else {
            return Err(CanvasError::Gdi(GdiCall::CreateDibSection));
        };
        let dib = Self { bitmap, bits: bits.cast(), len: byte_len(width, height) };
        if dib.bits.is_null() {
            return Err(CanvasError::Gdi(GdiCall::CreateDibSection));
        }
        Ok(dib)
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: CreateDIBSection made the bitmap, and no device context holds it here.
        if !unsafe { DeleteObject(self.bitmap.into()) }.as_bool() {
            warn!("a DIB section of a pixel canvas was not deleted");
        }
    }
}

/// The font of a line of text: "Segoe UI" at the text's height, or the system's default GUI font
/// when that font cannot be created. Dropping it deletes a created font.
pub(crate) enum Font {
    Created(HFONT),
    Stock(HGDIOBJ),
}

impl Font {
    fn create(height: u16) -> Result<Self, CanvasError> {
        Self::segoe(height).map_or_else(Self::stock, Ok)
    }

    /// "Segoe UI" with characters `height` pixels high and grayscale antialiasing, since subpixel
    /// text assumes the pixel layout of a flat LCD, which a headset's view of the panel lacks.
    fn segoe(height: u16) -> Option<Self> {
        // SAFETY: every argument is a plain value, and the face name is a NUL-terminated literal. A
        // negative height asks for the character height in place of the cell height.
        let font = unsafe {
            CreateFontW(
                i32::from(height).saturating_neg(),
                0,
                0,
                0,
                REGULAR_WEIGHT,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                0,
                w!("Segoe UI"),
            )
        };
        (!font.is_invalid()).then_some(Self::Created(font))
    }

    /// The system's default GUI font, a stock object that is never deleted.
    pub(crate) fn stock() -> Result<Self, CanvasError> {
        // SAFETY: DEFAULT_GUI_FONT names a stock object.
        let stock = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
        if stock.is_invalid() {
            return Err(CanvasError::Gdi(GdiCall::GetStockObject));
        }
        Ok(Self::Stock(stock))
    }

    fn handle(&self) -> HGDIOBJ {
        match *self {
            Self::Created(font) => font.into(),
            Self::Stock(stock) => stock,
        }
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        let Self::Created(font) = *self else {
            return;
        };
        // SAFETY: CreateFontW made the font, and its selection ended before this drop.
        if !unsafe { DeleteObject(font.into()) }.as_bool() {
            warn!("a font of a pixel canvas was not deleted");
        }
    }
}

/// A font selected into the canvas's device context. Dropping it selects the font from before.
struct FontSelection<'a> {
    dc: &'a MemoryDc,
    before: HGDIOBJ,
}

impl<'a> FontSelection<'a> {
    fn select(dc: &'a MemoryDc, font: &Font) -> Result<Self, CanvasError> {
        // SAFETY: both handles are live, and a font can be selected into a device context.
        let before = unsafe { SelectObject(dc.0, font.handle()) };
        if before.is_invalid() {
            return Err(CanvasError::Gdi(GdiCall::SelectObject));
        }
        Ok(Self { dc, before })
    }
}

impl Drop for FontSelection<'_> {
    fn drop(&mut self) {
        // SAFETY: `before` is the object the font replaced in the live device context.
        unsafe { SelectObject(self.dc.0, self.before) };
    }
}
