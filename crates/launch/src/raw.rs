//! The launch crate's unsafe edge: registry reads, the launch token mapping, the Steam URL, the
//! window close, the key post and the process wait and end. `raw\desktop.rs` holds the desktop
//! calls, `raw\cursor.rs` the cursor calls, `raw\recycle.rs` the Recycle Bin call and
//! `raw\window_copy.rs` the window search and the screen copy.

#![expect(
    unsafe_code,
    reason = "the registry, file mapping, shell, window and process calls are Windows calls"
)]

use std::ffi::c_void;
use std::marker::PhantomData;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::ptr;
use std::time::Duration;

use ipc_layout::launch_token::{self, LaunchToken};
use tracing::{info, warn};
use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE,
    LPARAM, RPC_E_CHANGED_MODE, TRUE, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::Win32::System::Registry::{
    REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::Win32::System::Threading::{
    INFINITE, OpenProcess, PROCESS_ACCESS_RIGHTS, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_SHOWNORMAL, WM_CLOSE,
    WM_KEYDOWN, WM_KEYUP,
};
use windows::core::{BOOL, HSTRING, PCWSTR, w};

use crate::key::KeyPress;
use crate::registry::{RegistryError, RegistryValue};
use crate::stop::{ProcessState, StopError};

/// The launch token's size as `CreateFileMappingW` takes it.
const TOKEN_MAPPING_SIZE: u32 = 64;
const _: () = assert!(TOKEN_MAPPING_SIZE as usize == launch_token::SIZE);

/// `ShellExecuteW` returns a value above 32 on success and an error code up to 32 otherwise.
const SHELL_EXECUTE_MAX_ERROR: usize = 32;

/// The exit code of a process that `Process::terminate` or a failed desktop start ended.
pub(crate) const ENDED_EXIT_CODE: u32 = 1;

/// The longest wait `WaitForSingleObject` takes that is not `INFINITE`.
const LONGEST_WAIT_MS: u32 = INFINITE.saturating_sub(1);

impl RegistryValue {
    /// The value as a string, or `None` when the key or the value does not exist.
    pub(crate) fn read_string(&self) -> Result<Option<String>, RegistryError> {
        let Some(size) = self.get(RRF_RT_REG_SZ, None)? else {
            return Ok(None);
        };
        let mut bytes = vec![0u8; size];
        let Some(written) = self.get(RRF_RT_REG_SZ, Some(&mut bytes))? else {
            return Ok(None);
        };
        bytes.truncate(written);
        let (units, _odd_byte) = bytes.as_chunks::<2>();
        let text: Vec<u16> = units
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .take_while(|unit| *unit != 0)
            .collect();
        Ok(Some(String::from_utf16_lossy(&text)))
    }

    /// The value as a `REG_DWORD`, or `None` when the key or the value does not exist.
    pub(crate) fn read_dword(&self) -> Result<Option<u32>, RegistryError> {
        let mut bytes = [0u8; 4];
        let found = self.get(RRF_RT_REG_DWORD, Some(&mut bytes))?;
        Ok(found.map(|_size| u32::from_le_bytes(bytes)))
    }

    /// Reads the value into `data`, or only its size for `None`, and returns the size in bytes.
    fn get(
        &self,
        flags: REG_ROUTINE_FLAGS,
        data: Option<&mut [u8]>,
    ) -> Result<Option<usize>, RegistryError> {
        let (key, name) = (HSTRING::from(self.key), HSTRING::from(self.name));
        let (pointer, capacity) = match data {
            Some(buffer) => (Some(buffer.as_mut_ptr().cast::<c_void>()), buffer.len()),
            None => (None, 0),
        };
        // A size below the buffer's length only makes the call write less.
        let mut size = u32::try_from(capacity).unwrap_or(u32::MAX);
        // SAFETY: both names are NUL-terminated wide strings that outlive the call, and `pointer`
        // is `None` or the start of a live buffer of at least `size` writable bytes.
        let status = unsafe {
            RegGetValueW(self.hive.key(), &key, &name, flags, None, pointer, Some(&raw mut size))
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        match status.ok() {
            Ok(()) => Ok(Some(size as usize)),
            Err(source) => Err(RegistryError { value: *self, source }),
        }
    }
}

/// Why the launch token could not be created.
#[derive(Debug, thiserror::Error)]
pub enum TokenCreateError {
    #[error("a launch token already exists")]
    Exists,
    #[error("could not create the launch token's file mapping")]
    Create(#[source] windows::core::Error),
    #[error("could not map a view of the launch token")]
    Map(#[source] windows::core::Error),
}

/// The launch token this process created. Dropping it closes the view and the mapping, so the token
/// is gone once no game process holds a view of it.
pub struct TokenMapping {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
    _mapping: OwnedHandle,
}

impl TokenMapping {
    /// Creates the mapping `name` and writes `token` into it.
    ///
    /// Returns [`TokenCreateError::Exists`] when a mapping `name` already exists, including one
    /// this process holds, and writes nothing then.
    pub(crate) fn create(name: &str, token: &LaunchToken) -> Result<Self, TokenCreateError> {
        let name = HSTRING::from(name);
        // SAFETY: a pagefile-backed mapping with default security and a NUL-terminated name.
        let created = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                TOKEN_MAPPING_SIZE,
                &name,
            )
        };
        let handle = match created {
            Ok(handle) => handle,
            Err(source) => return Err(TokenCreateError::Create(source)),
        };
        // SAFETY: GetLastError takes no arguments and reads the error the create call set.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        // SAFETY: `handle` is the mapping handle the create call opened, and only `mapping` closes it.
        let mapping = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        if existed {
            return Err(TokenCreateError::Exists);
        }
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `handle` stays open while `mapping` lives, and the view spans the token's bytes.
        let address = unsafe { MapViewOfFile(handle, access, 0, 0, launch_token::SIZE) };
        if address.Value.is_null() {
            return Err(TokenCreateError::Map(windows::core::Error::from_thread()));
        }
        let view = Self { address, _mapping: mapping };
        view.write(&token.encode());
        Ok(view)
    }

    /// Copies the token's bytes. Each read is volatile because the game process writes the memory.
    pub(crate) fn read(&self) -> [u8; launch_token::SIZE] {
        let base = self.address.Value.cast::<u8>().cast_const();
        let mut bytes = [0; launch_token::SIZE];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            *byte = unsafe { ptr::read_volatile(base.wrapping_add(offset)) };
        }
        bytes
    }

    fn write(&self, bytes: &[u8; launch_token::SIZE]) {
        let base = self.address.Value.cast::<u8>();
        for (offset, byte) in bytes.iter().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            unsafe { ptr::write_volatile(base.wrapping_add(offset), *byte) };
        }
    }
}

impl Drop for TokenMapping {
    fn drop(&mut self) {
        // SAFETY: `address` is the base of the view `create` mapped, and only this drop unmaps it.
        if let Err(error) = unsafe { UnmapViewOfFile(self.address) } {
            warn!(error = %error, "unmapping the launch token failed, so it lasts until the app exits");
        }
    }
}

/// Windows found no handler for a URL, or the handler failed to start.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("Windows could not open {url} (ShellExecute code {code})")]
pub struct OpenUrlError {
    url: &'static str,
    code: usize,
}

/// One successful `CoInitializeEx` on this thread. Dropping it makes the matching `CoUninitialize`,
/// so a caller's thread leaves `open_url` with the COM state it entered with.
struct ComInit {
    // A raw pointer keeps the value on the thread that initialized COM.
    _thread: PhantomData<*const ()>,
}

impl ComInit {
    /// Starts COM on this thread as `ShellExecuteW` asks, or returns `None` when it did not start.
    fn start() -> Option<Self> {
        // SAFETY: no reserved pointer is passed, and a repeat start on this thread returns a code.
        let started =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if started.is_ok() {
            return Some(Self { _thread: PhantomData });
        }
        if started == RPC_E_CHANGED_MODE {
            info!("COM already runs in another apartment on this thread, so the URL opens in it");
            return None;
        }
        let error = windows::core::Error::from(started);
        warn!(error = %error, "COM failed to start, so the URL opens without it");
        None
    }
}

impl Drop for ComInit {
    fn drop(&mut self) {
        // SAFETY: only a successful CoInitializeEx on this thread creates `self`, and it is not Send.
        unsafe { CoUninitialize() };
    }
}

/// Opens `url` with its registered handler, as a click on a link does.
pub(crate) fn open_url(url: &'static str) -> Result<(), OpenUrlError> {
    // ShellExecuteW may hand the URL to a shell extension through COM, so COM starts first.
    let _com = ComInit::start();
    let file = HSTRING::from(url);
    // SAFETY: the operation and the file are NUL-terminated wide strings that outlive the call, and
    // the parameters and the folder are optional.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    let code = result.0.addr();
    if code > SHELL_EXECUTE_MAX_ERROR { Ok(()) } else { Err(OpenUrlError { url, code }) }
}

/// A process opened by id. While it is open, Windows gives the id to no other process.
pub(crate) struct Process {
    pid: u32,
    handle: OwnedHandle,
}

impl Process {
    /// Opens process `pid` with `access`. A wait needs `PROCESS_SYNCHRONIZE` and an end needs
    /// `PROCESS_TERMINATE`.
    pub(crate) fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<Self, StopError> {
        // SAFETY: OpenProcess has no pointer arguments.
        let handle = match unsafe { OpenProcess(access, false, pid) } {
            Ok(handle) => handle,
            Err(source) => return Err(StopError::Open { pid, source }),
        };
        // SAFETY: `handle` is the process handle the open call returned, and only `Self` closes it.
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        Ok(Self { pid, handle })
    }

    /// Waits up to `timeout` for the process to exit.
    pub(crate) fn wait(&self, timeout: Duration) -> Result<ProcessState, StopError> {
        let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX).min(LONGEST_WAIT_MS);
        // SAFETY: the handle is open while `self` lives. Missing synchronize access fails the call.
        let waited = unsafe { WaitForSingleObject(self.raw(), ms) };
        if waited == WAIT_OBJECT_0 {
            return Ok(ProcessState::Exited);
        }
        if waited == WAIT_TIMEOUT {
            return Ok(ProcessState::Running);
        }
        Err(StopError::Wait { pid: self.pid, source: windows::core::Error::from_thread() })
    }

    /// Starts ending the process. It is gone once a wait sees it exit.
    pub(crate) fn terminate(&self) -> Result<(), StopError> {
        // SAFETY: the handle is open while `self` lives. Missing terminate access fails the call.
        match unsafe { TerminateProcess(self.raw(), ENDED_EXIT_CODE) } {
            Ok(()) => Ok(()),
            Err(source) => Err(StopError::End { pid: self.pid, source }),
        }
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }
}

/// The top-level windows of one process, or of every process, which `collect_window` gathers.
struct WindowSearch {
    /// The process whose windows the search keeps, or `None` to keep every window.
    owner: Option<u32>,
    windows: Vec<HWND>,
}

/// The top-level windows of process `owner`, or of every process for `None`, hidden windows
/// included.
fn top_level_windows(owner: Option<u32>) -> Vec<HWND> {
    let mut search = WindowSearch { owner, windows: Vec::new() };
    let search_address = LPARAM((&raw mut search).expose_provenance().cast_signed());
    // SAFETY: `collect_window` reads `search_address` as `search`, which outlives the call.
    if let Err(error) = unsafe { EnumWindows(Some(collect_window), search_address) } {
        warn!(?owner, error = %error, "the window list stopped early, so it misses some windows");
    }
    search.windows
}

/// Posts `WM_CLOSE` to every top-level window of process `pid`, hidden windows included. Returns
/// the number of windows that got the message.
pub(crate) fn ask_windows_to_close(pid: u32) -> usize {
    let mut asked = 0usize;
    for window in top_level_windows(Some(pid)) {
        // SAFETY: a posted message carries no pointer, and a window gone since the search fails it.
        match unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) } {
            Ok(()) => asked = asked.saturating_add(1),
            Err(error) => warn!(pid, error = %error, "a window did not take the close message"),
        }
    }
    asked
}

/// Posts `key` as `WM_KEYDOWN`, then `WM_KEYUP`, to each visible top-level window of process `pid`.
/// Returns the number of windows that got both messages.
pub(crate) fn post_key(pid: u32, key: &KeyPress) -> usize {
    let mut posted = 0usize;
    for window in top_level_windows(Some(pid)) {
        // SAFETY: IsWindowVisible has no pointer arguments, and a window gone since the search is
        // not visible.
        if !unsafe { IsWindowVisible(window) }.as_bool() {
            continue;
        }
        let pressed = post_key_message(pid, window, WM_KEYDOWN, key.virtual_key, key.down)
            && post_key_message(pid, window, WM_KEYUP, key.virtual_key, key.up);
        if pressed {
            posted = posted.saturating_add(1);
        }
    }
    posted
}

/// Posts one key message to `window` of process `pid` and returns whether the window took it.
fn post_key_message(pid: u32, window: HWND, message: u32, w: usize, l: isize) -> bool {
    // SAFETY: a posted message carries no pointer, and a window gone since the search fails it.
    match unsafe { PostMessageW(Some(window), message, WPARAM(w), LPARAM(l)) } {
        Ok(()) => true,
        Err(error) => {
            warn!(pid, message, error = %error, "a window did not take the key message");
            false
        }
    }
}

/// `EnumWindows` calls this once per top-level window. `search` is the address of a `WindowSearch`.
extern "system" fn collect_window(window: HWND, search: LPARAM) -> BOOL {
    let search = ptr::with_exposed_provenance_mut::<WindowSearch>(search.0.cast_unsigned());
    let mut owner = 0u32;
    // SAFETY: `window` is a handle EnumWindows passes, and `owner` is a writable local.
    unsafe { GetWindowThreadProcessId(window, Some(&raw mut owner)) };
    // SAFETY: `search` is the `WindowSearch` that `top_level_windows` owns for the EnumWindows
    // call, and nothing else touches it during the call.
    let search = unsafe { &mut *search };
    if search.owner.is_none_or(|pid| pid == owner) {
        search.windows.push(window);
    }
    TRUE
}

pub(crate) mod cursor;
pub(crate) mod desktop;
pub(crate) mod recycle;
pub(crate) mod window_copy;

#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod desktop_tests;
#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_copy_tests;
