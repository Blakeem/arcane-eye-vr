//! The Win32 calls of the drawn cursor: the thread that owns the game window, the cursor
//! display counter, the pointer in the window's client area, the calling thread's cursor, and the
//! rows of a cursor's bitmaps.

use std::ptr;

use diag::rate::RateLimit;
use diag::warn_every;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, RGBQUAD, ReleaseDC, ScreenToClient,
};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursor, GetCursorPos, GetIconInfo, GetWindowThreadProcessId, HCURSOR, HICON,
    ICONINFO, IsWindowVisible, ShowCursor,
};
use windows::core::HRESULT;

use crate::ui_layer::LOG_PERIOD;
use crate::ui_layer::cursor::counter::Call;
use crate::ui_layer::cursor::image::{COLOR_BITS, MASK_BITS, Shape, Sizes, Source, dib_stride};
use crate::ui_layer::cursor::place::Pointer;

/// The game window: the output window of the swapchain the engine presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Window(HWND);

impl Window {
    pub(super) const fn new(window: HWND) -> Self {
        Self(window)
    }
}

/// A cursor handle. It stays the same while the thread's cursor does not change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CursorHandle(usize);

impl CursorHandle {
    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

impl From<HCURSOR> for CursorHandle {
    fn from(cursor: HCURSOR) -> Self {
        Self(cursor.0.expose_provenance())
    }
}

/// Which pointer read failed, and the code its call returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PointerFault {
    #[error("GetCursorPos failed with {0}")]
    Position(HRESULT),
    #[error("ScreenToClient failed")]
    ToClient,
    #[error("GetClientRect failed with {0}")]
    Client(HRESULT),
}

/// Which read of a cursor's bitmaps failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReadFault {
    #[error("GetIconInfo failed with {0}")]
    IconInfo(HRESULT),
    #[error("GetIconInfo gave no mask bitmap")]
    NoMask,
    #[error("GetObjectW gave no bitmap size")]
    BitmapSize,
    #[error("GetDC gave no screen device context")]
    ScreenDc,
    #[error("the rows of a {width} pixel wide bitmap exceed a buffer's size")]
    RowSize { width: u16 },
    #[error("GetDIBits copied {copied} of {rows} bitmap rows")]
    Rows { copied: i32, rows: u32 },
}

pub(crate) fn current_thread() -> u32 {
    // SAFETY: the call takes no argument.
    unsafe { GetCurrentThreadId() }
}

/// The thread that owns `window`, or `None` when the handle names no window.
pub(crate) fn window_thread(window: Window) -> Option<u32> {
    // SAFETY: the call takes any handle and returns 0 for one that names no window.
    let thread = unsafe { GetWindowThreadProcessId(window.0, None) };
    (thread != 0).then_some(thread)
}

/// Makes `call` on the calling thread's cursor display counter. The new count is not returned,
/// since the core tracks only its own calls.
pub(crate) fn show(call: Call) {
    let show = call == Call::Show;
    // SAFETY: the call changes only the calling thread's counter.
    unsafe { ShowCursor(show) };
}

/// The pointer in `window`'s client area, or `None` while the window is hidden, since a hidden
/// window shows no pointer.
pub(crate) fn pointer(window: Window) -> Result<Option<Pointer>, PointerFault> {
    // SAFETY: the call takes any handle and returns false for one that names no window.
    if !unsafe { IsWindowVisible(window.0) }.as_bool() {
        return Ok(None);
    }
    let (mut point, mut client) = (POINT::default(), RECT::default());
    // SAFETY: `point` is writable for the call.
    if let Err(error) = unsafe { GetCursorPos(&raw mut point) } {
        return Err(PointerFault::Position(error.code()));
    }
    // SAFETY: `point` is writable for the call, and the call fails for a handle that names no window.
    if !unsafe { ScreenToClient(window.0, &raw mut point) }.as_bool() {
        return Err(PointerFault::ToClient);
    }
    // SAFETY: `client` is writable for the call, and the call fails for a handle that names no
    // window.
    if let Err(error) = unsafe { GetClientRect(window.0, &raw mut client) } {
        return Err(PointerFault::Client(error.code()));
    }
    // The client rectangle starts at 0, 0, so its right and bottom edges are its size.
    Ok(Some(Pointer { x: point.x, y: point.y, width: client.right, height: client.bottom }))
}

/// The calling thread's cursor, or `None` when the thread shows none.
pub(crate) fn current_cursor() -> Option<CursorHandle> {
    // SAFETY: the call takes no argument and reads the calling thread's cursor.
    let cursor = unsafe { GetCursor() };
    (!cursor.0.is_null()).then(|| CursorHandle::from(cursor))
}

/// The rate limits of the warnings that a failed release of a cursor read's GDI objects logs. A
/// read runs on every handle change, which follows the pointer.
#[derive(Debug, Default)]
pub(crate) struct ReleaseLogs {
    bitmap: RateLimit,
    screen_dc: RateLimit,
}

/// The bitmaps `GetIconInfo` made of a cursor, and its hotspot. The bitmaps are deleted on drop.
pub(crate) struct CursorBitmaps<'log> {
    mask: HBITMAP,
    color: Option<HBITMAP>,
    hotspot: [u32; 2],
    logs: &'log mut ReleaseLogs,
}

impl<'log> CursorBitmaps<'log> {
    /// The bitmaps of `cursor`. A failed release of a GDI object logs through `logs`.
    pub(crate) fn of(cursor: CursorHandle, logs: &'log mut ReleaseLogs) -> Result<Self, ReadFault> {
        let icon = HICON(ptr::with_exposed_provenance_mut(cursor.0));
        let mut info = ICONINFO::default();
        // SAFETY: `info` is writable for the call, which fails for a handle that names no icon or
        // cursor.
        if let Err(error) = unsafe { GetIconInfo(icon, &raw mut info) } {
            return Err(ReadFault::IconInfo(error.code()));
        }
        let color = (!info.hbmColor.0.is_null()).then_some(info.hbmColor);
        if info.hbmMask.0.is_null() {
            delete(color, &mut logs.bitmap);
            return Err(ReadFault::NoMask);
        }
        let hotspot = [info.xHotspot, info.yHotspot];
        Ok(Self { mask: info.hbmMask, color, hotspot, logs })
    }

    /// The hotspot, x then y.
    pub(crate) const fn hotspot(&self) -> [u32; 2] {
        self.hotspot
    }

    pub(crate) fn sizes(&self) -> Result<Sizes, ReadFault> {
        let color = match self.color {
            Some(color) => Some(bitmap_size(color)?),
            None => None,
        };
        Ok(Sizes { mask: bitmap_size(self.mask)?, color })
    }

    /// The rows of the mask bitmap of a cursor of `shape`, 1 bit per pixel.
    pub(crate) fn mask_rows(&mut self, shape: Shape) -> Result<Vec<u8>, ReadFault> {
        let log = &mut self.logs.screen_dc;
        dib_rows(self.mask, shape.width, shape.mask_rows(), MASK_BITS, log)
    }

    /// The rows of the color bitmap of a cursor of `shape`, 32 bits per pixel, or `None` for a
    /// cursor with no color bitmap.
    pub(crate) fn color_rows(&mut self, shape: Shape) -> Result<Option<Vec<u8>>, ReadFault> {
        let log = &mut self.logs.screen_dc;
        match (shape.source, self.color) {
            (Source::Color, Some(color)) => {
                Ok(Some(dib_rows(color, shape.width, u32::from(shape.height), COLOR_BITS, log)?))
            }
            (Source::Color | Source::Mask, _) => Ok(None),
        }
    }
}

impl Drop for CursorBitmaps<'_> {
    fn drop(&mut self) {
        delete(Some(self.mask), &mut self.logs.bitmap);
        delete(self.color, &mut self.logs.bitmap);
    }
}

/// Deletes `bitmap`, a bitmap `GetIconInfo` made. A failed delete leaks one bitmap and logs it
/// through `log`.
fn delete(bitmap: Option<HBITMAP>, log: &mut RateLimit) {
    let Some(bitmap) = bitmap else { return };
    // SAFETY: the bitmap is one `GetIconInfo` made for this process, deleted once.
    let deleted = unsafe { DeleteObject(bitmap.into()) };
    if !deleted.as_bool() {
        warn_every!(*log, LOG_PERIOD, "deleting a cursor bitmap failed, so one bitmap leaks");
    }
}

/// The width and height of `bitmap`.
fn bitmap_size(bitmap: HBITMAP) -> Result<[u32; 2], ReadFault> {
    let mut info = BITMAP::default();
    let Ok(len) = i32::try_from(size_of::<BITMAP>()) else {
        return Err(ReadFault::BitmapSize);
    };
    // SAFETY: `info` is a writable `BITMAP` of `len` bytes for the call.
    let written = unsafe { GetObjectW(bitmap.into(), len, Some((&raw mut info).cast())) };
    if written == 0 {
        return Err(ReadFault::BitmapSize);
    }
    let (Ok(width), Ok(height)) = (u32::try_from(info.bmWidth), u32::try_from(info.bmHeight))
    else {
        return Err(ReadFault::BitmapSize);
    };
    Ok([width, height])
}

/// The header of a DIB and room for the two color entries of a 1-bit DIB, which `GetDIBits` fills.
#[repr(C)]
struct DibInfo {
    header: BITMAPINFOHEADER,
    colors: [RGBQUAD; 2],
}

/// `rows` top-down rows of `bitmap`, `width` pixels of `bits` bits each. A failed release of the
/// screen's device context logs through `log`.
fn dib_rows(
    bitmap: HBITMAP,
    width: u16,
    rows: u32,
    bits: u16,
    log: &mut RateLimit,
) -> Result<Vec<u8>, ReadFault> {
    let row_size = ReadFault::RowSize { width };
    let stride = dib_stride(usize::from(width), bits).ok_or(row_size)?;
    let len = usize::try_from(rows).ok().and_then(|rows| rows.checked_mul(stride));
    let top_down = i32::try_from(rows).ok().and_then(i32::checked_neg).ok_or(row_size)?;
    let header_size = u32::try_from(size_of::<BITMAPINFOHEADER>()).ok().ok_or(row_size)?;
    let mut bytes = vec![0_u8; len.ok_or(row_size)?];
    let mut info = DibInfo {
        header: BITMAPINFOHEADER {
            biSize: header_size,
            biWidth: i32::from(width),
            biHeight: top_down,
            biPlanes: 1,
            biBitCount: bits,
            biCompression: BI_RGB.0,
            ..BITMAPINFOHEADER::default()
        },
        colors: [RGBQUAD::default(); 2],
    };
    let screen = ScreenDc::get(log)?;
    // SAFETY: `bytes` holds `rows` rows of the stride `GetDIBits` writes for `bits` bits per pixel,
    // and `info` holds the header and the two color entries a 1-bit DIB adds. The bitmap is one
    // `GetIconInfo` made, which no device context selects.
    let copied = unsafe {
        GetDIBits(
            screen.0,
            bitmap,
            0,
            rows,
            Some(bytes.as_mut_ptr().cast()),
            (&raw mut info).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
        )
    };
    if u32::try_from(copied).ok() != Some(rows) {
        return Err(ReadFault::Rows { copied, rows });
    }
    Ok(bytes)
}

/// The screen's device context, released on drop. A failed release logs through the rate limit.
struct ScreenDc<'log>(HDC, &'log mut RateLimit);

impl<'log> ScreenDc<'log> {
    fn get(log: &'log mut RateLimit) -> Result<Self, ReadFault> {
        // SAFETY: no window selects the screen's device context.
        let dc = unsafe { GetDC(None) };
        if dc.0.is_null() {
            return Err(ReadFault::ScreenDc);
        }
        Ok(Self(dc, log))
    }
}

impl Drop for ScreenDc<'_> {
    fn drop(&mut self) {
        // SAFETY: the context is the screen's, which `get` took and only this drop releases.
        let released = unsafe { ReleaseDC(None, self.0) };
        if released == 0 {
            warn_every!(
                *self.1,
                LOG_PERIOD,
                "releasing the screen device context failed, so one common context stays in use"
            );
        }
    }
}
