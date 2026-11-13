//! The viewer's unsafe edge: the window and the viewer header view here, and in `raw/gpu.rs` the
//! D3D11 and DXGI calls that open the shared texture and draw it.

#![expect(
    unsafe_code,
    reason = "the window, the header mapping and the D3D11 and DXGI calls are Windows calls"
)]

mod gpu;

use std::ptr;
use std::time::Duration;

use ipc_layout::viewer_header::{HEARTBEAT_OFFSET, SIZE, WORDS};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, IDC_ARROW,
    LoadCursorW, MB_ICONERROR, MB_OK, MSG, MessageBoxW, MsgWaitForMultipleObjects, PM_REMOVE,
    PeekMessageW, PostQuitMessage, QS_ALLINPUT, RegisterClassExW, SetWindowTextW, WINDOW_EX_STYLE,
    WM_DESTROY, WM_QUIT, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{HSTRING, w};

pub(crate) use gpu::{Copied, GpuError, Renderer, Screen, Source};

/// The window's first outer size, frame and title bar included, before the user resizes it.
const INITIAL_SIZE: (i32, i32) = (1280, 720);

/// The viewer's top-level window. It closes when the process exits.
pub(crate) struct Window {
    handle: HWND,
}

impl Window {
    pub(crate) fn create(title: &str) -> windows::core::Result<Self> {
        // SAFETY: `None` names the viewer's own exe, which stays loaded.
        let instance = unsafe { GetModuleHandleW(None) }?;
        // SAFETY: IDC_ARROW is a system cursor, so no module is passed.
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }?;
        // SAFETY: BLACK_BRUSH is a stock object, which is never freed.
        let background = HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0);
        let class_name = w!("arcane-eye-vr-viewer");
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or(u32::MAX),
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: cursor,
            hbrBackground: background,
            lpszClassName: class_name,
            ..Default::default()
        };
        // SAFETY: `class` is live for the call, and its name is a static wide string.
        if unsafe { RegisterClassExW(&raw const class) } == 0 {
            return Err(windows::core::Error::from_thread());
        }
        let title = HSTRING::from(title);
        // SAFETY: the class was registered above, and the window has no parent, menu or create data.
        let handle = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                &title,
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                INITIAL_SIZE.0,
                INITIAL_SIZE.1,
                None,
                None,
                Some(instance.into()),
                None,
            )
        }?;
        Ok(Self { handle })
    }

    /// The client area's `(width, height)`, or `(0, 0)` when it cannot be read.
    pub(crate) fn client_size(&self) -> (u32, u32) {
        let mut rect = RECT::default();
        // SAFETY: `rect` is writable, and the window lives until the process exits.
        if unsafe { GetClientRect(self.handle, &raw mut rect) }.is_err() {
            return (0, 0);
        }
        // A client rectangle starts at (0, 0), so its right and bottom are its size.
        (u32::try_from(rect.right).unwrap_or(0), u32::try_from(rect.bottom).unwrap_or(0))
    }

    pub(crate) fn set_title(&self, title: &str) {
        let title = HSTRING::from(title);
        // A title that fails to change keeps the old status text, and the image is unaffected.
        // SAFETY: `title` is a NUL-terminated wide string that outlives the call.
        drop(unsafe { SetWindowTextW(self.handle, &title) });
    }
}

/// Dispatches every message queued for this thread. Returns `false` once a window closed, which
/// posts the quit message.
pub(crate) fn pump_messages() -> bool {
    let mut message = MSG::default();
    // SAFETY: `message` is writable. `None` also reads the thread's quit message, which no window
    // owns.
    while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        if message.message == WM_QUIT {
            return false;
        }
        // SAFETY: `message` is the message `PeekMessageW` just removed.
        unsafe { DispatchMessageW(&raw const message) };
    }
    true
}

/// Waits until a message arrives for this thread or `timeout` passes.
pub(crate) fn wait_for_messages(timeout: Duration) {
    let millis = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: no handles are passed, so only this thread's message queue ends the wait early.
    unsafe { MsgWaitForMultipleObjects(None, false, millis, QS_ALLINPUT) };
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        // SAFETY: posts WM_QUIT to this thread's queue, which ends the message loop.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: the arguments are the ones Windows passed to this window procedure.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Shows `text` under `caption` in an error message box and waits until it is closed.
pub(crate) fn show_error(caption: &str, text: &str) {
    let (caption, text) = (HSTRING::from(caption), HSTRING::from(text));
    // SAFETY: both strings are NUL-terminated wide strings that outlive the call, and the box has
    // no owner window.
    unsafe { MessageBoxW(None, &text, &caption, MB_OK | MB_ICONERROR) };
}

/// A view of the viewer header that the core created. The view is unmapped on drop.
pub(crate) struct HeaderView {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

/// Why the viewer header could not be opened or mapped, for any cause but a missing mapping.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HeaderViewError {
    #[error("opening the viewer header's mapping failed: {0}")]
    Open(#[source] windows::core::Error),
    #[error("mapping a view of the viewer header failed: {0}")]
    Map(#[source] windows::core::Error),
}

impl HeaderView {
    /// Maps the header in the mapping `name`, or returns `None` when no core created it.
    ///
    /// # Errors
    ///
    /// Returns [`HeaderViewError`] when opening fails for another cause, such as a viewer without
    /// access to an elevated game's mapping, or when mapping the view fails.
    pub(crate) fn open(name: &str) -> Result<Option<Self>, HeaderViewError> {
        let name = HSTRING::from(name);
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let mapping = match unsafe { OpenFileMappingW(access.0, false, &name) } {
            Ok(mapping) => mapping,
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => return Ok(None),
            Err(error) => return Err(HeaderViewError::Open(error)),
        };
        // SAFETY: `mapping` is the open mapping handle above, and the view spans the header's bytes.
        let address = unsafe { MapViewOfFile(mapping, access, 0, 0, SIZE) };
        // The error is read before the close, which can overwrite the thread's last error.
        let map_error = address.Value.is_null().then(windows::core::Error::from_thread);
        // The view keeps the mapping alive, so the handle closes at once. A failed close leaks one
        // handle and changes nothing else.
        // SAFETY: `mapping` is a handle this function opened and closes once.
        drop(unsafe { CloseHandle(mapping) });
        match map_error {
            Some(error) => Err(HeaderViewError::Map(error)),
            None => Ok(Some(Self { address })),
        }
    }

    /// Copies the header's words. Each read is volatile, since the core writes the header.
    pub(crate) fn read_words(&self) -> [u32; WORDS] {
        let base = self.address.Value.cast::<u32>().cast_const();
        let mut words = [0; WORDS];
        // The last word is read first, so the generation is read before the texture fields.
        for (index, word) in words.iter_mut().enumerate().rev() {
            // SAFETY: `index` is below `WORDS`, and the page-aligned view spans the header's bytes.
            *word = unsafe { ptr::read_volatile(base.wrapping_add(index)) };
        }
        words
    }

    /// Writes the heartbeat, the one field the viewer owns.
    pub(crate) fn write_heartbeat(&self, heartbeat: u32) {
        let field = self.address.Value.wrapping_byte_add(HEARTBEAT_OFFSET).cast::<u32>();
        // SAFETY: the heartbeat is a 4-aligned word inside the page-aligned view of the header.
        unsafe { ptr::write_volatile(field, heartbeat) };
    }
}

impl Drop for HeaderView {
    fn drop(&mut self) {
        // A failed unmap leaves this one view mapped until the viewer exits.
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(self.address) });
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
