//! The window copy against a framed window of this test with a known picture: a left half and a
//! right half of two colors whose three channels all differ, so a mirrored copy or swapped channels
//! show.

use std::env;
use std::io::{self, Write as _};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, OpenInputDesktop,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWL_EXSTYLE, GetClientRect,
    GetWindowLongPtrW, GetWindowRect, MSG, PM_REMOVE, PeekMessageW, RegisterClassW, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOZORDER, SetWindowPos, WM_PAINT, WNDCLASSW, WS_CAPTION, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_THICKFRAME, WS_VISIBLE,
};
use windows::core::{HSTRING, PCWSTR};

use super::cursor::PhysicalPixels;
use crate::window_copy::{self, WindowPicture};

/// The screen position of the test window and the size of its client area.
const LEFT: i32 = 200;
const TOP: i32 = 200;
const WIDTH: i32 = 64;
const HEIGHT: i32 = 48;
const HALF_WIDTH: i32 = WIDTH / 2;

/// The width and height the test window is created with. A frame takes less, so the client area
/// is not empty when `fit_client_area` measures the frame.
const CREATED_SIZE: i32 = 400;

/// The red, green and blue of the window's left half and of its right half.
const LEFT_COLOR: [u8; 3] = [200, 40, 120];
const RIGHT_COLOR: [u8; 3] = [30, 160, 220];

/// The longest wait for the copy to show the painted window, which the desktop composes a frame
/// or more after the paint.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(5);

/// The time between two copies while the picture is not on the screen yet.
const RETRY: Duration = Duration::from_millis(50);

/// A `COLORREF` holds red in its low byte, then green, then blue.
fn colorref([red, green, blue]: [u8; 3]) -> COLORREF {
    COLORREF(u32::from_le_bytes([red, green, blue, 0]))
}

/// Paints the left half of the client area in `LEFT_COLOR` and the right half in `RIGHT_COLOR`.
extern "system" fn paint_halves(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if message != WM_PAINT {
        // SAFETY: the arguments are the ones Windows passed to this window procedure.
        return unsafe { DefWindowProcW(window, message, w, l) };
    }
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: `paint` is a writable local, and Windows sent WM_PAINT to `window`.
    let dc = unsafe { BeginPaint(window, &raw mut paint) };
    let halves = [(0, HALF_WIDTH, LEFT_COLOR), (HALF_WIDTH, WIDTH, RIGHT_COLOR)];
    for (left, right, color) in halves {
        let rect = RECT { left, top: 0, right, bottom: HEIGHT };
        // SAFETY: CreateSolidBrush has no pointer arguments.
        let brush = unsafe { CreateSolidBrush(colorref(color)) };
        // SAFETY: `dc` is the paint context, and `rect` and `brush` live through the call.
        unsafe { FillRect(dc, &raw const rect, brush) };
        // A brush that is not deleted only leaks until the test process exits.
        // SAFETY: this loop made the brush, and no device context holds it.
        let _deleted = unsafe { DeleteObject(brush.into()) };
    }
    // EndPaint always returns nonzero.
    // SAFETY: `paint` is the structure BeginPaint filled in for `window`.
    let _always_nonzero = unsafe { EndPaint(window, &raw const paint) };
    LRESULT(0)
}

/// A visible window of this thread titled with this process's id, so no other test shares it.
/// Dropping it destroys the window.
struct PaintedWindow {
    window: HWND,
    title: String,
}

impl PaintedWindow {
    fn create() -> Self {
        let title = format!("arcane-eye-launch-test-window-copy-{}", process::id());
        // SAFETY: no module name means this exe's own module.
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.expect("this exe's module");
        let instance = HINSTANCE::from(module);
        let class = HSTRING::from(title.as_str());
        let class_info = WNDCLASSW {
            lpfnWndProc: Some(paint_halves),
            hInstance: instance,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..WNDCLASSW::default()
        };
        // SAFETY: `class_info` and the class name it points to outlive the call.
        assert_ne!(unsafe { RegisterClassW(&raw const class_info) }, 0, "register the class");
        // A tool window that takes no activation keeps the focus where the person has it.
        let ex_style = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        // A title bar and a sizing frame, so a copy that holds them fails the picture check.
        let style = WS_CAPTION | WS_THICKFRAME | WS_VISIBLE;
        // SAFETY: the class is registered above, and each pointer argument is a live string or null.
        let window = unsafe {
            CreateWindowExW(
                ex_style,
                &class,
                &class,
                style,
                LEFT,
                TOP,
                CREATED_SIZE,
                CREATED_SIZE,
                None,
                None,
                Some(instance),
                None,
            )
        };
        let window = window.expect("create the test window");
        square_corners(window);
        fit_client_area(window);
        Self { window, title }
    }
}

/// Turns off the rounded corners of Windows 11 for `window`, which show the desktop behind the
/// bottom corners of the client area. Windows 10 rounds no corner and rejects the attribute.
fn square_corners(window: HWND) {
    let square = DWMWCP_DONOTROUND;
    let size = u32::try_from(size_of_val(&square)).expect("a small attribute");
    // SAFETY: `square` lives through the call, and `size` is its size.
    let set = unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const square).cast(),
            size,
        )
    };
    if let Err(error) = set {
        writeln!(io::stderr(), "the test window keeps its corners: {error}")
            .expect("log the error");
    }
}

/// Grows `window` by the size of its frame, so its client area is `WIDTH` by `HEIGHT` pixels.
fn fit_client_area(window: HWND) {
    let (mut outer, mut inner) = (RECT::default(), RECT::default());
    // SAFETY: `outer` is a writable local, and this thread's window is live.
    unsafe { GetWindowRect(window, &raw mut outer) }.expect("read the window rectangle");
    // SAFETY: `inner` is a writable local, and this thread's window is live.
    unsafe { GetClientRect(window, &raw mut inner) }.expect("read the client area");
    let frame = |outer_size: i32, client_size: i32| outer_size.checked_sub(client_size);
    let outer_width = outer.right.checked_sub(outer.left);
    let outer_height = outer.bottom.checked_sub(outer.top);
    let frame_width = outer_width.and_then(|outer_size| frame(outer_size, inner.right));
    let frame_height = outer_height.and_then(|outer_size| frame(outer_size, inner.bottom));
    let width = frame_width.and_then(|border| WIDTH.checked_add(border)).expect("a small width");
    let height =
        frame_height.and_then(|border| HEIGHT.checked_add(border)).expect("a small height");
    let flags = SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE;
    // SAFETY: SetWindowPos has no pointer arguments, and this thread's window is live.
    unsafe { SetWindowPos(window, None, 0, 0, width, height, flags) }.expect("size the window");
}

/// Whether `window` is topmost.
fn is_topmost(window: HWND) -> bool {
    // SAFETY: GetWindowLongPtrW has no pointer arguments, and this thread's window is live.
    let ex_style = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) };
    let topmost = isize::try_from(WS_EX_TOPMOST.0).expect("a small flag");
    ex_style & topmost != 0
}

impl Drop for PaintedWindow {
    fn drop(&mut self) {
        // Windows destroys a window that this call leaves when its thread exits.
        // SAFETY: this thread created the window, and only this drop destroys it.
        unsafe { DestroyWindow(self.window) }.unwrap_or_default();
    }
}

/// Runs every message waiting for this thread's windows.
fn run_messages() {
    let mut message = MSG::default();
    // SAFETY: `message` is a writable local, and the loop reads every window of this thread.
    while unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        // SAFETY: `message` is the message PeekMessageW filled in.
        unsafe { DispatchMessageW(&raw const message) };
    }
}

/// Whether this session has a desktop that takes input and shows windows. A service session or a
/// locked screen has none, and a screen copy there holds no window.
fn input_desktop_visible() -> bool {
    // SAFETY: OpenInputDesktop has no pointer arguments.
    match unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) } {
        Ok(desktop) => {
            // SAFETY: the open call returned the handle, and only this call closes it.
            unsafe { CloseDesktop(desktop) }.unwrap_or_default();
            true
        }
        Err(_) => false,
    }
}

/// Whether every pixel of `picture` left of its middle is `LEFT_COLOR` and every other pixel is
/// `RIGHT_COLOR`.
fn shows_the_halves(picture: &WindowPicture) -> bool {
    let (pixels, _no_remainder) = picture.rgb.as_chunks::<3>();
    let width = usize::try_from(WIDTH).expect("a small width");
    let half = usize::try_from(HALF_WIDTH).expect("a small width");
    let column = |index: usize| index.checked_rem(width).expect("a nonzero width");
    let expected = |index: usize| if column(index) < half { LEFT_COLOR } else { RIGHT_COLOR };
    let size = (i64::from(picture.width), i64::from(picture.height));
    size == (i64::from(WIDTH), i64::from(HEIGHT))
        && pixels.iter().enumerate().all(|(index, pixel)| *pixel == expected(index))
}

#[test]
fn a_window_of_this_test_is_found_by_title_and_program_and_copied_with_its_picture() {
    if !input_desktop_visible() {
        let reason = "skipped: this session has no visible input desktop to copy a window from";
        writeln!(io::stderr(), "{reason}").expect("log the skip");
        return;
    }
    // Physical pixels, so the window has the size the test names on a scaled display too.
    let _physical = PhysicalPixels::start();
    let window = PaintedWindow::create();
    let exe = env::current_exe().expect("the test binary path");
    let program = exe.file_stem().and_then(|stem| stem.to_str()).expect("a program name");
    let started = Instant::now();
    let picture = loop {
        run_messages();
        let copy = window_copy::copy(program, &window.title).expect("copy the window");
        let picture = copy.expect("the window is found by its title and its program");
        if shows_the_halves(&picture) || started.elapsed() > PICTURE_TIMEOUT {
            break picture;
        }
        thread::park_timeout(RETRY);
    };
    let size = (i64::from(picture.width), i64::from(picture.height));
    assert_eq!(size, (i64::from(WIDTH), i64::from(HEIGHT)));
    let (pixels, _no_remainder) = picture.rgb.as_chunks::<3>();
    let width = usize::try_from(WIDTH).expect("a small width");
    let middle_row = width.checked_mul(usize::try_from(HEIGHT / 2).expect("small")).expect("small");
    let left = pixels.get(middle_row).copied();
    let right =
        pixels.get(middle_row.checked_add(width.saturating_sub(1)).expect("small")).copied();
    assert_eq!((left, right), (Some(LEFT_COLOR), Some(RIGHT_COLOR)), "the middle row's ends");
    assert!(shows_the_halves(&picture), "a pixel differs from the painted picture");
    assert!(is_topmost(window.window), "the copied window is not topmost");
}

#[test]
fn a_title_that_no_window_has_finds_no_window() {
    let title = format!("arcane-eye-launch-test-no-such-window-{}", process::id());
    let copy = window_copy::copy("no-such-program", &title).expect("a search with no window");
    assert_eq!(copy, None);
}
