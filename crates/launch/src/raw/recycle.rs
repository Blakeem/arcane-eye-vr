//! The shell call that moves a folder to the Recycle Bin.

use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;
use std::ptr;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{FO_DELETE, SHFILEOPSTRUCTW, SHFileOperationW};
use windows::core::PCWSTR;

/// The file operation flags of `shellapi.h` for a silent move to the Recycle Bin. 0x40 allows the
/// undo, which sends the folder to the Recycle Bin. 0x614 shows no dialog of any kind.
const RECYCLE_FLAGS: u16 = 0x40 | 0x614;

/// How the shell's move of a folder to the Recycle Bin ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Moved {
    Done,
    /// The shell stopped the move without an error code.
    Aborted,
    /// The shell's nonzero result.
    Failed(i32),
}

/// Asks the shell to move `folder`, an absolute path, to the Recycle Bin, with no dialog.
pub(crate) fn to_recycle_bin(folder: &Path) -> Moved {
    // The shell reads a list of paths, so the one path ends with two nulls.
    let from: Vec<u16> = folder.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: HWND::default(),
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        pTo: PCWSTR::null(),
        fFlags: RECYCLE_FLAGS,
        fAnyOperationsAborted: false.into(),
        hNameMappings: ptr::null_mut(),
        lpszProgressTitle: PCWSTR::null(),
    };
    // SAFETY: `operation` is live for the call, and its source list is one path with two nulls at
    // its end, which outlives the call. The delete takes no destination, and the flags ask for no
    // name mappings and no progress title.
    let code = unsafe { SHFileOperationW(&raw mut operation) };
    match (code, operation.fAnyOperationsAborted.as_bool()) {
        (0, false) => Moved::Done,
        (0, true) => Moved::Aborted,
        (code, _) => Moved::Failed(code),
    }
}
