//! The cursor calls: the foreground window, its client area on the screen, the cursor move and the
//! thread's DPI awareness.

use tracing::warn;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetForegroundWindow, GetWindowThreadProcessId, SetCursorPos,
};

use crate::cursor::{ClientArea, CursorError};

/// This thread's DPI awareness set to per monitor, so window rectangles and cursor positions are
/// physical pixels. Dropping it restores the awareness the thread had.
pub(crate) struct PhysicalPixels {
    /// The awareness before the start.
    before: DPI_AWARENESS_CONTEXT,
}

impl PhysicalPixels {
    /// Sets the awareness, or returns `None` when the thread kept the one it had.
    pub(crate) fn try_start() -> Option<Self> {
        // SAFETY: the context is one of the predefined awareness values.
        let before =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        // Only null means failure. `is_invalid` also rejects -1, the predefined unaware context.
        (!before.0.is_null()).then_some(Self { before })
    }

    /// [`Self::try_start`] that warns when the thread kept its awareness, for a caller that runs
    /// once and goes on in the awareness it has.
    pub(crate) fn start() -> Option<Self> {
        let started = Self::try_start();
        if started.is_none() {
            warn!("the thread kept its DPI awareness, so a scaled display may shift the cursor");
        }
        started
    }
}

impl Drop for PhysicalPixels {
    fn drop(&mut self) {
        // SAFETY: `before` is the awareness the thread had, which the start call returned.
        unsafe { SetThreadDpiAwarenessContext(self.before) };
    }
}

/// The foreground window, or `None` while no window is in the foreground.
pub(crate) fn foreground_window() -> Option<HWND> {
    // SAFETY: GetForegroundWindow has no arguments.
    let window = unsafe { GetForegroundWindow() };
    (!window.is_invalid()).then_some(window)
}

/// The foreground window when process `pid` owns it.
pub(crate) fn foreground_window_of(pid: u32) -> Option<HWND> {
    let window = foreground_window()?;
    let mut owner = 0u32;
    // SAFETY: `window` is a window handle, and `owner` is a writable local. A window gone since the
    // read leaves `owner` at 0.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut owner)) };
    (owner == pid).then_some(window)
}

/// The call of a client area read that failed. Each caller names the window in its own error.
#[derive(Debug)]
pub(crate) enum AreaFailure {
    /// `GetClientRect` failed.
    Rect(windows::core::Error),
    /// `ClientToScreen` failed. Its documentation names no error code for a failure.
    ToScreen,
}

/// The client area of `window` on the screen.
pub(crate) fn client_area(window: HWND) -> Result<ClientArea, AreaFailure> {
    let mut rect = RECT::default();
    // SAFETY: `rect` is a writable local, and a window gone since the search fails the call.
    if let Err(source) = unsafe { GetClientRect(window, &raw mut rect) } {
        return Err(AreaFailure::Rect(source));
    }
    let mut corner = POINT { x: 0, y: 0 };
    // SAFETY: `corner` is a writable local, and a window gone since the search fails the call.
    if !unsafe { ClientToScreen(window, &raw mut corner) }.as_bool() {
        return Err(AreaFailure::ToScreen);
    }
    Ok(ClientArea { left: corner.x, top: corner.y, width: rect.right, height: rect.bottom })
}

/// Moves the cursor to the screen position `x`, `y`.
pub(crate) fn move_cursor(x: i32, y: i32) -> Result<(), CursorError> {
    // SAFETY: SetCursorPos has no pointer arguments.
    match unsafe { SetCursorPos(x, y) } {
        Ok(()) => Ok(()),
        Err(source) => Err(CursorError::Move { x, y, source }),
    }
}
