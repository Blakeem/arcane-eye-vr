//! version.dll stand-in. Forwards the 17 version.dll exports to System32 and starts the core loader thread only when a launch token exists.
//!
//! `build.rs` links the forwarders. This file holds every Windows call: `DllMain`, the launch token
//! view and the loader thread. `attach.rs` holds the decisions as pure functions.

#![expect(
    unsafe_code,
    reason = "DllMain, the launch token view and the core load are Windows calls"
)]

mod attach;
#[cfg(test)]
mod tests;

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt as _;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::{io, mem, panic, ptr};

use ipc_layout::core_init::{INIT_EXPORT, InitFn};
use ipc_layout::launch_token::{self, GAME_PID_OFFSET, LaunchToken, STATUS_OFFSET, Status};
use windows::Win32::Foundation::{CloseHandle, HMODULE, TRUE};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetProcAddress, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LoadLibraryExW,
};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile,
};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::core::{BOOL, HSTRING, PCSTR};

use crate::attach::{Action, CoreLoadError, Exe};

/// The longest path `GetModuleFileNameW` returns, in UTF-16 units with the terminator.
const MAX_PATH_UNITS: usize = 32_768;

const LOADER_THREAD_NAME: &str = "arcane-eye-loader";

/// The DLL entry point. It runs under the loader lock, so it makes kernel32 calls only, never loads a
/// library or waits, and returns TRUE on every path.
#[unsafe(export_name = "DllMain")]
extern "system" fn dll_main(module: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // A panic that unwinds out of DllMain aborts the game. The proxy has no log to report it to.
        drop(panic::catch_unwind(|| process_attach(module)));
    }
    TRUE
}

fn process_attach(module: HMODULE) {
    let (Some(exe_path), Some(proxy_path)) = (module_path(None), module_path(Some(module))) else {
        return;
    };
    let exe = attach::exe(&exe_path);
    // Dropping the handle detaches the loader thread, since DllMain must never wait for it.
    drop(attach(exe, proxy_path, launch_token::MAPPING_NAME, spawn_loader));
}

/// Reads the launch token in the mapping `mapping_name`, acts on it for `exe`, and starts the loader
/// thread through `spawn` when the token asks for the core.
///
/// Returns the loader thread when it started one.
fn attach(
    exe: Exe,
    proxy_path: PathBuf,
    mapping_name: &str,
    spawn: impl FnOnce(LoaderJob) -> io::Result<JoinHandle<()>>,
) -> Option<JoinHandle<()>> {
    let mapping_name = HSTRING::from(mapping_name);
    let view = TokenView::open(&mapping_name)?;
    let token = LaunchToken::decode(&view.read());
    match attach::action(exe, token) {
        Action::StartNothing => None,
        Action::ReportWrongRenderer => {
            view.write_status(Status::WrongRenderer);
            None
        }
        Action::LoadCore { mode } => {
            view.write_game_pid(current_process_id());
            match spawn(LoaderJob { mapping_name, proxy_path, mode }) {
                Ok(loader) => Some(loader),
                // The proxy has no log, so status 11 is its entire report of the spawn failure.
                Err(_spawn_error) => {
                    view.write_status(Status::CoreLoadFailed);
                    None
                }
            }
        }
    }
}

/// What the loader thread needs from process attach.
struct LoaderJob {
    mapping_name: HSTRING,
    proxy_path: PathBuf,
    mode: u32,
}

fn spawn_loader(job: LoaderJob) -> io::Result<JoinHandle<()>> {
    thread::Builder::new().name(LOADER_THREAD_NAME.to_owned()).spawn(move || load(&job, load_core))
}

/// The loader thread's body: reports core loading, then starts the core through `load_core` and
/// reports a failure.
fn load(job: &LoaderJob, load_core: impl FnOnce(&Path, u32) -> Result<(), CoreLoadError>) {
    // A view of its own, so no raw view address crosses threads. The launch app may have exited
    // since process attach, so a missing token only skips the status writes.
    let view = TokenView::open(&job.mapping_name);
    if let Some(view) = &view {
        view.write_status(Status::CoreLoading);
    }
    let loaded = load_core(&attach::core_dll_path(&job.proxy_path), job.mode);
    // The proxy has no log, so status 11 is its entire report of the load or init failure.
    if let (Err(_cause), Some(view)) = (loaded, &view) {
        view.write_status(Status::CoreLoadFailed);
    }
}

/// Loads the core DLL at `core_path` and calls its init export with `mode`.
fn load_core(core_path: &Path, mode: u32) -> Result<(), CoreLoadError> {
    let core_path = HSTRING::from(core_path);
    // The core's imports resolve from its folder, the game folder, System32 and added DLL
    // directories, never the current directory or PATH.
    let search = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
    // SAFETY: `core_path` is a NUL-terminated wide string that outlives the call.
    let core_module = unsafe { LoadLibraryExW(&core_path, None, search) }?;
    let export_name = PCSTR::from_raw(INIT_EXPORT.as_ptr().cast());
    // SAFETY: `core_module` is the module loaded above, and `export_name` is a NUL-terminated constant.
    let init =
        unsafe { GetProcAddress(core_module, export_name) }.ok_or(CoreLoadError::MissingInit)?;
    // SAFETY: arcane-eye-core exports `INIT_EXPORT` with the `InitFn` signature, which its build checks,
    // and the core stays loaded for the life of the process.
    let init = unsafe { mem::transmute::<unsafe extern "system" fn() -> isize, InitFn>(init) };
    let code = init(mode);
    attach::init_result(code)
}

/// The full path of `module`, or of the process exe for `None`.
///
/// Returns `None` when Windows reports no path or truncates it.
fn module_path(module: Option<HMODULE>) -> Option<PathBuf> {
    let mut buffer = vec![0u16; MAX_PATH_UNITS];
    // SAFETY: `module` is `None` or the module the loader passed to DllMain, and the call writes
    // inside `buffer`.
    let length = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    buffer.truncate(length);
    Some(PathBuf::from(OsString::from_wide(&buffer)))
}

fn current_process_id() -> u32 {
    // SAFETY: GetCurrentProcessId takes no arguments and has no preconditions.
    unsafe { GetCurrentProcessId() }
}

/// A read-write view of the launch token's 64 bytes, unmapped on drop.
struct TokenView {
    address: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl TokenView {
    /// Maps the token in the mapping `name`.
    ///
    /// Returns `None` when no such mapping exists, which is off mode, or when it cannot be mapped.
    fn open(name: &HSTRING) -> Option<Self> {
        let access = FILE_MAP_READ | FILE_MAP_WRITE;
        // SAFETY: `name` is a NUL-terminated wide string that outlives the call.
        let Ok(mapping) = (unsafe { OpenFileMappingW(access.0, false, name) }) else {
            return None;
        };
        // SAFETY: `mapping` is the open mapping handle above, and the view spans the token's bytes.
        let address = unsafe { MapViewOfFile(mapping, access, 0, 0, launch_token::SIZE) };
        // The view keeps the mapping alive, so the handle closes at once. A failed close leaks one
        // handle and changes nothing else.
        // SAFETY: `mapping` is a handle this function opened and closes once.
        drop(unsafe { CloseHandle(mapping) });
        (!address.Value.is_null()).then_some(Self { address })
    }

    /// Copies the token's bytes. Each read is volatile because another process shares the memory.
    fn read(&self) -> [u8; launch_token::SIZE] {
        let base = self.address.Value.cast::<u8>().cast_const();
        let mut bytes = [0; launch_token::SIZE];
        for (offset, byte) in bytes.iter_mut().enumerate() {
            // SAFETY: `offset` is below 64, and the view spans the token's 64 bytes.
            *byte = unsafe { ptr::read_volatile(base.wrapping_add(offset)) };
        }
        bytes
    }

    fn write_status(&self, status: Status) {
        self.write_u32(STATUS_OFFSET, status as u32);
    }

    fn write_game_pid(&self, pid: u32) {
        self.write_u32(GAME_PID_OFFSET, pid);
    }

    /// Writes a `u32` field. Callers pass only the 4-aligned field offsets of `ipc-layout`.
    fn write_u32(&self, offset: usize, value: u32) {
        let field = self.address.Value.wrapping_byte_add(offset).cast::<u32>();
        // SAFETY: `offset` is a 4-aligned field offset inside the 64-byte view, and the view starts
        // on a page boundary, so `field` is in bounds and aligned.
        unsafe { ptr::write_volatile(field, value) };
    }
}

impl Drop for TokenView {
    fn drop(&mut self) {
        // A failed unmap leaves this one view mapped. The proxy has no log to report it to.
        // SAFETY: `address` is the base of the view `open` mapped, and only this drop unmaps it.
        drop(unsafe { UnmapViewOfFile(self.address) });
    }
}
