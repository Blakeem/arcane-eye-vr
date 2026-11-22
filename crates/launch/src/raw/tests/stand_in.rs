//! The stand-in child process of the launch crate's tests: a copy of the test binary that runs one
//! ignored test, which creates a window, or a window and a popup it owns, and runs its messages
//! until the window closes. `tests.rs`, `key_tests.rs` and `mouse/launcher_tests.rs` start it.

use std::ffi::OsStr;
use std::io::{self, BufRead as _, BufReader, Write as _};
use std::os::windows::process::CommandExt as _;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;
use std::{env, fs};

use test_support::TestDir;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostQuitMessage,
    RegisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WNDCLASSW, WNDPROC,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, PCWSTR};

use crate::test_dir;

/// The line a stand-in writes to standard output once its window exists.
const READY_LINE: &str = "stand-in window ready";

/// The start of the name of the test binary copy that a stand-in child runs as, which the calling
/// test's name ends. A stand-in test that runs in any other exe returns at once, since nothing would
/// close its window.
pub(in crate::raw) const STAND_IN_EXE: &str = "launch-stand-in-";

/// The program file name of the stand-in that the calling test `test_name` starts.
pub(in crate::raw) fn exe_name(test_name: &str) -> String {
    format!("{STAND_IN_EXE}{test_name}.exe")
}

/// Long enough for a stand-in to process one message on a loaded test machine.
pub(in crate::raw) const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

/// The end of the title of the window a stand-in's window owns, after the class name.
pub(in crate::raw) const OWNED_TITLE_END: &str = " owned";

pub(in crate::raw) extern "system" fn close_on_request(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        // SAFETY: PostQuitMessage has no pointer arguments.
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: the arguments are the ones Windows passed to this window procedure.
    unsafe { DefWindowProcW(window, message, w, l) }
}

/// The styles of a stand-in's window.
pub(in crate::raw) type WindowStyles = (WINDOW_STYLE, WINDOW_EX_STYLE);

/// The styles of a hidden stand-in window.
pub(in crate::raw) const HIDDEN: WindowStyles = (WS_OVERLAPPEDWINDOW, WINDOW_EX_STYLE(0));

/// Creates a window with `procedure` and `styles`, reports it ready and runs its messages until it
/// quits.
pub(in crate::raw) fn run_stand_in(class: &str, procedure: WNDPROC, styles: WindowStyles) {
    run_stand_in_owning(class, procedure, styles, None);
}

/// Creates a window with `procedure` and `styles` and, for `Some(owned)`, a window of the styles
/// `owned` that it owns, titled with [`OWNED_TITLE_END`]. Then reports them ready and runs their
/// messages until they quit.
pub(in crate::raw) fn run_stand_in_owning(
    class: &str,
    procedure: WNDPROC,
    styles: WindowStyles,
    owned: Option<WindowStyles>,
) {
    let exe = env::current_exe().expect("the test binary path");
    let running_name = exe.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if !running_name.starts_with(STAND_IN_EXE) {
        return;
    }
    // SAFETY: no module name means this exe's own module.
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.expect("this exe's module");
    let instance = HINSTANCE::from(module);
    let class = HSTRING::from(class);
    let class_info = WNDCLASSW {
        lpfnWndProc: procedure,
        hInstance: instance,
        lpszClassName: PCWSTR(class.as_ptr()),
        ..WNDCLASSW::default()
    };
    // SAFETY: `class_info` and the class name it points to outlive the call.
    assert_ne!(unsafe { RegisterClassW(&raw const class_info) }, 0, "register the class");
    let window = create_window(&class, &class, styles, None, instance);
    if let Some(owned) = owned {
        let title = HSTRING::from(format!("{class}{OWNED_TITLE_END}"));
        create_window(&class, &title, owned, Some(window), instance);
    }
    let mut stdout = io::stdout();
    writeln!(stdout, "{READY_LINE}").and_then(|()| stdout.flush()).expect("report ready");
    let mut message = MSG::default();
    // SAFETY: `message` is a writable local, and the loop reads every window of this thread.
    while unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0 > 0 {
        // SAFETY: `message` is the message GetMessageW filled in.
        unsafe { DispatchMessageW(&raw const message) };
    }
}

/// Creates a window of the registered class `class`, titled `title`, with `styles`. A popup's
/// `owner` owns it.
fn create_window(
    class: &HSTRING,
    title: &HSTRING,
    styles: WindowStyles,
    owner: Option<HWND>,
    instance: HINSTANCE,
) -> HWND {
    let (style, ex_style) = styles;
    // The window has no size, so a visible stand-in covers nothing on the desktop.
    // SAFETY: the class is registered, and every pointer argument is a live string or null.
    unsafe {
        CreateWindowExW(
            ex_style,
            class,
            title,
            style,
            0,
            0,
            0,
            0,
            owner,
            None,
            Some(instance),
            None,
        )
    }
    .expect("create the stand-in window")
}

/// A stand-in child process. Dropping it ends a child that a failed assertion left running and
/// removes its folder.
pub(in crate::raw) struct StandIn {
    pub(in crate::raw) child: Child,
    // Held open, since libtest exits with a failure when its last lines meet a closed pipe.
    pub(in crate::raw) stdout: BufReader<ChildStdout>,
    // Dropped after `drop` waits for the child, since the stand-in's exe stays locked until then.
    _dir: TestDir,
}

impl StandIn {
    /// Starts the child test `child_test` in a copy of this test binary, named after the calling
    /// test `test_name` in its folder, and returns once its window exists.
    pub(in crate::raw) fn start(child_test: &str, test_name: &str) -> Self {
        let dir = test_dir(test_name);
        let exe = dir.path().join(exe_name(test_name));
        let test_binary = env::current_exe().expect("the test binary path");
        fs::copy(test_binary, &exe).expect("copy the test binary as the stand-in");
        // CREATE_NO_WINDOW gives the child no console window, so the close cannot reach a console.
        let mut child = Command::new(exe)
            .args(["--exact", child_test, "--ignored"])
            .creation_flags(CREATE_NO_WINDOW.0)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the stand-in");
        let mut stdout = BufReader::new(child.stdout.take().expect("the stand-in's output"));
        // A serial libtest run prints the test's name with no line end before the test's own output.
        let stand_in_ready =
            (&mut stdout).lines().map_while(Result::ok).any(|line| line.ends_with(READY_LINE));
        let stand_in = Self { child, stdout, _dir: dir };
        assert!(stand_in_ready, "the stand-in ended before its window existed");
        stand_in
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        // A kill of a child that already exited also returns Ok.
        self.child.kill().unwrap_or_default();
        self.child.wait().unwrap_or_default();
    }
}
