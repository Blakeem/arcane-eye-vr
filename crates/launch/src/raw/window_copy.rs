//! The window copy calls: the search for a window by its title and its process's program file, the
//! list of top-level windows the launcher window's search chooses from, whether a window shows, the
//! topmost order and the copy of the window's client area from the screen through GDI.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};

use tracing::warn;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SRCCOPY, SelectObject,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HWND_BOTTOM, HWND_TOPMOST,
    IsIconic, IsWindowVisible, SET_WINDOW_POS_FLAGS, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
};
use windows::core::PWSTR;

use super::cursor::{AreaFailure, client_area};
use super::top_level_windows;
use crate::cursor::ClientArea;
use crate::launcher_window::{Candidate, Showing};
use crate::window_copy::{
    BITMAP_PIXEL_BYTES, Delivery, WindowCopyError, WindowPicture, bitmap_to_rgb,
};

/// The longest program path `QueryFullProcessImageNameW` returns, in UTF-16 units.
const LONGEST_PATH: usize = 32_768;

/// The size of `BITMAPINFOHEADER` as its `biSize` field holds it.
const HEADER_SIZE: u32 = 40;
const _: () = assert!(HEADER_SIZE as usize == size_of::<BITMAPINFOHEADER>());

/// The bits of one pixel of the copied bitmap.
const PIXEL_BITS: u16 = 32;
const _: () = assert!(PIXEL_BITS as usize == BITMAP_PIXEL_BYTES * 8);

/// The first visible top-level window titled `title` whose process's program file `is_program`
/// accepts.
pub(crate) fn find(title: &str, is_program: impl Fn(&Path) -> bool) -> Option<HWND> {
    top_level_windows(None).into_iter().find(|&window| {
        // SAFETY: IsWindowVisible has no pointer arguments, and a window gone since the search is
        // not visible.
        let visible = unsafe { IsWindowVisible(window) }.as_bool();
        visible
            && window_title(window) == title
            && program_of(window).is_some_and(|program| is_program(&program))
    })
}

/// Every top-level window as the launcher window's search reads it. A window gone since the list
/// was taken is left out. The program file is read only for a visible window that is not
/// minimized, since the search chooses no other window.
pub(crate) fn candidates() -> Vec<Candidate<HWND>> {
    top_level_windows(None).into_iter().filter_map(candidate).collect()
}

/// `window` as the launcher window's search reads it, or `None` for a window gone since the list
/// was taken, whose client area cannot be read.
fn candidate(window: HWND) -> Option<Candidate<HWND>> {
    let Ok(area) = client_area(window) else {
        return None;
    };
    let showing = showing(window);
    let program = if showing == Showing::Visible { program_of(window) } else { None };
    Some(Candidate { window, showing, program, area: (area.width, area.height) })
}

/// How `window` shows on the desktop. A window gone since the caller read it is hidden.
pub(crate) fn showing(window: HWND) -> Showing {
    // SAFETY: IsWindowVisible has no pointer arguments.
    let visible = unsafe { IsWindowVisible(window) }.as_bool();
    // SAFETY: IsIconic has no pointer arguments.
    let minimized = unsafe { IsIconic(window) }.as_bool();
    match (visible, minimized) {
        (false, _) => Showing::Hidden,
        (true, true) => Showing::Minimized,
        (true, false) => Showing::Visible,
    }
}

/// The title of `window`, empty for a window gone since the search.
pub(crate) fn window_title(window: HWND) -> String {
    // SAFETY: GetWindowTextLengthW has no pointer arguments.
    let length = unsafe { GetWindowTextLengthW(window) };
    let mut text = vec![0_u16; usize::try_from(length).unwrap_or(0).saturating_add(1)];
    // SAFETY: the call writes at most `text.len()` units, the length of the slice it gets.
    let copied = unsafe { GetWindowTextW(window, &mut text) };
    let copied = text.get(..usize::try_from(copied).unwrap_or(0)).unwrap_or_default();
    String::from_utf16_lossy(copied)
}

/// The program file of the process that owns `window`. `None` when the process cannot be opened,
/// such as a process of another user, since such a window is not the one the caller looks for.
pub(crate) fn program_of(window: HWND) -> Option<PathBuf> {
    let mut pid = 0_u32;
    // SAFETY: `window` is a window handle, and `pid` is a writable local. A window gone since the
    // search leaves `pid` at 0.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut pid)) };
    // SAFETY: OpenProcess has no pointer arguments.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
    else {
        return None;
    };
    // SAFETY: `process` is the handle the open call returned, and only this owner closes it.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    let mut path = vec![0_u16; LONGEST_PATH];
    let mut length = u32::try_from(path.len()).unwrap_or(u32::MAX);
    let name = PWSTR(path.as_mut_ptr());
    let handle = HANDLE(process.as_raw_handle());
    // SAFETY: `name` holds `length` units, and the call writes at most that many.
    let read =
        unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, name, &raw mut length) };
    let units = path.get(..usize::try_from(length).ok()?)?;
    read.is_ok().then(|| PathBuf::from(OsString::from_wide(units)))
}

/// Puts `window`, titled `title`, above every window that is not topmost, without activating it.
pub(crate) fn make_topmost(
    window: HWND,
    title: &str,
    delivery: Delivery,
) -> Result<(), WindowCopyError> {
    match set_order(window, HWND_TOPMOST, delivery) {
        Ok(()) => Ok(()),
        Err(source) => Err(WindowCopyError::Topmost { title: title.to_owned(), source }),
    }
}

/// Ends the topmost order of `window`, titled `title`, and places it below every other window,
/// without activating it. The order after every window that is not topmost, as `HWND_NOTOPMOST`
/// gives it, would leave the window above a game window that was activated while it was topmost.
pub(crate) fn end_topmost(
    window: HWND,
    title: &str,
    delivery: Delivery,
) -> Result<(), WindowCopyError> {
    match set_order(window, HWND_BOTTOM, delivery) {
        Ok(()) => Ok(()),
        Err(source) => Err(WindowCopyError::EndTopmost { title: title.to_owned(), source }),
    }
}

/// Places `window` in the order `after` names, without moving, sizing or activating it.
fn set_order(window: HWND, after: HWND, delivery: Delivery) -> windows::core::Result<()> {
    let post = match delivery {
        Delivery::Wait => SET_WINDOW_POS_FLAGS(0),
        Delivery::Post => SWP_ASYNCWINDOWPOS,
    };
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | post;
    // SAFETY: SetWindowPos has no pointer arguments, and a window gone since the search fails it.
    unsafe { SetWindowPos(window, Some(after), 0, 0, 0, 0, flags) }
}

/// The pixels of the screen in the client area of `window`, titled `title`. The client area leaves
/// out a frame and its invisible resize border, whose pixels on the screen belong to other windows.
pub(crate) fn copy_from_screen(
    window: HWND,
    title: &str,
) -> Result<WindowPicture, WindowCopyError> {
    let failed = |call| WindowCopyError::Screen { title: title.to_owned(), call };
    let ClientArea { left, top, width, height } = match client_area(window) {
        Ok(area) => area,
        Err(AreaFailure::Rect(source)) => {
            return Err(WindowCopyError::Rect { title: title.to_owned(), source });
        }
        Err(AreaFailure::ToScreen) => return Err(failed("ClientToScreen")),
    };
    let (Ok(columns), Ok(rows)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(WindowCopyError::Empty { title: title.to_owned(), width, height });
    };
    if columns == 0 || rows == 0 {
        return Err(WindowCopyError::Empty { title: title.to_owned(), width, height });
    }
    let screen = ScreenDc::get().ok_or_else(|| failed("GetDC"))?;
    let memory = MemoryDc::create(&screen).ok_or_else(|| failed("CreateCompatibleDC"))?;
    let bitmap =
        Bitmap::create(&screen, width, height).ok_or_else(|| failed("CreateCompatibleBitmap"))?;
    {
        let _selected =
            Selection::select(&memory, &bitmap).ok_or_else(|| failed("SelectObject"))?;
        // SAFETY: both device contexts live in this scope, and the bitmap selected into `memory`
        // holds `width` by `height` pixels.
        let blit =
            unsafe { BitBlt(memory.0, 0, 0, width, height, Some(screen.0), left, top, SRCCOPY) };
        if let Err(source) = blit {
            return Err(WindowCopyError::Blit { title: title.to_owned(), source });
        }
    }
    let bits = read_bits(&screen, &bitmap, columns, rows).ok_or_else(|| failed("GetDIBits"))?;
    Ok(WindowPicture { width: columns, height: rows, rgb: bitmap_to_rgb(&bits) })
}

/// The pixels of `bitmap`, `columns` by `rows`, rows top first as blue, green, red and an unused
/// byte. `None` when the read failed or returned fewer rows. The bitmap is selected into no device
/// context, as the read requires.
fn read_bits(screen: &ScreenDc, bitmap: &Bitmap, columns: u32, rows: u32) -> Option<Vec<u8>> {
    let pixels = usize::try_from(u64::from(columns).checked_mul(u64::from(rows))?).ok()?;
    let mut bits = vec![0_u8; pixels.checked_mul(BITMAP_PIXEL_BYTES)?];
    let header = BITMAPINFOHEADER {
        biSize: HEADER_SIZE,
        biWidth: i32::try_from(columns).ok()?,
        // A negative height asks for the rows top first.
        biHeight: i32::try_from(rows).ok()?.checked_neg()?,
        biPlanes: 1,
        biBitCount: PIXEL_BITS,
        biCompression: BI_RGB.0,
        ..BITMAPINFOHEADER::default()
    };
    let mut info = BITMAPINFO { bmiHeader: header, ..BITMAPINFO::default() };
    let target = Some(bits.as_mut_ptr().cast());
    // SAFETY: `bits` holds `rows` rows of `columns` 32-bit pixels, the format `info` asks for, and
    // `info` is a writable local.
    let read =
        unsafe { GetDIBits(screen.0, bitmap.0, 0, rows, target, &raw mut info, DIB_RGB_COLORS) };
    (u32::try_from(read).ok() == Some(rows)).then_some(bits)
}

/// The device context of the entire screen. Dropping it releases it.
struct ScreenDc(HDC);

impl ScreenDc {
    fn get() -> Option<Self> {
        // SAFETY: no window means the device context of the entire screen.
        let dc = unsafe { GetDC(None) };
        (!dc.is_invalid()).then_some(Self(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: this thread got the device context from GetDC, and only this drop releases it.
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A memory device context like the screen's. Dropping it deletes it.
struct MemoryDc(HDC);

impl MemoryDc {
    fn create(screen: &ScreenDc) -> Option<Self> {
        // SAFETY: `screen` is a live device context.
        let dc = unsafe { CreateCompatibleDC(Some(screen.0)) };
        (!dc.is_invalid()).then_some(Self(dc))
    }
}

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: CreateCompatibleDC made the device context, and only this drop deletes it.
        if !unsafe { DeleteDC(self.0) }.as_bool() {
            warn!("a memory device context of a window copy was not deleted");
        }
    }
}

/// A bitmap in the screen's format. Dropping it deletes it, after its selection ended.
struct Bitmap(HBITMAP);

impl Bitmap {
    fn create(screen: &ScreenDc, width: i32, height: i32) -> Option<Self> {
        // SAFETY: `screen` is a live device context.
        let bitmap = unsafe { CreateCompatibleBitmap(screen.0, width, height) };
        (!bitmap.is_invalid()).then_some(Self(bitmap))
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: CreateCompatibleBitmap made the bitmap, and no device context holds it here.
        if !unsafe { DeleteObject(self.0.into()) }.as_bool() {
            warn!("a bitmap of a window copy was not deleted");
        }
    }
}

/// A bitmap selected into a memory device context. Dropping it selects the object from before.
struct Selection<'a> {
    memory: &'a MemoryDc,
    before: HGDIOBJ,
}

impl<'a> Selection<'a> {
    fn select(memory: &'a MemoryDc, bitmap: &'a Bitmap) -> Option<Self> {
        // SAFETY: both handles are live, and a bitmap can be selected into a memory context.
        let before = unsafe { SelectObject(memory.0, bitmap.0.into()) };
        (!before.is_invalid()).then_some(Self { memory, before })
    }
}

impl Drop for Selection<'_> {
    fn drop(&mut self) {
        // SAFETY: `before` is the object the selection replaced in the live memory context.
        unsafe { SelectObject(self.memory.0, self.before) };
    }
}
